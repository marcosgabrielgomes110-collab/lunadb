from __future__ import annotations

import json
import os
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Iterable, Mapping, Optional, Union

from .errors import LunaConnectionError, LunaError, error_from_status
from .models import DbMeta, Grant, Level, QueryResult, Role, Sort, TableSchema, User

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


def _normalize_schema(schema: Union[TableSchema, Mapping[str, Any]]) -> dict[str, Any]:
    if isinstance(schema, TableSchema):
        return schema.to_dict()
    if isinstance(schema, Mapping):
        return dict(schema)
    raise LunaError("schema deve ser TableSchema ou dict")


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
            raise LunaError(f"config nao encontrado: {config_path}")
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
    ) -> list[dict[str, Any]]:
        data = self._request(
            "GET",
            f"/dbs/{urllib.parse.quote(db)}/tables/{urllib.parse.quote(table)}/records",
            params={"limit": limit, "offset": offset},
        )
        return list(data["records"])

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
        data = self._request(
            "POST",
            f"/dbs/{urllib.parse.quote(db)}/tables/{urllib.parse.quote(table)}/records/query",
            body=body,
        )
        return QueryResult.from_dict(data)

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
                    "api_key nao definida; use login() ou informe api_key"
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
                f"falha ao conectar em {self.base_url}: {err.reason}"
            ) from err
        if not raw:
            return None
        return json.loads(raw)
