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
        client.login("root", "senha-errada")

    bad_key = LunaClient(base_url=server["base_url"], api_key="luna_invalida")
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
    first = admin.create_record(db, "users", {"nome": "Ana", "idade": 30, "tags": ["dev"]})
    second = admin.create_record(db, "users", {"nome": "Bia", "idade": 25, "tags": ["vip"]})
    admin.create_record(db, "users", {"nome": "Caio", "idade": 40, "tags": ["vip", "dev"]})

    assert first["_id"]
    assert admin.get_record(db, "users", first["_id"])["nome"] == "Ana"
    updated = admin.replace_record(
        db, "users", second["_id"], {"nome": "Bia", "idade": 26, "tags": ["vip"]}
    )
    assert updated["idade"] == 26

    result = admin.query(
        db,
        "users",
        filter={"idade": {"$gte": 30}},
        sort=[Sort("idade", "desc")],
        fields=["nome", "idade"],
    )
    assert result.total == 2
    assert [record["nome"] for record in result.records] == ["Caio", "Ana"]
    assert "tags" not in result.records[0]

    by_tag = admin.query(db, "users", filter={"tags": {"$contains": "vip"}}, fields=["nome"])
    assert by_tag.total == 2

    page = admin.query(db, "users", sort=["-idade"], limit=1, offset=1, fields=["nome"])
    assert page.total == 3 and page.count == 1 and page.records[0]["nome"] == "Ana"

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
            "idade": Field(FieldType.INT, min=0, max=150),
            "ativo": Field(FieldType.BOOL, default=True),
        },
    )
    admin.create_table(db, "users", schema=schema)
    assert admin.get_schema(db, "users") == schema

    record = admin.create_record(
        db, "users", {"tenant": "t1", "email": "ana@x.com", "cpf": "111", "idade": 30}
    )
    assert record["ativo"] is True

    with pytest.raises(LunaValidationError) as err:
        admin.create_record(db, "users", {"email": "sem-arroba", "idade": 200, "extra": 1})
    fields = {item["field"] for item in err.value.fields}
    assert {"tenant", "email", "idade", "extra"} <= fields

    with pytest.raises(LunaUniqueViolationError) as err:
        admin.create_record(db, "users", {"tenant": "t1", "email": "ana@x.com", "cpf": "222"})
    assert err.value.fields[0]["field"] == "tenant+email"

    with pytest.raises(LunaUniqueViolationError):
        admin.create_record(db, "users", {"tenant": "t9", "email": "z@x.com", "cpf": "111"})


def test_encryption_at_rest(admin, db, server):
    schema = TableSchema(
        fields={
            "nome": Field(FieldType.STRING, required=True),
            "senha": Field(FieldType.STRING, required=True, encrypted=True),
        }
    )
    admin.create_table(db, "cofre", schema=schema)
    admin.create_record(db, "cofre", {"nome": "Ana", "senha": "super-secreta"})

    table_path = server["storage"] / "dbs" / db / "cofre.msgpack"
    assert table_path.exists(), "tabela deve ser gravada em msgpack"
    raw = table_path.read_bytes()
    assert b"enc:v1:" in raw
    assert b"super-secreta" not in raw

    listed = admin.list_records(db, "cofre")
    assert listed[0]["senha"] == "super-secreta"
    queried = admin.query(db, "cofre", filter={"senha": "super-secreta"}, fields=["nome"])
    assert queried.total == 1 and queried.records[0]["nome"] == "Ana"


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

    self_key, self_plaintext = scoped.create_api_key(name="pessoal")
    assert self_key.name == "pessoal"
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
        pytest.skip("binario nao encontrado")
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
    admin.create_record(db, "t", {"email": "a@x.com", "papel": "user"})
    admin.create_record(db, "t", {"email": "b@x.com", "papel": "user"})

    schema = admin.add_schema_fields(
        db, "t", {"ativo": Field(FieldType.BOOL, default=True)}
    )
    assert schema.fields["ativo"].default is True
    assert all(record["ativo"] is True for record in admin.list_records(db, "t"))

    schema = admin.update_schema_field(db, "t", "ativo", {"required": True})
    assert schema.fields["ativo"].required is True
    admin.add_schema_fields(db, "t", {"papel": Field(FieldType.STRING)})

    with pytest.raises(LunaSchemaRejectedError):
        admin.add_unique_key(db, "t", "papel")
    schema = admin.add_unique_key(db, "t", "papel", force=True)
    assert ("papel",) in schema.unique
    with pytest.raises(LunaBadRequestError):
        admin.add_unique_key(db, "t", "inexistente")
    schema = admin.remove_unique_key(db, "t", "papel")
    assert ("papel",) not in schema.unique

    with pytest.raises(LunaBadRequestError):
        admin.add_schema_fields(db, "t", {"ativo": Field(FieldType.BOOL)})
    with pytest.raises(LunaBadRequestError):
        admin.remove_schema_fields(db, "t", "nao_existe")
    with pytest.raises(LunaBadRequestError):
        admin.update_schema_field(db, "t", "nao_existe", {"required": True})

    schema = admin.remove_schema_fields(db, "t", "ativo")
    assert "ativo" not in schema.fields

    schema = admin.patch_schema(
        db, "t", {"add": {"n": {"type": "int"}}, "strict": False}
    )
    assert "n" in schema.fields

    fresh = admin.patch_schema(
        db,
        "t",
        {
            "add": {"composto": {"type": "string"}, "email": {"type": "string"}},
            "unique_add": [["email", "composto"]],
        },
    )
    assert ("email", "composto") in fresh.unique


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
        writer_client.create_db("nao-pode")
    with pytest.raises(LunaForbiddenError):
        writer_client.list_users()

    admin.delete_user(guest.id)
    admin.delete_user(writer.id)
    with pytest.raises(LunaAuthError):
        guest_client.me()


def uuid_suffix() -> str:
    import uuid

    return uuid.uuid4().hex[:8]
