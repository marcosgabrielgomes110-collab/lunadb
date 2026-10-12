from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

from lunapy import (
    Field,
    FieldType,
    Grant,
    Level,
    LunaAuthError,
    LunaBadRequestError,
    LunaClient,
    LunaForbiddenError,
    LunaNotFoundError,
    LunaSchemaRejectedError,
    LunaUniqueViolationError,
    LunaValidationError,
    Sort,
    TableSchema,
)

REPO_ROOT = Path(__file__).resolve().parents[2]


def test_health_and_auth_errors(server):
    client = LunaClient(base_url=server["base_url"])
    assert client.health()["status"] == "ok"

    with pytest.raises(LunaAuthError):
        client.login("root", "wrong-password")

    bad_key = LunaClient(base_url=server["base_url"], api_key="luna_invalid")
    with pytest.raises(LunaAuthError):
        bad_key.me()

    anonymous = LunaClient(base_url=server["base_url"])
    with pytest.raises(Exception):
        anonymous.me()


def test_status_and_rotate(server):
    client = LunaClient(base_url=server["base_url"])
    client.login("root", "pytest-secret")
    status = client.status()
    assert status["host"] == "127.0.0.1"
    assert status["uptime_seconds"] >= 0
    assert status["users"] >= 1

    rotated = LunaClient(base_url=server["base_url"], api_key=client.rotate_key())
    assert rotated.me().username == "root"

    client.login("root", "pytest-secret")


def test_records_crud_and_query(admin, db):
    admin.create_table(db, "users")
    first = admin.create_record(db, "users", {"name": "Ana", "age": 30, "tags": ["dev"]})
    second = admin.create_record(db, "users", {"name": "Bia", "age": 25, "tags": ["vip"]})
    admin.create_record(db, "users", {"name": "Caio", "age": 40, "tags": ["vip", "dev"]})

    assert first["_id"]
    assert admin.get_record(db, "users", first["_id"])["name"] == "Ana"
    updated = admin.replace_record(
        db, "users", second["_id"], {"name": "Bia", "age": 26, "tags": ["vip"]}
    )
    assert updated["age"] == 26

    result = admin.query(
        db,
        "users",
        filter={"age": {"$gte": 30}},
        sort=[Sort("age", "desc")],
        fields=["name", "age"],
    )
    assert result.total == 2
    assert [record["name"] for record in result.records] == ["Caio", "Ana"]
    assert "tags" not in result.records[0]

    by_tag = admin.query(db, "users", filter={"tags": {"$contains": "vip"}}, fields=["name"])
    assert by_tag.total == 2

    page = admin.query(db, "users", sort=["-age"], limit=1, offset=1, fields=["name"])
    assert page.total == 3 and page.count == 1 and page.records[0]["name"] == "Ana"

    admin.delete_record(db, "users", first["_id"])
    assert len(admin.list_records(db, "users")) == 2

    with pytest.raises(LunaNotFoundError):
        admin.get_record(db, "users", "00000000-0000-0000-0000-000000000000")


def test_schema_validation_and_unique(admin, db):
    schema = TableSchema(
        strict=True,
        unique=("cpf", ("tenant", "email")),
        fields={
            "tenant": Field(FieldType.STRING, required=True),
            "email": Field(FieldType.STRING, required=True, pattern=r".+@.+\..+"),
            "cpf": Field(FieldType.STRING, nullable=True),
            "age": Field(FieldType.INT, min=0, max=150),
            "active": Field(FieldType.BOOL, default=True),
        },
    )
    admin.create_table(db, "users", schema=schema)
    assert admin.get_schema(db, "users") == schema

    record = admin.create_record(
        db, "users", {"tenant": "t1", "email": "ana@x.com", "cpf": "111", "age": 30}
    )
    assert record["active"] is True

    with pytest.raises(LunaValidationError) as err:
        admin.create_record(db, "users", {"email": "no-at-sign", "age": 200, "extra": 1})
    fields = {item["field"] for item in err.value.fields}
    assert {"tenant", "email", "age", "extra"} <= fields

    with pytest.raises(LunaUniqueViolationError) as err:
        admin.create_record(db, "users", {"tenant": "t1", "email": "ana@x.com", "cpf": "222"})
    assert err.value.fields[0]["field"] == "tenant+email"

    with pytest.raises(LunaUniqueViolationError):
        admin.create_record(db, "users", {"tenant": "t9", "email": "z@x.com", "cpf": "111"})


def test_encryption_at_rest(admin, db, server):
    schema = TableSchema(
        fields={
            "name": Field(FieldType.STRING, required=True),
            "password": Field(FieldType.STRING, required=True, encrypted=True),
        }
    )
    admin.create_table(db, "vault", schema=schema)
    admin.create_record(db, "vault", {"name": "Ana", "password": "super-secret"})

    table_path = server["storage"] / "dbs" / db / "vault.msgpack"
    assert table_path.exists(), "table must be stored as msgpack"
    raw = table_path.read_bytes()
    assert b"enc:v1:" in raw
    assert b"super-secret" not in raw

    listed = admin.list_records(db, "vault")
    assert listed[0]["password"] == "super-secret"
    queried = admin.query(db, "vault", filter={"password": "super-secret"}, fields=["name"])
    assert queried.total == 1 and queried.records[0]["name"] == "Ana"


def test_api_keys_sdk(admin, server):
    user = admin.create_user(f"keys{uuid_suffix()}", "keys12345")
    key, plaintext = admin.create_api_key(user.id, name="ci")
    assert key.name == "ci" and key.revoked is False
    assert key.prefix.startswith("luna_")
    assert key.id

    scoped = LunaClient(base_url=admin.base_url, api_key=plaintext)
    assert scoped.me().username == user.username

    login_client = LunaClient(base_url=admin.base_url)
    login_client.login(user.username, "keys12345")
    keys = admin.list_api_keys(user.id)
    assert {item.name for item in keys} == {"ci", "login"}
    assert scoped.me().username == user.username

    self_key, self_plaintext = scoped.create_api_key(name="personal")
    assert self_key.name == "personal"
    assert len(scoped.list_api_keys()) >= 3

    with pytest.raises(LunaForbiddenError):
        scoped.list_api_keys(admin.me().id)

    revoked = admin.revoke_api_key(key.id, user.id)
    assert revoked.revoked is True
    with pytest.raises(LunaAuthError):
        scoped.me()
    still_valid = LunaClient(base_url=admin.base_url, api_key=self_plaintext)
    assert still_valid.me().username == user.username

    admin.delete_user(user.id)


def test_cli_apikey_commands(admin, server):
    binary = Path(
        os.environ.get("LUNADB_BIN", REPO_ROOT / "target" / "debug" / "lunadb")
    )
    if not binary.exists():
        pytest.skip("binary not found")
    user = admin.create_user(f"cli{uuid_suffix()}", "cli12345678")
    env = {**os.environ, "LUNADB_API_KEY": admin.api_key}

    def run(*args: str) -> str:
        result = subprocess.run(
            [str(binary), "--config", str(server["config"]), *args],
            capture_output=True,
            text=True,
            env=env,
        )
        assert result.returncode == 0, result.stderr
        return result.stdout

    created = run("apikey", "create", user.username, "--name", "cli-key").strip()
    assert created.startswith("luna_")

    listing = json.loads(run("apikey", "list", user.username))
    assert any(item["name"] == "cli-key" for item in listing)

    prefix = next(item["prefix"] for item in listing if item["name"] == "cli-key")
    run("apikey", "revoke", user.username, prefix.rstrip("."))
    listing = json.loads(run("apikey", "list", user.username))
    revoked = next(item for item in listing if item["name"] == "cli-key")
    assert revoked["revoked"] is True

    admin.delete_user(user.id)


def test_schema_migration(admin, db):
    admin.create_table(db, "t", schema=TableSchema(fields={"a": Field(FieldType.STRING)}))
    admin.create_record(db, "t", {"a": "x"})

    with pytest.raises(LunaSchemaRejectedError) as err:
        admin.set_schema(
            db,
            "t",
            {"strict": False, "fields": {"a": {"type": "string"}, "b": {"type": "string", "required": True}}},
        )
    assert err.value.records[0]["fields"][0]["field"] == "b"

    admin.set_schema(
        db,
        "t",
        {"strict": False, "fields": {"a": {"type": "string"}, "b": {"type": "string", "required": True}}},
        force=True,
    )
    admin.set_schema(
        db,
        "t",
        {"strict": False, "fields": {"a": {"type": "string"}, "b": {"type": "string", "default": "ok"}}},
    )
    records = admin.list_records(db, "t")
    assert records[0]["b"] == "ok"
    admin.delete_schema(db, "t")


def test_schema_patch_sdk(admin, db):
    admin.create_table(db, "t")
    admin.create_record(db, "t", {"email": "a@x.com", "role": "user"})
    admin.create_record(db, "t", {"email": "b@x.com", "role": "user"})

    schema = admin.add_schema_fields(
        db, "t", {"active": Field(FieldType.BOOL, default=True)}
    )
    assert schema.fields["active"].default is True
    assert all(record["active"] is True for record in admin.list_records(db, "t"))

    schema = admin.update_schema_field(db, "t", "active", {"required": True})
    assert schema.fields["active"].required is True
    admin.add_schema_fields(db, "t", {"role": Field(FieldType.STRING)})

    with pytest.raises(LunaSchemaRejectedError):
        admin.add_unique_key(db, "t", "role")
    schema = admin.add_unique_key(db, "t", "role", force=True)
    assert ("role",) in schema.unique
    with pytest.raises(LunaBadRequestError):
        admin.add_unique_key(db, "t", "does_not_exist")
    schema = admin.remove_unique_key(db, "t", "role")
    assert ("role",) not in schema.unique

    with pytest.raises(LunaBadRequestError):
        admin.add_schema_fields(db, "t", {"active": Field(FieldType.BOOL)})
    with pytest.raises(LunaBadRequestError):
        admin.remove_schema_fields(db, "t", "does_not_exist")
    with pytest.raises(LunaBadRequestError):
        admin.update_schema_field(db, "t", "does_not_exist", {"required": True})

    schema = admin.remove_schema_fields(db, "t", "active")
    assert "active" not in schema.fields

    schema = admin.patch_schema(
        db, "t", {"add": {"n": {"type": "int"}}, "strict": False}
    )
    assert "n" in schema.fields

    fresh = admin.patch_schema(
        db,
        "t",
        {
            "add": {"composite": {"type": "string"}, "email": {"type": "string"}},
            "unique_add": [["email", "composite"]],
        },
    )
    assert ("email", "composite") in fresh.unique


def test_patch_aggregate_and_password(admin, db):
    admin.create_table(db, "sales")
    admin.create_record(db, "sales", {"city": "SP", "value": 10, "notes": "a"})
    admin.create_record(db, "sales", {"city": "SP", "value": 20})
    admin.create_record(db, "sales", {"city": "RJ", "value": 5})
    record = admin.create_record(db, "sales", {"city": "RJ", "value": 7, "notes": "b"})

    patched = admin.patch_record(db, "sales", record["_id"], {"value": 8, "notes": None})
    assert patched["value"] == 8 and "notes" not in patched

    groups = admin.aggregate(
        db,
        "sales",
        group_by=["city"],
        aggregates={
            "n": "count",
            "sum": "sum:value",
            "avg": {"op": "avg", "field": "value"},
        },
    )
    sp = next(group for group in groups if group["key"]["city"] == "SP")
    assert sp["n"] == 2 and sp["sum"] == 30 and sp["avg"] == 15

    user = admin.create_user(f"pw{uuid_suffix()}", "initial123")
    client = LunaClient(base_url=admin.base_url)
    client.login(user.username, "initial123")
    client.set_password(user.id, "newpassword123", current_password="initial123")
    with pytest.raises(LunaAuthError):
        client.me()
    client.login(user.username, "newpassword123")
    admin.delete_user(user.id)


def test_users_and_permissions(admin, db):
    guest = admin.create_user(
        f"guest{uuid_suffix()}",
        "guest12345",
        grants=[Grant(db=db, level=Level.READ)],
    )
    writer = admin.create_user(
        f"writer{uuid_suffix()}",
        "writer12345",
        grants=[(db, "write")],
    )

    guest_client = LunaClient(base_url=admin.base_url)
    guest_client.login(guest.username, "guest12345")
    assert [meta.name for meta in guest_client.list_dbs()] == [db]
    admin.create_table(db, "notes")

    with pytest.raises(LunaForbiddenError):
        guest_client.create_record(db, "notes", {"x": 1})
    admin.create_record(db, "notes", {"x": 1})
    assert len(guest_client.list_records(db, "notes")) == 1

    writer_client = LunaClient(base_url=admin.base_url)
    writer_client.login(writer.username, "writer12345")
    writer_client.create_record(db, "notes", {"x": 2})
    with pytest.raises(LunaForbiddenError):
        writer_client.create_db("cannot-create")
    with pytest.raises(LunaForbiddenError):
        writer_client.list_users()

    admin.delete_user(guest.id)
    admin.delete_user(writer.id)
    with pytest.raises(LunaAuthError):
        guest_client.me()


def uuid_suffix() -> str:
    import uuid

    return uuid.uuid4().hex[:8]
