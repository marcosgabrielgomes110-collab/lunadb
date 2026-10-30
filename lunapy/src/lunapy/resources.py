"""High-level handles: ``client.db("app")`` and ``client.table("app", "users")``.

These wrappers only add ergonomics; every call goes through ``LunaClient``, so
the full low-level API remains available.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Any, Iterable, Iterator, Mapping, Sequence

from .errors import LunaNotFoundError
from .models import (
    DbMeta,
    Field,
    QueryResult,
    Record,
    RecordPage,
    TableSchema,
)

if TYPE_CHECKING:  # pragma: no cover - import cycle only for type checkers
    from .client import LunaClient, SortLike

__all__ = ["Database", "QueryBuilder", "Table"]

_CONDITION_OPERATORS = {
    "eq": "$eq",
    "ne": "$ne",
    "gt": "$gt",
    "gte": "$gte",
    "lt": "$lt",
    "lte": "$lte",
    "in": "$in",
    "nin": "$nin",
    "contains": "$contains",
    "startswith": "$starts_with",
    "starts_with": "$starts_with",
    "endswith": "$ends_with",
    "ends_with": "$ends_with",
    "regex": "$regex",
    "exists": "$exists",
}


def _flatten(items: Iterable[Any]) -> list[Any]:
    """Flattens one level, but keeps a single ("field", "dir") tuple intact."""
    flat: list[Any] = []
    for item in items:
        if isinstance(item, (list, tuple)) and not (
            isinstance(item, tuple)
            and len(item) == 2
            and all(isinstance(part, str) for part in item)
        ):
            flat.extend(item)
        else:
            flat.append(item)
    return flat


def _conditions_to_filter(conditions: Mapping[str, Any]) -> dict[str, Any]:
    """Turns Python keyword arguments into an API filter.

    ``name="Ana"`` becomes ``{"name": "Ana"}``; ``age__gte=18`` becomes
    ``{"age": {"$gte": 18}}``. Use the explicit ``filter=`` argument for
    ``$and``/``$or``/``$not`` or for field names that cannot be keywords.
    """
    result: dict[str, Any] = {}
    for raw_name, value in conditions.items():
        name, separator, operator = raw_name.rpartition("__")
        if not separator or not name or operator not in _CONDITION_OPERATORS:
            result[raw_name] = value
            continue
        normalized = _CONDITION_OPERATORS[operator]
        current = result.get(name)
        if current is None:
            result[name] = {normalized: value}
        elif isinstance(current, Mapping):
            result[name] = {**current, normalized: value}
        else:
            result[name] = {"$eq": current, normalized: value}
    return result


def _merge_filter(
    base: Mapping[str, Any] | None,
    extra: Mapping[str, Any],
) -> dict[str, Any] | None:
    if base is None:
        return dict(extra) if extra else None
    if not extra:
        return dict(base)
    return {**base, **extra}


class Database:
    """A database handle obtained with ``client.db("app")``."""

    def __init__(self, client: LunaClient, name: str) -> None:
        self.client = client
        self.name = name

    def __repr__(self) -> str:
        return f"Database({self.name!r})"

    def __enter__(self) -> Database:
        return self

    def __exit__(self, *exc_info: object) -> None:
        return None

    def __getitem__(self, table: str) -> Table:
        return self.table(table)

    def __iter__(self) -> Iterator[Table]:
        return iter([self.table(name) for name in self.tables()])

    def __len__(self) -> int:
        return len(self.tables())

    def __contains__(self, table: object) -> bool:
        name = table.name if isinstance(table, Table) else table
        return isinstance(name, str) and name in self.tables()

    def info(self) -> DbMeta:
        """Returns the database metadata (``DbMeta``)."""
        return self.client.get_db(self.name)

    def delete(self) -> None:
        self.client.delete_db(self.name)

    def tables(self) -> list[str]:
        return self.client.list_tables(self.name)

    def table(self, name: str) -> Table:
        return Table(self.client, self.name, name)

    def create_table(
        self,
        name: str,
        schema: TableSchema | Mapping[str, Any] | None = None,
    ) -> Table:
        """Creates a table and returns a bound handle to it."""
        self.client.create_table(self.name, name, schema=schema)
        return Table(self.client, self.name, name)

    def drop_table(self, name: str) -> None:
        self.client.delete_table(self.name, name)


class Table:
    """A table handle obtained with ``client.table("app", "users")``."""

    def __init__(self, client: LunaClient, db: str, name: str) -> None:
        self.client = client
        self.db = db
        self.name = name

    def __repr__(self) -> str:
        return f"Table({self.db!r}, {self.name!r})"

    # -- schema ---------------------------------------------------------
    @property
    def schema(self) -> TableSchema:
        return self.get_schema()

    def get_schema(self) -> TableSchema:
        return self.client.get_schema(self.db, self.name)

    def set_schema(
        self,
        schema: TableSchema | Mapping[str, Any],
        *,
        force: bool = False,
    ) -> TableSchema:
        return self.client.set_schema(self.db, self.name, schema, force=force)

    def patch_schema(
        self, patch: Mapping[str, Any], *, force: bool = False
    ) -> TableSchema:
        return self.client.patch_schema(self.db, self.name, patch, force=force)

    def add_fields(
        self,
        fields: Mapping[str, Field | Mapping[str, Any]],
        *,
        force: bool = False,
    ) -> TableSchema:
        return self.client.add_schema_fields(
            self.db, self.name, fields, force=force
        )

    def update_field(
        self,
        field: str,
        changes: Mapping[str, Any],
        *,
        force: bool = False,
    ) -> TableSchema:
        return self.client.update_schema_field(
            self.db, self.name, field, changes, force=force
        )

    def remove_fields(self, *fields: str, force: bool = False) -> TableSchema:
        return self.client.remove_schema_fields(
            self.db, self.name, *fields, force=force
        )

    def add_unique(
        self,
        key: str | Sequence[str],
        *,
        force: bool = False,
    ) -> TableSchema:
        return self.client.add_unique_key(
            self.db, self.name, key, force=force
        )

    def remove_unique(
        self,
        key: str | Sequence[str],
        *,
        force: bool = False,
    ) -> TableSchema:
        return self.client.remove_unique_key(
            self.db, self.name, key, force=force
        )

    def delete_schema(self) -> None:
        self.client.delete_schema(self.db, self.name)

    # -- records --------------------------------------------------------
    def create(
        self,
        record: Mapping[str, Any] | None = None,
        **fields: Any,
    ) -> Record:
        """Creates a record: ``users.create(name="Ana", age=30)``."""
        return self.client.create_record(self.db, self.name, record, **fields)

    def create_many(
        self,
        records: Iterable[Mapping[str, Any]],
    ) -> list[Record]:
        """Creates records one by one, in order (no batch endpoint exists)."""
        return [
            self.client.create_record(self.db, self.name, record)
            for record in records
        ]

    def get(self, record_id: str) -> Record:
        return self.client.get_record(self.db, self.name, record_id)

    def replace(
        self,
        record_id: str,
        record: Mapping[str, Any] | None = None,
        **fields: Any,
    ) -> Record:
        return self.client.replace_record(
            self.db, self.name, record_id, record, **fields
        )

    def update(
        self,
        record_id: str,
        patch: Mapping[str, Any] | None = None,
        **fields: Any,
    ) -> Record:
        """Partial update: ``users.update(id, age=31)``."""
        return self.client.patch_record(
            self.db, self.name, record_id, patch, **fields
        )

    patch = update

    def delete(self, record_id: str) -> None:
        self.client.delete_record(self.db, self.name, record_id)

    def delete_many(self, *record_ids: str) -> None:
        """Deletes several records; accepts varargs or a single iterable."""
        if len(record_ids) == 1 and not isinstance(record_ids[0], str):
            record_ids = tuple(record_ids[0])  # type: ignore[arg-type]
        for record_id in record_ids:
            self.delete(record_id)

    def validate(
        self,
        record: Mapping[str, Any] | None = None,
        **fields: Any,
    ) -> Record:
        return self.client.validate_record(
            self.db, self.name, record, **fields
        )

    def all(
        self,
        *,
        limit: int | None = None,
        offset: int | None = None,
        cursor: str | None = None,
    ) -> list[Record]:
        return self.client.list_records(
            self.db, self.name, limit=limit, offset=offset, cursor=cursor
        )

    def page(
        self,
        *,
        limit: int | None = None,
        offset: int | None = None,
        cursor: str | None = None,
    ) -> RecordPage:
        return self.client.list_records_page(
            self.db, self.name, limit=limit, offset=offset, cursor=cursor
        )

    def iter(
        self,
        *,
        page_size: int = 100,
        limit: int | None = None,
        offset: int | None = None,
    ) -> Iterator[Record]:
        yield from self.client.iter_records(
            self.db,
            self.name,
            page_size=page_size,
            limit=limit,
            offset=offset,
        )

    def query(
        self,
        *,
        filter: Mapping[str, Any] | None = None,
        sort: SortLike | Iterable[SortLike] | None = None,
        fields: str | Iterable[str] | None = None,
        limit: int | None = None,
        offset: int | None = None,
        cursor: str | None = None,
    ) -> QueryResult:
        return self.client.query(
            self.db,
            self.name,
            filter=filter,
            sort=sort,
            fields=fields,
            limit=limit,
            offset=offset,
            cursor=cursor,
        )

    def where(
        self,
        filter: Mapping[str, Any] | None = None,
        **conditions: Any,
    ) -> QueryBuilder:
        """Starts a chainable query.

        >>> users.where(age__gte=18).sort("-age").select("name").all()
        """
        return QueryBuilder(
            self,
            filter=_merge_filter(filter, _conditions_to_filter(conditions)),
        )

    def find_one(
        self,
        filter: Mapping[str, Any] | None = None,
        **conditions: Any,
    ) -> Record | None:
        return self.where(filter, **conditions).first()

    def first(
        self,
        filter: Mapping[str, Any] | None = None,
        **conditions: Any,
    ) -> Record | None:
        return self.find_one(filter, **conditions)

    def count(
        self,
        filter: Mapping[str, Any] | None = None,
        **conditions: Any,
    ) -> int:
        merged = _merge_filter(filter, _conditions_to_filter(conditions))
        return self.client.count(self.db, self.name, filter=merged)

    def aggregate(
        self,
        group_by: str | Iterable[str] | None = None,
        *,
        by: str | Iterable[str] | None = None,
        filter: Mapping[str, Any] | None = None,
        limit: int | None = None,
        aggregates: Mapping[str, str | Mapping[str, Any]] | None = None,
        **specs: str | Mapping[str, Any],
    ) -> list[Record]:
        """Aggregates records; ``by`` is an alias for ``group_by``."""
        return self.client.aggregate(
            self.db,
            self.name,
            aggregates=aggregates,
            group_by=group_by if group_by is not None else by,
            filter=filter,
            limit=limit,
            **specs,
        )

    # -- dunder sugar ---------------------------------------------------
    def __iter__(self) -> Iterator[Record]:
        return self.iter()

    def __len__(self) -> int:
        return self.count()

    def __getitem__(self, record_id: str) -> Record:
        return self.get(record_id)

    def __contains__(self, record_id: object) -> bool:
        return isinstance(record_id, str) and self.client.exists(
            self.db, self.name, record_id
        )


class QueryBuilder:
    """Immutable, chainable query builder returned by ``Table.where()``."""

    def __init__(
        self,
        table: Table,
        *,
        filter: Mapping[str, Any] | None = None,
        sort: Iterable[SortLike] | None = None,
        fields: Iterable[str] | None = None,
        limit: int | None = None,
        offset: int | None = None,
        cursor: str | None = None,
        page_size: int = 100,
    ) -> None:
        self._table = table
        self._filter = dict(filter) if filter is not None else None
        self._sort = list(sort) if sort is not None else None
        self._fields = list(fields) if fields is not None else None
        self._limit = limit
        self._offset = offset
        self._cursor = cursor
        self._page_size = page_size

    def __repr__(self) -> str:
        return (
            f"QueryBuilder(table={self._table!r}, filter={self._filter!r}, "
            f"sort={self._sort!r}, fields={self._fields!r}, "
            f"limit={self._limit!r}, offset={self._offset!r})"
        )

    def _replace(self, **changes: Any) -> QueryBuilder:
        values = {
            "filter": self._filter,
            "sort": self._sort,
            "fields": self._fields,
            "limit": self._limit,
            "offset": self._offset,
            "cursor": self._cursor,
            "page_size": self._page_size,
        }
        values.update(changes)
        return QueryBuilder(self._table, **values)

    def where(
        self,
        filter: Mapping[str, Any] | None = None,
        **conditions: Any,
    ) -> QueryBuilder:
        extra = _conditions_to_filter(conditions)
        if filter is not None:
            extra = {**dict(filter), **extra}
        return self._replace(filter=_merge_filter(self._filter, extra))

    def sort(self, *items: SortLike | Iterable[SortLike]) -> QueryBuilder:
        return self._replace(sort=_flatten(items))

    order_by = sort

    def select(self, *fields: str | Iterable[str]) -> QueryBuilder:
        return self._replace(fields=[str(item) for item in _flatten(fields)])

    def limit(self, value: int | None) -> QueryBuilder:
        return self._replace(limit=value)

    def offset(self, value: int | None) -> QueryBuilder:
        return self._replace(offset=value)

    def cursor(self, value: str | None) -> QueryBuilder:
        return self._replace(cursor=value)

    def page_size(self, value: int) -> QueryBuilder:
        return self._replace(page_size=value)

    def result(self) -> QueryResult:
        return self._table.query(
            filter=self._filter,
            sort=self._sort,
            fields=self._fields,
            limit=self._limit,
            offset=self._offset,
            cursor=self._cursor,
        )

    def all(self) -> list[Record]:
        return list(self.result().records)

    def first(self) -> Record | None:
        return self._replace(limit=1).result().first

    def one(self) -> Record:
        """Returns the first record or raises ``LunaNotFoundError``."""
        record = self.first()
        if record is None:
            raise LunaNotFoundError(404, "record not found")
        return record

    def count(self) -> int:
        return self._replace(limit=0, offset=None, cursor=None).result().total

    def exists(self) -> bool:
        return self.first() is not None

    def ids(self) -> list[str]:
        return [str(record["_id"]) for record in self.all()]

    def __iter__(self) -> Iterator[Record]:
        offset = 0 if self._cursor is not None else (self._offset or 0)
        cap = None if self._limit is None else self._limit + offset
        records = self._table.client.iter_query(
            self._table.db,
            self._table.name,
            page_size=self._page_size,
            limit=cap,
            filter=self._filter,
            sort=self._sort,
            fields=self._fields,
        )
        for index, record in enumerate(records):
            if index < offset:
                continue
            yield record
