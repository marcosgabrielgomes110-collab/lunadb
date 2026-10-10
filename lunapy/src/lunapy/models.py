from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Any


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


@dataclass(frozen=True)
class Grant:
    db: str
    level: Level

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> Grant:
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

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> Field:
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


@dataclass(frozen=True)
class TableSchema:
    fields: dict[str, Field]
    strict: bool = False
    unique: tuple[tuple[str, ...], ...] = ()

    def __post_init__(self) -> None:
        normalized: list[tuple[str, ...]] = []
        for key in self.unique:
            if isinstance(key, str):
                normalized.append((key,))
            else:
                normalized.append(tuple(key))
        object.__setattr__(self, "unique", tuple(normalized))

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> TableSchema:
        unique: list[tuple[str, ...]] = []
        for item in data.get("unique") or []:
            if isinstance(item, str):
                unique.append((item,))
            else:
                unique.append(tuple(item))
        return cls(
            fields={
                name: Field.from_dict(item)
                for name, item in (data.get("fields") or {}).items()
            },
            strict=bool(data.get("strict", False)),
            unique=tuple(unique),
        )

    def to_dict(self) -> dict[str, Any]:
        output: dict[str, Any] = {
            "strict": self.strict,
            "fields": {name: item.to_dict() for name, item in self.fields.items()},
        }
        if self.unique:
            keys: list[Any] = []
            for key in self.unique:
                if isinstance(key, str):
                    keys.append(key)
                elif len(key) == 1:
                    keys.append(key[0])
                else:
                    keys.append(list(key))
            output["unique"] = keys
        return output


@dataclass(frozen=True)
class Sort:
    field: str
    dir: str = "asc"

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> Sort:
        return cls(field=data["field"], dir=data.get("dir", "asc"))

    def to_dict(self) -> dict[str, str]:
        return {"field": self.field, "dir": self.dir}


@dataclass(frozen=True)
class QueryResult:
    total: int
    count: int
    records: tuple[dict[str, Any], ...]

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> QueryResult:
        records = data.get("records") or []
        return cls(
            total=int(data.get("total", len(records))),
            count=int(data.get("count", len(records))),
            records=tuple(records),
        )


@dataclass(frozen=True)
class User:
    id: str
    username: str
    role: str
    grants: tuple[Grant, ...]
    created_at: str

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> User:
        return cls(
            id=data["id"],
            username=data["username"],
            role=data["role"],
            grants=tuple(Grant.from_dict(item) for item in data.get("grants", [])),
            created_at=data["created_at"],
        )


@dataclass(frozen=True)
class Role:
    name: str
    description: str
    builtin: bool
    permissions: tuple[Grant, ...]

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> Role:
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
    def from_dict(cls, data: dict[str, Any]) -> DbMeta:
        return cls(
            name=data["name"],
            owner_id=data["owner_id"],
            created_at=data["created_at"],
        )
