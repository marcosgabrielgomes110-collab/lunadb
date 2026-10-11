mod common;

use axum::http::StatusCode;
use common::TestServer;
use serde_json::{Value, json};

#[tokio::test]
async fn health_and_auth_flow() {
    let server = TestServer::spawn().await;

    let (status, body) = server.request("GET", "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");

    let (status, _) = server.request("GET", "/dbs", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = server
        .request(
            "POST",
            "/auth/login",
            None,
            Some(json!({"username": "root", "password": "errada"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let token = server.login_root().await;
    let (status, me) = server.request("GET", "/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["username"], "root");
    assert_eq!(me["role"], "admin");

    let (status, body) = server.request("GET", "/status", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["users"], json!(1));
    assert_eq!(body["dbs"], json!(0));

    let (status, body) = server
        .request("POST", "/auth/rotate-key", Some(&token), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let new_token = body["api_key"].as_str().unwrap().to_string();
    assert_ne!(new_token, token);

    let (status, _) = server.request("GET", "/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = server.request("GET", "/me", Some(&new_token), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn users_and_permissions() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;

    let (status, _) = server
        .request("POST", "/dbs", Some(&root), Some(json!({"name": "app"})))
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables",
            Some(&root),
            Some(json!({"name": "notes"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, bob) = server
        .request(
            "POST",
            "/users",
            Some(&root),
            Some(json!({
                "username": "bob",
                "password": "bob12345",
                "grants": [{"db": "app", "level": "write"}]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bob["grants"][0]["level"], "write");

    let (status, _) = server
        .request(
            "POST",
            "/users",
            Some(&root),
            Some(json!({
                "username": "bob",
                "password": "outra12345"
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = server
        .request(
            "POST",
            "/users",
            Some(&root),
            Some(json!({
                "username": "carol",
                "password": "carol123",
                "grants": [{"db": "app", "level": "read"}]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let bob = server.login("bob", "bob12345").await;
    let carol = server.login("carol", "carol123").await;

    let (status, dbs) = server.request("GET", "/dbs", Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(dbs.as_array().unwrap().len(), 1);

    let (status, _) = server
        .request("POST", "/dbs", Some(&bob), Some(json!({"name": "hack"})))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = server.request("GET", "/users", Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = server.request("GET", "/roles", Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = server.request("GET", "/status", Some(&bob), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/notes/records",
            Some(&bob),
            Some(json!({"titulo": "do bob"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/notes/records",
            Some(&carol),
            Some(json!({"titulo": "da carol"})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = server
        .request("GET", "/dbs/app/tables/notes/records", Some(&carol), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    let bob_id = bob_user_id(&server, &root).await;
    let (status, _) = server
        .request("DELETE", &format!("/users/{bob_id}"), Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = server.request("GET", "/me", Some(&bob), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

async fn bob_user_id(server: &TestServer, root: &str) -> String {
    let (_, users) = server.request("GET", "/users", Some(root), None).await;
    users
        .as_array()
        .unwrap()
        .iter()
        .find(|user| user["username"] == "bob")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn schema_fixture() -> Value {
    json!({
        "strict": true,
        "unique": ["cpf", ["tenant", "email"]],
        "fields": {
            "tenant": {"type": "string", "required": true},
            "email": {"type": "string", "required": true, "pattern": ".+@.+\\..+"},
            "cpf": {"type": "string", "nullable": true},
            "nome": {"type": "string", "required": true},
            "senha": {"type": "string", "encrypted": true},
            "idade": {"type": "int", "min": 0, "max": 150},
            "ativo": {"type": "bool", "default": true},
            "criado": {"type": "datetime", "auto": "created_at"}
        }
    })
}

#[tokio::test]
async fn records_validation_encryption_and_schema() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;
    let (_, _) = server
        .request("POST", "/dbs", Some(&root), Some(json!({"name": "app"})))
        .await;
    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables",
            Some(&root),
            Some(json!({"name": "users", "schema": schema_fixture()})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, record) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records",
            Some(&root),
            Some(json!({
                "tenant": "t1", "email": "ana@x.com", "cpf": "111",
                "nome": "Ana", "senha": "segredo", "idade": 30
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(record["ativo"], json!(true));
    assert!(record["criado"].is_string());
    assert_eq!(record["senha"], "segredo");
    let record_id = record["_id"].as_str().unwrap().to_string();

    let (status, body) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records",
            Some(&root),
            Some(json!({"email": "sem-arroba", "idade": 200, "extra": true})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let fields = body["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|error| error["field"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(fields.contains(&"tenant"));
    assert!(fields.contains(&"email"));
    assert!(fields.contains(&"idade"));
    assert!(fields.contains(&"extra"));

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records",
            Some(&root),
            Some(json!({"tenant": "t1", "email": "ana@x.com", "cpf": "999", "nome": "B"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records",
            Some(&root),
            Some(json!({"tenant": "t2", "email": "bia@x.com", "cpf": "111", "nome": "Bia"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let on_disk = server.disk("dbs/app/users.msgpack");
    assert!(on_disk.contains("enc:v1:"), "campo deveria estar cifrado no disco");
    assert!(!on_disk.contains("segredo"), "plaintext nao deveria aparecer no disco");

    let (status, body) = server
        .request("GET", "/dbs/app/tables/users/records", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["records"][0]["senha"], "segredo");

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records/validate",
            Some(&root),
            Some(json!({"tenant": "t3", "email": "c@x.com", "nome": "C"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = server
        .request("GET", "/dbs/app/tables/users/records", Some(&root), None)
        .await;
    assert_eq!(body["count"], json!(1), "validate nao deve persistir");

    let (status, body) = server
        .request("GET", "/dbs/app/tables/users/schema", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["unique"], json!(["cpf", ["tenant", "email"]]));

    let (status, body) = server
        .request(
            "PUT",
            "/dbs/app/tables/users/schema",
            Some(&root),
            Some(json!({
                "strict": false,
                "fields": {
                    "tenant": {"type": "string", "required": true},
                    "email": {"type": "string", "required": true},
                    "obrigatorio": {"type": "string", "required": true}
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["records"].as_array().is_some_and(|items| !items.is_empty()));
    let (status, _) = server
        .request("GET", "/dbs/app/tables/users/schema", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = server
        .request(
            "PUT",
            "/dbs/app/tables/users/schema?force=true",
            Some(&root),
            Some(json!({
                "strict": false,
                "fields": {
                    "tenant": {"type": "string", "required": true},
                    "email": {"type": "string", "required": true},
                    "obrigatorio": {"type": "string", "required": true}
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = server
        .request(
            "PUT",
            "/dbs/app/tables/users/schema",
            Some(&root),
            Some(json!({
                "fields": {
                    "tenant": {"type": "string"},
                    "email": {"type": "string"},
                    "senha": {"type": "string", "encrypted": true}
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let on_disk = server.disk("dbs/app/users.msgpack");
    assert!(on_disk.contains("enc:v1:"));

    let (status, _) = server
        .request("DELETE", "/dbs/app/tables/users/schema", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let on_disk = server.disk("dbs/app/users.msgpack");
    assert!(!on_disk.contains("enc:v1:"), "delete schema deve decifrar");
    assert!(on_disk.contains("segredo"));

    let (status, _) = server
        .request(
            "GET",
            &format!("/dbs/app/tables/users/records/{record_id}"),
            Some(&root),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = server
        .request("GET", "/dbs/app/tables/users/records/inexistente", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = server.request("GET", "/dbs/nope", Some(&root), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = server
        .request("GET", "/dbs/app/tables/nope/records", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = server
        .request("GET", "/dbs/..%2F..%2Fetc", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn query_filters_sorting_pagination() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;
    let (_, _) = server
        .request("POST", "/dbs", Some(&root), Some(json!({"name": "q"})))
        .await;
    let (_, _) = server
        .request(
            "POST",
            "/dbs/q/tables",
            Some(&root),
            Some(json!({"name": "users"})),
        )
        .await;

    for record in [
        json!({"tenant": "t1", "nome": "Ana", "idade": 30, "tags": ["dev"], "endereco": {"cidade": "SP"}}),
        json!({"tenant": "t2", "nome": "Bia", "idade": 25, "tags": ["vip"], "endereco": {"cidade": "RJ"}}),
        json!({"tenant": "t3", "nome": "Caio", "idade": 40, "tags": ["vip", "dev"], "endereco": {"cidade": "SP"}}),
    ] {
        let (status, _) = server
            .request(
                "POST",
                "/dbs/q/tables/users/records",
                Some(&root),
                Some(record),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let (status, body) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({"filter": {"idade": {"$gte": 30}}, "sort": ["-idade"], "fields": ["nome", "idade"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(2));
    assert_eq!(body["records"][0]["nome"], "Caio");
    assert!(body["records"][0].get("tenant").is_none());

    let (status, body) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({
                "filter": {"$or": [{"tags": {"$contains": "vip"}}, {"endereco.cidade": "SP"}]},
                "sort": [{"field": "idade", "dir": "asc"}],
                "limit": 2,
                "fields": ["nome"]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(3));
    assert_eq!(body["count"], json!(2));
    assert_eq!(body["records"][0]["nome"], "Bia");

    let (status, body) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({"filter": {"nome": {"$regex": "^[AB]"}}, "sort": ["nome"], "offset": 1, "fields": ["nome"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(2));
    assert_eq!(body["records"][0]["nome"], "Bia");

    let (status, _) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({"filter": {"idade": {"$foo": 1}}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({"desconhecido": true})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn encrypted_field_is_queryable() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;
    let (_, _) = server
        .request("POST", "/dbs", Some(&root), Some(json!({"name": "v"})))
        .await;
    let (_, _) = server
        .request(
            "POST",
            "/dbs/v/tables",
            Some(&root),
            Some(json!({
                "name": "cofre",
                "schema": {"fields": {"segredo": {"type": "string", "encrypted": true}}}
            })),
        )
        .await;
    for (segredo, nome) in [("alpha", "um"), ("beta", "dois")] {
        let (status, _) = server
            .request(
                "POST",
                "/dbs/v/tables/cofre/records",
                Some(&root),
                Some(json!({"segredo": segredo, "nome": nome})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, body) = server
        .request(
            "POST",
            "/dbs/v/tables/cofre/records/query",
            Some(&root),
            Some(json!({"filter": {"segredo": "beta"}, "fields": ["nome"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(1));
    assert_eq!(body["records"][0]["nome"], "dois");
}

#[tokio::test]
async fn api_keys_lifecycle() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;

    let (status, bob) = server
        .request(
            "POST",
            "/users",
            Some(&root),
            Some(json!({"username": "bob", "password": "bob12345"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let bob_id = bob["id"].as_str().unwrap().to_string();

    let (status, created) = server
        .request(
            "POST",
            &format!("/users/{bob_id}/api-keys"),
            Some(&root),
            Some(json!({"name": "ci"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let ci_key = created["key"].as_str().unwrap().to_string();
    let ci_id = created["api_key"]["id"].as_str().unwrap().to_string();
    assert!(
        created["api_key"]["prefix"]
            .as_str()
            .unwrap()
            .starts_with("luna_")
    );

    let (status, me) = server.request("GET", "/me", Some(&ci_key), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["username"], "bob");

    let bob_login = server.login("bob", "bob12345").await;
    let (status, _) = server.request("GET", "/me", Some(&ci_key), None).await;
    assert_eq!(status, StatusCode::OK, "login nao deve revogar outras chaves");
    let (status, keys) = server
        .request("GET", &format!("/users/{bob_id}/api-keys"), Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(keys.as_array().unwrap().len(), 2);
    let (status, _) = server.request("GET", "/me", Some(&bob_login), None).await;
    assert_eq!(status, StatusCode::OK);

    let (_, carol) = server
        .request(
            "POST",
            "/users",
            Some(&root),
            Some(json!({"username": "carol", "password": "carol123"})),
        )
        .await;
    let carol_id = carol["id"].as_str().unwrap();
    let (status, _) = server
        .request(
            "GET",
            &format!("/users/{carol_id}/api-keys"),
            Some(&ci_key),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "bob nao pode ver chaves da carol");

    let (status, self_created) = server
        .request(
            "POST",
            "/auth/api-keys",
            Some(&ci_key),
            Some(json!({"name": "pessoal"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let self_key = self_created["key"].as_str().unwrap().to_string();
    let (status, _) = server.request("GET", "/me", Some(&self_key), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, keys) = server.request("GET", "/auth/api-keys", Some(&ci_key), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(keys.as_array().unwrap().len() >= 3);

    let prefix = created["api_key"]["prefix"]
        .as_str()
        .unwrap()
        .trim_end_matches("...")
        .to_string();
    let (status, revoked) = server
        .request(
            "DELETE",
            &format!("/users/{bob_id}/api-keys/{prefix}"),
            Some(&root),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(revoked["revoked"], json!(true));
    let (status, _) = server.request("GET", "/me", Some(&ci_key), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = server
        .request(
            "DELETE",
            &format!("/users/{bob_id}/api-keys/{ci_id}"),
            Some(&root),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "revogar de novo deve conflitar");
}

#[tokio::test]
async fn msgpack_legacy_and_migrate() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;
    let (_, _) = server
        .request("POST", "/dbs", Some(&root), Some(json!({"name": "m"})))
        .await;

    let (status, _) = server
        .request(
            "POST",
            "/dbs/m/tables",
            Some(&root),
            Some(json!({"name": "novo"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(server.dir.path().join("dbs/m/novo.msgpack").exists());
    let raw = server.disk_bytes("dbs/m/novo.msgpack");
    assert!(!raw.is_empty() && raw[0] != b'{', "msgpack nao deve ser JSON");

    let legacy = json!({"name": "legado", "records": {"abc": {"campo": "valor"}}});
    std::fs::write(
        server.dir.path().join("dbs/m/legado.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();

    let (status, tables) = server.request("GET", "/dbs/m/tables", Some(&root), None).await;
    assert_eq!(status, StatusCode::OK);
    let names = tables["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"legado") && names.contains(&"novo"));
    assert_eq!(names.len(), 2, "sem duplicatas por extensao");

    let (status, body) = server
        .request("GET", "/dbs/m/tables/legado/records", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["records"][0]["campo"], "valor");

    let (status, _) = server
        .request(
            "POST",
            "/dbs/m/tables/legado/records",
            Some(&root),
            Some(json!({"extra": true})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(server.dir.path().join("dbs/m/legado.msgpack").exists());
    assert!(!server.dir.path().join("dbs/m/legado.json").exists());

    let report = lunadb::engine::migrate_tables(&server.paths, lunadb::storage::Format::Json)
        .await
        .unwrap();
    assert_eq!(report.converted.len(), 2);
    assert!(server.dir.path().join("dbs/m/novo.json").exists());
    assert!(!server.dir.path().join("dbs/m/novo.msgpack").exists());
    let (status, _) = server
        .request("GET", "/dbs/m/tables/novo/records", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn schema_patch_migration() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;
    let (_, _) = server
        .request("POST", "/dbs", Some(&root), Some(json!({"name": "p"})))
        .await;

    let (status, _) = server
        .request(
            "POST",
            "/dbs/p/tables",
            Some(&root),
            Some(json!({"name": "t"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = server
        .request(
            "POST",
            "/dbs/p/tables/t/records",
            Some(&root),
            Some(json!({"email": "a@x.com", "n": 1})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, schema) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema",
            Some(&root),
            Some(json!({"add": {"papel": {"type": "string", "default": "user"}}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(schema["fields"]["papel"]["default"], json!("user"));
    let (_, body) = server
        .request("GET", "/dbs/p/tables/t/records", Some(&root), None)
        .await;
    assert_eq!(
        body["records"][0]["papel"],
        json!("user"),
        "migracao deve aplicar default nos registros existentes"
    );

    let (status, _) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema",
            Some(&root),
            Some(json!({"update": {"papel": {"required": true}}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    for (body, label) in [
        (json!({"add": {"papel": {"type": "string"}}}), "add existente"),
        (json!({"remove": ["nao_existe"]}), "remove inexistente"),
        (json!({"update": {"nao_existe": {"required": true}}}), "update inexistente"),
    ] {
        let (status, _) = server
            .request("PATCH", "/dbs/p/tables/t/schema", Some(&root), Some(body))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}");
    }
    let (status, _) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema",
            Some(&root),
            Some(json!({"desconhecido": true})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/p/tables/t/records",
            Some(&root),
            Some(json!({"email": "a@x.com", "n": 2, "papel": "admin"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema",
            Some(&root),
            Some(json!({"add": {"email": {"type": "string"}, "n": {"type": "int"}}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema",
            Some(&root),
            Some(json!({"unique_add": ["email"]})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "emails duplicados devem rejeitar");
    assert!(body["records"].as_array().is_some_and(|items| !items.is_empty()));
    let (status, schema) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema?force=true",
            Some(&root),
            Some(json!({"unique_add": ["email"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(schema["unique"], json!(["email"]));

    let (status, _) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema",
            Some(&root),
            Some(json!({"unique_add": ["email"]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "unique duplicada no patch");
    let (status, schema) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema?force=true",
            Some(&root),
            Some(json!({"unique_remove": ["email"], "remove": ["papel"], "strict": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(schema["unique"], json!([]));
    assert!(schema["fields"].get("papel").is_none());
    assert_eq!(schema["strict"], json!(true));

    let (status, _) = server
        .request(
            "PATCH",
            "/dbs/p/tables/nao_existe/schema",
            Some(&root),
            Some(json!({"add": {"x": {"type": "string"}}})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (_, _) = server
        .request(
            "POST",
            "/users",
            Some(&root),
            Some(json!({
                "username": "carol",
                "password": "carol123",
                "grants": [{"db": "p", "level": "read"}]
            })),
        )
        .await;
    let carol_key = server.login("carol", "carol123").await;
    let (status, _) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema",
            Some(&carol_key),
            Some(json!({"add": {"outro": {"type": "string"}}})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "read nao pode migrar schema");
}

