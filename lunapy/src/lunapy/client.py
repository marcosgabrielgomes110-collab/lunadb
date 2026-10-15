from __future__ import annotations

import json
import os
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Iterable, Iterator, Mapping, Optional, Sequence, Union

from .errors import LunaConnectionError, LunaError, error_from_status
from .models import (
    ApiKey,
    DbMeta,
    Field,
    Grant,
    Level,
    QueryResult,
    RecordPage,
    Role,
    Sort,
    TableSchema,
    User,
)

DEFAULT_URL = "http://127.0.0.1:7700"
DEFAULT_CONFIG_PATH = "~/.lunadb/config.toml"

GrantLike = Union[Grant, Mapping[str, str], tuple[str, str], list[str]]


def _normalize_grants(grants: Iterable[GrantLike]) -> list[dict[str, str]]:
    normalized: list[dict[str, str]] = []
    for grant in grants:
        if isinstance(grant, Grant):
            normalized.append(grant.to_dict())
        elif isinstance(grant, Mapping):
            normalized.append(Grant.from_dict(dict(grant)).to_dict())
        else:
            db, level = grant
            normalized.append(Grant(db=db, level=Level(level)).to_dict())
    return normalized


def _unique_key(key: Union[str, Sequence[str]]) -> Union[str, list[str]]:
    if isinstance(key, str):
        return key
    return list(key)


def _normalize_schema(schema: Union[TableSchema, Mapping[str, Any]]) -> dict[str, Any]:
    if isinstance(schema, TableSchema):
        return schema.to_dict()
    if isinstance(schema, Mapping):
        return dict(schema)
    raise LunaError("schema must be a TableSchema or dict")


def _normalize_aggregate(
    spec: Union[str, Mapping[str, Any]],
) -> dict[str, Any]:
    """Accepts {"op": "...", "field": "..."}, "count", or "sum:field"."""
    if isinstance(spec, Mapping):
        return dict(spec)
    if not isinstance(spec, str):
        raise LunaError("aggregate must be a dict or string")
    op, _, field = spec.partition(":")
    spec_dict: dict[str, Any] = {"op": op.strip()}
    if field.strip():
        spec_dict["field"] = field.strip()
    return spec_dict


def _normalize_sort(item: Union[Sort, Mapping[str, str], tuple[str, str], str]) -> dict[str, str]:
    if isinstance(item, Sort):
        return item.to_dict()
    if isinstance(item, Mapping):
        return Sort.from_dict(dict(item)).to_dict()
    if isinstance(item, str):
        if item.startswith("-"):
            return Sort(field=item[1:], dir="desc").to_dict()
        return Sort(field=item).to_dict()
    field, direction = item
    return Sort(field=field, dir=direction).to_dict()


def _extract_payload(err: urllib.error.HTTPError) -> dict[str, Any]:
    try:
        payload = json.loads(err.read())
        if isinstance(payload, dict):
            return payload
    except Exception:
        pass
    return {}


class LunaClient:
    def __init__(
        self,
        base_url: Optional[str] = None,
        api_key: Optional[str] = None,
        timeout: float = 30.0,
    ) -> None:
        self.base_url = (
            base_url or os.environ.get("LUNADB_URL") or DEFAULT_URL
        ).rstrip("/")
        self.api_key = api_key or os.environ.get("LUNADB_API_KEY")
        self.timeout = timeout

    @classmethod
    def from_config(
        cls,
        path: Optional[Union[str, Path]] = None,
        api_key: Optional[str] = None,
    ) -> LunaClient:
        config_path = Path(
            path or os.environ.get("LUNADB_CONFIG") or DEFAULT_CONFIG_PATH
        ).expanduser()
        if not config_path.exists():
            raise LunaError(f"config not found: {config_path}")
        with config_path.open("rb") as handle:
            data = tomllib.load(handle)
        server = data.get("server", {})
        host = server.get("host", "127.0.0.1")
        port = server.get("port", 7700)
        return cls(base_url=f"http://{host}:{port}", api_key=api_key)

    def health(self) -> dict[str, Any]:
        return self._request("GET", "/health", auth=False)

    def status(self) -> dict[str, Any]:
        return self._request("GET", "/status")

    def login(self, username: str, password: str) -> User:
        data = self._request(
            "POST",
            "/auth/login",
            body={"username": username, "password": password},
            auth=False,
        )
        self.api_key = data["api_key"]
        return User.from_dict(data["user"])

    def rotate_key(self) -> str:
        data = self._request("POST", "/auth/rotate-key")
        self.api_key = data["api_key"]
        return self.api_key

    def list_api_keys(self, user_id: Optional[str] = None) -> list[ApiKey]:
        path = (
            f"/users/{urllib.parse.quote(user_id)}/api-keys"
            if user_id
            else "/auth/api-keys"
        )
        return [ApiKey.from_dict(item) for item in self._request("GET", path)]

    def create_api_key(
        self, user_id: Optional[str] = None, name: Optional[str] = None
    ) -> tuple[ApiKey, str]:
        path = (
            f"/users/{urllib.parse.quote(user_id)}/api-keys"
            if user_id
            else "/auth/api-keys"
        )
        data = self._request("POST", path, body={"name": name})
        return ApiKey.from_dict(data["api_key"]), data["key"]

    def revoke_api_key(
        self, key_id: str, user_id: Optional[str] = None
    ) -> ApiKey:
        path = (
            f"/users/{urllib.parse.quote(user_id)}/api-keys/{urllib.parse.quote(key_id)}"
            if user_id
            else f"/auth/api-keys/{urllib.parse.quote(key_id)}"
        )
        return ApiKey.from_dict(self._request("DELETE", path))

    def me(self) -> User:
        return User.from_dict(self._request("GET", "/me"))

    def list_users(self) -> list[User]:
        return [User.from_dict(item) for item in self._request("GET", "/users")]

    def create_user(
        self,
        username: str,
        password: str,
        role: str = "user",
        grants: Iterable[GrantLike] = (),
    ) -> User:
        data = self._request(
            "POST",
            "/users",
            body={
                "username": username,
                "password": password,
                "role": role,
                "grants": _normalize_grants(grants),
            },
        )
        return User.from_dict(data)

    def delete_user(self, user_id: str) -> None:
        self._request("DELETE", f"/users/{urllib.parse.quote(user_id)}")

    def set_grants(self, user_id: str, grants: Iterable[GrantLike]) -> User:
        data = self._request(
            "PUT",
            f"/users/{urllib.parse.quote(user_id)}/grants",
            body={"grants": _normalize_grants(grants)},
        )
        return User.from_dict(data)

    def list_roles(self) -> list[Role]:
        return [Role.from_dict(item) for item in self._request("GET", "/roles")]

    def list_dbs(self) -> list[DbMeta]:
        return [DbMeta.from_dict(item) for item in self._request("GET", "/dbs")]

    def create_db(self, name: str) -> DbMeta:
        return DbMeta.from_dict(self._request("POST", "/dbs", body={"name": name}))

    def get_db(self, name: str) -> DbMeta:
        return DbMeta.from_dict(
            self._request("GET", f"/dbs/{urllib.parse.quote(name)}")
        )

    def delete_db(self, name: str) -> None:
        self._request("DELETE", f"/dbs/{urllib.parse.quote(name)}")

    def list_tables(self, db: str) -> list[str]:
        data = self._request("GET", f"/dbs/{urllib.parse.quote(db)}/tables")
        return list(data["tables"])

    def create_table(
        self,
        db: str,
        table: str,
        schema: Union[TableSchema, Mapping[str, Any], None] = None,
    ) -> None:
        body: dict[str, Any] = {"name": table}
        if schema is not None:
            body["schema"] = _normalize_schema(schema)
        self._request(
            "POST",
            f"/dbs/{urllib.parse.quote(db)}/tables",
            body=body,
        )

    def delete_table(self, db: str, table: str) -> None:
        self._request(
            "DELETE",
            f"/dbs/{urllib.parse.quote(db)}/tables/{urllib.parse.quote(table)}",
        )

    def get_schema(self, db: str, table: str) -> TableSchema:
        data = self._request(
            "GET",
            self._schema_path(db, table),
        )
        return TableSchema.from_dict(data)

    def set_schema(
        self,
        db: str,
        table: str,
        schema: Union[TableSchema, Mapping[str, Any]],
        force: bool = False,
    ) -> TableSchema:
        data = self._request(
            "PUT",
            self._schema_path(db, table),
            body=_normalize_schema(schema),
            params={"force": "true" if force else None},
        )
        return TableSchema.from_dict(data)

    def delete_schema(self, db: str, table: str) -> None:
        self._request("DELETE", self._schema_path(db, table))

    def patch_schema(
        self,
        db: str,
        table: str,
        patch: Mapping[str, Any],
        force: bool = False,
    ) -> TableSchema:
        data = self._request(
            "PATCH",
            self._schema_path(db, table),
            body=dict(patch),
            params={"force": "true" if force else None},
        )
        return TableSchema.from_dict(data)

    def add_schema_fields(
        self,
        db: str,
        table: str,
        fields: Mapping[str, Union[Field, Mapping[str, Any]]],
        force: bool = False,
    ) -> TableSchema:
        normalized = {
            name: field.to_dict() if isinstance(field, Field) else dict(field)
            for name, field in fields.items()
        }
        return self.patch_schema(db, table, {"add": normalized}, force=force)

    def update_schema_field(
        self,
        db: str,
        table: str,
        field: str,
        changes: Mapping[str, Any],
        force: bool = False,
    ) -> TableSchema:
        return self.patch_schema(
            db, table, {"update": {field: dict(changes)}}, force=force
        )

    def remove_schema_fields(
        self, db: str, table: str, *fields: str, force: bool = False
    ) -> TableSchema:
        return self.patch_schema(db, table, {"remove": list(fields)}, force=force)

    def add_unique_key(
        self, db: str, table: str, key: Union[str, Sequence[str]], force: bool = False
    ) -> TableSchema:
        return self.patch_schema(
            db, table, {"unique_add": [_unique_key(key)]}, force=force
        )

    def remove_unique_key(
        self, db: str, table: str, key: Union[str, Sequence[str]], force: bool = False
    ) -> TableSchema:
        return self.patch_schema(
            db, table, {"unique_remove": [_unique_key(key)]}, force=force
        )

    def validate_record(
        self, db: str, table: str, record: Mapping[str, Any]
    ) -> dict[str, Any]:
        return self._request(
            "POST",
            f"/dbs/{urllib.parse.quote(db)}/tables/{urllib.parse.quote(table)}/records/validate",
            body=dict(record),
        )

    def _schema_path(self, db: str, table: str) -> str:
        return (
            f"/dbs/{urllib.parse.quote(db)}"
            f"/tables/{urllib.parse.quote(table)}/schema"
        )

    def list_records(
        self,
        db: str,
        table: str,
        limit: Optional[int] = None,
        offset: Optional[int] = None,
        cursor: Optional[str] = None,
    ) -> list[dict[str, Any]]:
        return list(self.list_records_page(db, table, limit, offset, cursor).records)

    def list_records_page(
        self,
        db: str,
        table: str,
        limit: Optional[int] = None,
        offset: Optional[int] = None,
        cursor: Optional[str] = None,
    ) -> RecordPage:
        data = self._request(
            "GET",
            f"/dbs/{urllib.parse.quote(db)}/tables/{urllib.parse.quote(table)}/records",
            params={"limit": limit, "offset": offset, "cursor": cursor},
        )
        return RecordPage.from_dict(data)

    def create_record(
        self, db: str, table: str, record: Mapping[str, Any]
    ) -> dict[str, Any]:
        return self._request(
            "POST",
            f"/dbs/{urllib.parse.quote(db)}/tables/{urllib.parse.quote(table)}/records",
            body=dict(record),
        )

    def get_record(
        self, db: str, table: str, record_id: str
    ) -> dict[str, Any]:
        return self._request(
            "GET",
            self._record_path(db, table, record_id),
        )

    def replace_record(
        self,
        db: str,
        table: str,
        record_id: str,
        record: Mapping[str, Any],
    ) -> dict[str, Any]:
        return self._request(
            "PUT",
            self._record_path(db, table, record_id),
            body=dict(record),
        )

    def patch_record(
        self,
        db: str,
        table: str,
        record_id: str,
        patch: Mapping[str, Any],
    ) -> dict[str, Any]:
        return self._request(
            "PATCH",
            self._record_path(db, table, record_id),
            body=dict(patch),
        )

    def aggregate(
        self,
        db: str,
        table: str,
        *,
        aggregates: Mapping[str, Union[str, Mapping[str, Any]]],
        group_by: Optional[Iterable[str]] = None,
        filter: Optional[Mapping[str, Any]] = None,
        limit: Optional[int] = None,
    ) -> list[dict[str, Any]]:
        body: dict[str, Any] = {
            "aggregates": {
                name: _normalize_aggregate(spec)
                for name, spec in aggregates.items()
            }
        }
        if group_by is not None:
            body["group_by"] = list(group_by)
        if filter is not None:
            body["filter"] = dict(filter)
        if limit is not None:
            body["limit"] = limit
        data = self._request(
            "POST",
            f"/dbs/{urllib.parse.quote(db)}/tables/{urllib.parse.quote(table)}/records/aggregate",
            body=body,
        )
        return list(data["groups"])

    def set_password(
        self,
        user_id: str,
        password: str,
        current_password: Optional[str] = None,
    ) -> User:
        body: dict[str, Any] = {"password": password}
        if current_password is not None:
            body["current_password"] = current_password
        data = self._request(
            "PUT",
            f"/users/{urllib.parse.quote(user_id)}/password",
            body=body,
        )
        return User.from_dict(data)

    def query(
        self,
        db: str,
        table: str,
        *,
        filter: Optional[Mapping[str, Any]] = None,
        sort: Optional[Iterable[Union[Sort, Mapping[str, str], tuple[str, str], str]]] = None,
        fields: Optional[Iterable[str]] = None,
        limit: Optional[int] = None,
        offset: Optional[int] = None,
        cursor: Optional[str] = None,
    ) -> QueryResult:
        body: dict[str, Any] = {}
        if filter is not None:
            body["filter"] = dict(filter)
        if sort is not None:
            body["sort"] = [_normalize_sort(item) for item in sort]
        if fields is not None:
            body["fields"] = list(fields)
        if limit is not None:
            body["limit"] = limit
        if offset is not None:
            body["offset"] = offset
        if cursor is not None:
            body["cursor"] = cursor
        data = self._request(
            "POST",
            f"/dbs/{urllib.parse.quote(db)}/tables/{urllib.parse.quote(table)}/records/query",
            body=body,
        )
        return QueryResult.from_dict(data)

    def iter_records(
        self,
        db: str,
        table: str,
        page_size: int = 100,
        limit: Optional[int] = None,
        offset: Optional[int] = None,
    ) -> Iterator[dict[str, Any]]:
        """Walks every record of `table` with keyset pagination.

        Stable under concurrent inserts and deletes, unlike offset paging.
        Pass `limit` to stop after that many records, or `offset` to skip a
        prefix (the first page is still requested with `offset`).
        """
        cursor: Optional[str] = None
        yielded = 0
        while True:
            page_offset = offset if cursor is None else None
            page = self.list_records_page(
                db, table, limit=page_size, offset=page_offset, cursor=cursor
            )
            if not page.records:
                return
            for record in page.records:
                if limit is not None and yielded >= limit:
                    return
                yielded += 1
                yield record
            if not page.next_cursor:
                return
            # after the first page the cursor already encodes the position, so
            # the offset must not be sent again
            offset = None
            cursor = page.next_cursor

    def iter_query(
        self,
        db: str,
        table: str,
        *,
        page_size: int = 100,
        limit: Optional[int] = None,
        filter: Optional[Mapping[str, Any]] = None,
        sort: Optional[Iterable[Union[Sort, Mapping[str, str], tuple[str, str], str]]] = None,
        fields: Optional[Iterable[str]] = None,
    ) -> Iterator[dict[str, Any]]:
        """Walks every record matching a query, one keyset page at a time."""
        cursor: Optional[str] = None
        yielded = 0
        while True:
            result = self.query(
                db,
                table,
                filter=filter,
                sort=sort,
                fields=fields,
                limit=page_size,
                cursor=cursor,
            )
            if not result.records:
                return
            for record in result.records:
                if limit is not None and yielded >= limit:
                    return
                yielded += 1
                yield record
            if not result.next_cursor:
                return
            cursor = result.next_cursor

    def delete_record(self, db: str, table: str, record_id: str) -> None:
        self._request("DELETE", self._record_path(db, table, record_id))

    def _record_path(self, db: str, table: str, record_id: str) -> str:
        return (
            f"/dbs/{urllib.parse.quote(db)}"
            f"/tables/{urllib.parse.quote(table)}"
            f"/records/{urllib.parse.quote(record_id)}"
        )

    def _request(
        self,
        method: str,
        path: str,
        body: Optional[Mapping[str, Any]] = None,
        params: Optional[Mapping[str, Any]] = None,
        auth: bool = True,
    ) -> Any:
        url = self.base_url + path
        if params:
            query = {key: value for key, value in params.items() if value is not None}
            if query:
                url += "?" + urllib.parse.urlencode(query)
        data = None
        headers = {"Accept": "application/json"}
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        if auth:
            if not self.api_key:
                raise LunaError(
                    "api_key not set; use login() or provide api_key"
                )
            headers["Authorization"] = f"Bearer {self.api_key}"
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                raw = response.read()
        except urllib.error.HTTPError as err:
            payload = _extract_payload(err)
            message = str(payload.get("error", err.reason))
            raise error_from_status(err.code, message, payload) from None
        except urllib.error.URLError as err:
            raise LunaConnectionError(
                f"failed to connect to {self.base_url}: {err.reason}"
            ) from err
        if not raw:
            return None
        return json.loads(raw)
