from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Iterator, Mapping, Self, Sequence

__all__ = [
    "ApiKey",
    "AutoField",
    "DbMeta",
    "Field",
    "FieldType",
    "Grant",
    "Level",
    "Page",
    "QueryResult",
    "Record",
    "RecordPage",
    "Role",
    "Sort",
    "TableSchema",
    "User",
]


class Level(str, Enum):
    READ = "read"
    WRITE = "write"
    ADMIN = "admin"


class FieldType(str, Enum):
    STRING = "string"
    INT = "int"
    FLOAT = "float"
    BOOL = "bool"
    DATETIME = "datetime"
    DATE = "date"
    UUID = "uuid"
    OBJECT = "object"
    ARRAY = "array"
    ANY = "any"


class AutoField(str, Enum):
    CREATED_AT = "created_at"
    UPDATED_AT = "updated_at"


def _wrap(value: Any) -> Any:
    """Turns JSON objects into Records, recursively."""
    if isinstance(value, Record):
        return value
    if isinstance(value, Mapping):
        return Record.from_dict(value)
    if isinstance(value, list):
        return [_wrap(item) for item in value]
    return value


def _unwrap(value: Any) -> Any:
    """Turns Records back into plain JSON-compatible data, recursively."""
    if isinstance(value, Mapping):
        return {key: _unwrap(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_unwrap(item) for item in value]
    if isinstance(value, tuple):
        return [_unwrap(item) for item in value]
    return value


class Record(dict):
    """A record returned by the API.

    Behaves exactly like a ``dict`` (``record["name"]``), but also supports
    attribute access (``record.name``) and conversion back to plain data.

    >>> record = Record.from_dict({"name": "Ana", "address": {"city": "SP"}})
    >>> record.name
    'Ana'
    >>> record.address.city
    'SP'
    >>> record.to_dict()
    {'name': 'Ana', 'address': {'city': 'SP'}}
    """

    def __getattr__(self, name: str) -> Any:
        if name.startswith("__") and name.endswith("__"):
            raise AttributeError(name)
        try:
            return self[name]
        except KeyError:
            raise AttributeError(
                f"record has no field {name!r}"
            ) from None

    def __dir__(self) -> list[str]:
        return sorted({*super().__dir__(), *self.keys()})

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> Record:
        if isinstance(data, Record):
            return data
        return cls({key: _wrap(value) for key, value in data.items()})

    def to_dict(self) -> dict[str, Any]:
        """Returns a plain, JSON-serializable ``dict`` (deep copy)."""
        return {key: _unwrap(value) for key, value in self.items()}


@dataclass(frozen=True)
class Grant:
    db: str
    level: Level

    def __post_init__(self) -> None:
        if not isinstance(self.level, Level):
            object.__setattr__(self, "level", Level(self.level))

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> Grant:
        return cls(db=data["db"], level=Level(data["level"]))

    def to_dict(self) -> dict[str, str]:
        return {"db": self.db, "level": self.level.value}


@dataclass(frozen=True)
class Field:
    type: FieldType
    required: bool = False
    nullable: bool = False
    default: Any = None
    encrypted: bool = False
    unique: bool = False
    min: float | None = None
    max: float | None = None
    min_length: int | None = None
    max_length: int | None = None
    pattern: str | None = None
    enum: tuple[Any, ...] | None = None
    items: Field | None = None
    fields: dict[str, Field] | None = None
    auto: AutoField | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.type, FieldType):
            object.__setattr__(self, "type", FieldType(self.type))
        if self.auto is not None and not isinstance(self.auto, AutoField):
            object.__setattr__(self, "auto", AutoField(self.auto))
        if self.items is not None and not isinstance(self.items, Field):
            object.__setattr__(self, "items", Field.from_value(self.items))
        if self.fields is not None:
            object.__setattr__(
                self,
                "fields",
                {
                    name: Field.from_value(item)
                    for name, item in self.fields.items()
                },
            )
        if self.enum is not None and not isinstance(self.enum, tuple):
            if isinstance(self.enum, str) or not hasattr(self.enum, "__iter__"):
                values = (self.enum,)
            else:
                values = tuple(self.enum)
            object.__setattr__(self, "enum", values)

    @classmethod
    def from_value(cls, value: Field | str | Mapping[str, Any]) -> Field:
        """Accepts a ``Field``, a ``FieldType``/``str`` or a raw schema dict."""
        if isinstance(value, Field):
            return value
        if isinstance(value, Mapping):
            return cls.from_dict(dict(value))
        return cls(value)

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> Field:
        return cls(
            type=FieldType(data["type"]),
            required=bool(data.get("required", False)),
            nullable=bool(data.get("nullable", False)),
            default=data.get("default"),
            encrypted=bool(data.get("encrypted", False)),
            unique=bool(data.get("unique", False)),
            min=data.get("min"),
            max=data.get("max"),
            min_length=data.get("min_length"),
            max_length=data.get("max_length"),
            pattern=data.get("pattern"),
            enum=tuple(data["enum"]) if data.get("enum") is not None else None,
            items=Field.from_dict(data["items"]) if data.get("items") else None,
            fields=(
                {name: Field.from_dict(item) for name, item in data["fields"].items()}
                if data.get("fields")
                else None
            ),
            auto=AutoField(data["auto"]) if data.get("auto") else None,
        )

    def to_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = {"type": self.type.value}
        if self.required:
            out["required"] = True
        if self.nullable:
            out["nullable"] = True
        if self.default is not None:
            out["default"] = self.default
        if self.encrypted:
            out["encrypted"] = True
        if self.unique:
            out["unique"] = True
        for name in ("min", "max", "min_length", "max_length", "pattern"):
            value = getattr(self, name)
            if value is not None:
                out[name] = value
        if self.enum is not None:
            out["enum"] = list(self.enum)
        if self.items is not None:
            out["items"] = self.items.to_dict()
        if self.fields is not None:
            out["fields"] = {name: item.to_dict() for name, item in self.fields.items()}
        if self.auto is not None:
            out["auto"] = self.auto.value
        return out

    # Readable shortcuts: Field.string(required=True), Field.int(min=0), ...
    @classmethod
    def string(cls, **kwargs: Any) -> Field:
        return cls(FieldType.STRING, **kwargs)

    str = string

    @classmethod
    def int(cls, **kwargs: Any) -> Field:
        return cls(FieldType.INT, **kwargs)

    integer = int

    @classmethod
    def float(cls, **kwargs: Any) -> Field:
        return cls(FieldType.FLOAT, **kwargs)

    @classmethod
    def bool(cls, **kwargs: Any) -> Field:
        return cls(FieldType.BOOL, **kwargs)

    boolean = bool

    @classmethod
    def datetime(cls, **kwargs: Any) -> Field:
        return cls(FieldType.DATETIME, **kwargs)

    @classmethod
    def date(cls, **kwargs: Any) -> Field:
        return cls(FieldType.DATE, **kwargs)

    @classmethod
    def uuid(cls, **kwargs: Any) -> Field:
        return cls(FieldType.UUID, **kwargs)

    @classmethod
    def object(cls, **kwargs: Any) -> Field:
        return cls(FieldType.OBJECT, **kwargs)

    @classmethod
    def array(cls, **kwargs: Any) -> Field:
        return cls(FieldType.ARRAY, **kwargs)

    @classmethod
    def any(cls, **kwargs: Any) -> Field:
        return cls(FieldType.ANY, **kwargs)


@dataclass(frozen=True)
class TableSchema:
    fields: Mapping[str, Field | str | Mapping[str, Any]] = field(
        default_factory=dict
    )
    strict: bool = False
    unique: (
        str
        | Sequence[str]
        | Sequence[Sequence[str]]
        | None
    ) = ()

    def __post_init__(self) -> None:
        object.__setattr__(
            self,
            "fields",
            {
                name: Field.from_value(item)
                for name, item in self.fields.items()
            },
        )
        if self.unique is None:
            normalized: list[tuple[str, ...]] = []
        elif isinstance(self.unique, str):
            normalized = [(self.unique,)]
        else:
            normalized = []
            for key in self.unique:
                normalized.append((key,) if isinstance(key, str) else tuple(key))
        object.__setattr__(self, "unique", tuple(normalized))

    @classmethod
    def from_value(cls, value: TableSchema | Mapping[str, Any]) -> TableSchema:
        if isinstance(value, TableSchema):
            return value
        if isinstance(value, Mapping):
            return cls.from_dict(dict(value))
        raise TypeError("schema must be a TableSchema or a dict")

    @classmethod
    def of(
        cls,
        *,
        strict: bool = False,
        unique: str | Sequence[str] | Sequence[Sequence[str]] | None = (),
        **fields: Field | str | Mapping[str, Any],
    ) -> TableSchema:
        """Builds a schema from keyword arguments.

        >>> TableSchema.of(
        ...     email=Field.string(required=True, unique=True),
        ...     age=Field.int(min=0),
        ... )
        """
        return cls(fields=fields, strict=strict, unique=unique)

    from_fields = of

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> TableSchema:
        return cls(
            fields=dict(data.get("fields") or {}),
            strict=bool(data.get("strict", False)),
            unique=data.get("unique") or (),
        )

    def to_dict(self) -> dict[str, Any]:
        output: dict[str, Any] = {
            "strict": self.strict,
            "fields": {name: item.to_dict() for name, item in self.fields.items()},
        }
        if self.unique:
            keys: list[Any] = []
            for key in self.unique:
                if len(key) == 1:
                    keys.append(key[0])
                else:
                    keys.append(list(key))
            output["unique"] = keys
        return output

    def __getitem__(self, name: str) -> Field:
        return self.fields[name]

    def __contains__(self, name: object) -> bool:
        return name in self.fields

    def __iter__(self) -> Iterator[str]:
        return iter(self.fields)

    def __len__(self) -> int:
        return len(self.fields)


@dataclass(frozen=True)
class Sort:
    field: str
    dir: str = "asc"

    def __post_init__(self) -> None:
        direction = str(self.dir).lower()
        if direction not in ("asc", "desc"):
            raise ValueError("sort direction must be 'asc' or 'desc'")
        object.__setattr__(self, "dir", direction)

    @classmethod
    def asc(cls, field: str) -> Sort:
        return cls(field=field, dir="asc")

    @classmethod
    def desc(cls, field: str) -> Sort:
        return cls(field=field, dir="desc")

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> Sort:
        return cls(field=data["field"], dir=data.get("dir", "asc"))

    def to_dict(self) -> dict[str, str]:
        return {"field": self.field, "dir": self.dir}


@dataclass(frozen=True)
class Page:
    """A page of records, with the cursor that leads to the next one."""

    total: int
    count: int
    records: tuple[Record, ...]
    next_cursor: str | None = None

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> Self:
        records = data.get("records") or []
        return cls(
            total=int(data.get("total", len(records))),
            count=int(data.get("count", len(records))),
            records=tuple(Record.from_dict(item) for item in records),
            next_cursor=data.get("next_cursor"),
        )

    @property
    def first(self) -> Record | None:
        """The first record of the page, or ``None`` when it is empty."""
        return self.records[0] if self.records else None

    def __iter__(self) -> Iterator[Record]:
        return iter(self.records)

    def __len__(self) -> int:
        return len(self.records)

    def __getitem__(self, index: int) -> Record:
        return self.records[index]

    def __bool__(self) -> bool:
        return bool(self.records)


@dataclass(frozen=True)
class QueryResult(Page):
    """Result of ``query()``: filtered total plus the records of this page."""


@dataclass(frozen=True)
class RecordPage(Page):
    """A page of records from the list endpoint."""


@dataclass(frozen=True)
class ApiKey:
    id: str
    name: str
    prefix: str
    created_at: str
    last_used_at: str | None
    revoked: bool

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> ApiKey:
        return cls(
            id=data["id"],
            name=data["name"],
            prefix=data.get("prefix", ""),
            created_at=data["created_at"],
            last_used_at=data.get("last_used_at"),
            revoked=bool(data.get("revoked", False)),
        )


@dataclass(frozen=True)
class User:
    id: str
    username: str
    role: str
    grants: tuple[Grant, ...]
    created_at: str

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> User:
        return cls(
            id=data["id"],
            username=data["username"],
            role=data["role"],
            grants=tuple(Grant.from_dict(item) for item in data.get("grants", [])),
            created_at=data["created_at"],
        )

    def __str__(self) -> str:
        return self.username


@dataclass(frozen=True)
class Role:
    name: str
    description: str
    builtin: bool
    permissions: tuple[Grant, ...]

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> Role:
        return cls(
            name=data["name"],
            description=data.get("description", ""),
            builtin=bool(data.get("builtin", False)),
            permissions=tuple(
                Grant.from_dict(item) for item in data.get("permissions", [])
            ),
        )


@dataclass(frozen=True)
class DbMeta:
    name: str
    owner_id: str
    created_at: str

    @classmethod
    def from_dict(cls, data: Mapping[str, Any]) -> DbMeta:
        return cls(
            name=data["name"],
            owner_id=data["owner_id"],
            created_at=data["created_at"],
        )
