from __future__ import annotations

import json

import pytest

from lunapy import (
    AutoField,
    Field,
    FieldType,
    Level,
    LunaClient,
    LunaError,
    LunaNotFoundError,
    LunaValidationError,
    Record,
    Sort,
    TableSchema,
    connect,
)


def test_resource_handles_and_keyword_records(admin, db):
    database = admin.db(db)
    assert database.name == db
    assert database.info().name == db

    people = database.create_table("people")
    assert people.name == "people"
    assert people in database
    assert "people" in database.tables()
    assert [table.name for table in database] == ["people"]

    record = people.create(name="Ana", age=30, tags=["dev"])
    assert isinstance(record, Record)
    assert isinstance(record, dict)
    assert record.name == "Ana"
    assert record.tags == ["dev"]
    assert json.loads(json.dumps(record.to_dict()))["name"] == "Ana"

    assert people[record["_id"]].name == "Ana"
    assert record["_id"] in people
    assert len(people) == 1

    updated = people.update(record["_id"], age=31)
    assert updated.age == 31
    patched = people.patch(record["_id"], {"age": 32, "tags": ["vip"]})
    assert patched.age == 32 and patched.tags == ["vip"]
    replaced = people.replace(record["_id"], name="Ana", age=33)
    assert replaced.age == 33

    preview = people.validate(name="Bia", age=20)
    assert preview.name == "Bia" and "_id" not in preview

    people.delete(record["_id"])
    assert record["_id"] not in people
    assert len(people) == 0

    people.create_many([{"name": "Caio"}, {"name": "Duda"}])
    assert sorted(item.name for item in people) == ["Caio", "Duda"]
    assert len(people.page(limit=1).records) == 1

    people.delete_many([item["_id"] for item in people.all()])
    assert people.count() == 0


def test_query_builder_and_aggregation(admin, db):
    people = admin.db(db).create_table("people")
    people.create_many(
        [
            {"name": "Ana", "age": 30, "city": "SP", "tags": ["dev"]},
            {"name": "Bia", "age": 25, "city": "RJ", "tags": ["vip"]},
            {"name": "Caio", "age": 40, "city": "SP", "tags": ["vip", "dev"]},
        ]
    )

    names = people.where(age__gte=30).sort("-age").select("name").all()
    assert [item.name for item in names] == ["Caio", "Ana"]
    assert "age" not in names[0]

    assert people.where(tags__contains="vip").count() == 2
    assert people.where(name__startswith="A").ids() == [
        people.first(name="Ana")["_id"]
    ]
    assert people.where(age__lt=30).exists() is True
    assert people.where(age__lt=18).exists() is False
    assert people.find_one(city="SP").name in {"Ana", "Caio"}
    assert people.first(name="Zoe") is None
    with pytest.raises(LunaNotFoundError):
        people.where(name="Zoe").one()

    ordered = [item.name for item in people.where(age__gte=25).sort("age")]
    assert ordered == ["Bia", "Ana", "Caio"]
    assert [item.name for item in people.where().sort("age").limit(2).all()] == [
        "Bia",
        "Ana",
    ]
    offset_page = people.where().sort("age").offset(1).limit(1).all()
    assert [item.name for item in offset_page] == ["Ana"]
    assert people.count() == 3
    assert people.count(age__gte=30) == 2
    assert people.where(filter={"age": {"$gte": 30}}).count() == 2

    groups = people.aggregate(group_by="city", n="count", total="sum:age")
    sp = next(group for group in groups if group.key.city == "SP")
    assert sp.n == 2 and sp.total == 70

    with pytest.raises(LunaError):
        people.aggregate(by="city")


def test_pythonic_models_and_schema_shortcuts(admin, db):
    schema = TableSchema.of(
        email=Field.string(
            required=True, unique=True, pattern=r".+@.+\..+"
        ),
        age=Field.int(min=0, max=150),
        active=Field.bool(default=True),
        tags=Field.array(items="string"),
        created=Field.datetime(auto="created_at"),
        note="string",
        tenant=Field(FieldType.STRING, required=True),
    )
    assert schema["tags"].items.type is FieldType.STRING
    assert schema["created"].auto is AutoField.CREATED_AT
    assert schema["note"].type is FieldType.STRING
    assert "email" in schema and len(schema) == 7
    assert TableSchema(unique="email").unique == (("email",),)

    table = admin.db(db).create_table("accounts", schema)
    assert table.schema == schema

    record = table.create(email="a@x.com", age=30, tenant="t1")
    assert record.active is True
    with pytest.raises(LunaValidationError):
        table.create(email="invalid", tenant="t1")

    schema = table.add_fields({"nickname": Field.string(nullable=True)})
    assert "nickname" in schema
    schema = table.update_field("nickname", {"required": True}, force=True)
    assert schema["nickname"].required is True
    schema = table.add_unique("nickname", force=True)
    assert ("nickname",) in schema.unique
    schema = table.remove_unique("nickname", force=True)
    assert ("nickname",) not in schema.unique
    schema = table.remove_fields("nickname")
    assert "nickname" not in schema

    table.delete_schema()
    with pytest.raises(LunaNotFoundError):
        table.schema


def test_client_extras_remain_backwards_compatible(admin, db, server):
    assert admin.table(db, "notes").client is admin

    people = admin[db].create_table("people")
    people.create(name="Ana", n=1)
    people.create({"name": "Bia", "n": 2})

    # single values are not iterated character by character anymore
    result = people.query(sort="-n", fields="n")
    assert [record.n for record in result.records] == [2, 1]
    assert result.first.n == 2 and len(result) == 2 and bool(result)

    flat = admin.query(db, "people", sort=("n", "asc"), fields=["n"])
    assert [record.n for record in flat.records] == [1, 2]

    assert admin.count(db, "people") == 2
    assert admin.count(db, "people", filter={"n": {"$gte": 2}}) == 1
    assert admin.exists(db, "people", people.first()["_id"])
    assert not admin.exists(
        db, "people", "00000000-0000-0000-0000-000000000000"
    )

    preview = admin.validate_record(db, "people", name="Caio", n=3)
    assert preview.name == "Caio"

    with LunaClient.connect(
        base_url=server["base_url"],
        username="root",
        password="pytest-secret",
    ) as client:
        assert client.me().username == "root"

    with connect(
        server["base_url"], username="root", password="pytest-secret"
    ) as client:
        assert client.me().username == "root"

    # logging in rotates the root "login" key, so refresh the fixture client
    admin.login("root", "pytest-secret")

    # old positional style still works
    old = admin.create_record(db, "people", {"name": "Duda", "n": 4})
    assert admin.get_record(db, "people", old["_id"])["name"] == "Duda"
    admin.update_record(db, "people", old["_id"], {"n": 5})
    assert admin.get_record(db, "people", old["_id"])["n"] == 5

    assert admin.aggregate(
        db, "people", group_by="n", total="count"
    )[0].total >= 1


def test_sort_and_grant_are_forgiving(admin, db):
    assert Sort("age", "DESC").to_dict() == {"field": "age", "dir": "desc"}
    assert Sort.desc("age").to_dict() == {"field": "age", "dir": "desc"}
    with pytest.raises(ValueError):
        Sort("age", "sideways")

    user = admin.create_user(
        "grantpythonic",
        "password123",
        grants=[{"db": db, "level": "read"}],
    )
    assert user.grants[0].level is Level.READ
    admin.delete_user(user.id)
