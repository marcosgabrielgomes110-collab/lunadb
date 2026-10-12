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
            Some(json!({"username": "root", "password": "wrong"})),
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
                "password": "other12345"
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
            Some(json!({"title": "from bob"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/notes/records",
            Some(&carol),
            Some(json!({"title": "from carol"})),
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
            "name": {"type": "string", "required": true},
            "password": {"type": "string", "encrypted": true},
            "age": {"type": "int", "min": 0, "max": 150},
            "active": {"type": "bool", "default": true},
            "created": {"type": "datetime", "auto": "created_at"}
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
                "name": "Ana", "password": "secret", "age": 30
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(record["active"], json!(true));
    assert!(record["created"].is_string());
    assert_eq!(record["password"], "secret");
    let record_id = record["_id"].as_str().unwrap().to_string();

    let (status, body) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records",
            Some(&root),
            Some(json!({"email": "no-at-sign", "age": 200, "extra": true})),
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
    assert!(fields.contains(&"age"));
    assert!(fields.contains(&"extra"));

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records",
            Some(&root),
            Some(json!({"tenant": "t1", "email": "ana@x.com", "cpf": "999", "name": "B"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records",
            Some(&root),
            Some(json!({"tenant": "t2", "email": "bia@x.com", "cpf": "111", "name": "Bia"})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let on_disk = server.disk("dbs/app/users.msgpack");
    assert!(
        on_disk.contains("enc:v1:"),
        "field should be encrypted on disk"
    );
    assert!(
        !on_disk.contains("secret"),
        "plaintext should not appear on disk"
    );

    let (status, body) = server
        .request("GET", "/dbs/app/tables/users/records", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["records"][0]["password"], "secret");

    let (status, _) = server
        .request(
            "POST",
            "/dbs/app/tables/users/records/validate",
            Some(&root),
            Some(json!({"tenant": "t3", "email": "c@x.com", "name": "C"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = server
        .request("GET", "/dbs/app/tables/users/records", Some(&root), None)
        .await;
    assert_eq!(body["count"], json!(1), "validate should not persist");

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
                    "required": {"type": "string", "required": true}
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        body["records"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
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
                    "required": {"type": "string", "required": true}
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
                    "password": {"type": "string", "encrypted": true}
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
    assert!(!on_disk.contains("enc:v1:"), "delete schema should decrypt");
    assert!(on_disk.contains("secret"));

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
        .request(
            "GET",
            "/dbs/app/tables/users/records/missing",
            Some(&root),
            None,
        )
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
        json!({"tenant": "t1", "name": "Ana", "age": 30, "tags": ["dev"], "address": {"city": "SP"}}),
        json!({"tenant": "t2", "name": "Bia", "age": 25, "tags": ["vip"], "address": {"city": "RJ"}}),
        json!({"tenant": "t3", "name": "Caio", "age": 40, "tags": ["vip", "dev"], "address": {"city": "SP"}}),
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
            Some(json!({"filter": {"age": {"$gte": 30}}, "sort": ["-age"], "fields": ["name", "age"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(2));
    assert_eq!(body["records"][0]["name"], "Caio");
    assert!(body["records"][0].get("tenant").is_none());

    let (status, body) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({
                "filter": {"$or": [{"tags": {"$contains": "vip"}}, {"address.city": "SP"}]},
                "sort": [{"field": "age", "dir": "asc"}],
                "limit": 2,
                "fields": ["name"]
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(3));
    assert_eq!(body["count"], json!(2));
    assert_eq!(body["records"][0]["name"], "Bia");

    let (status, body) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({"filter": {"name": {"$regex": "^[AB]"}}, "sort": ["name"], "offset": 1, "fields": ["name"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(2));
    assert_eq!(body["records"][0]["name"], "Bia");

    let (status, _) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({"filter": {"age": {"$foo": 1}}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/q/tables/users/records/query",
            Some(&root),
            Some(json!({"unknown": true})),
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
                "name": "vault",
                "schema": {"fields": {"secret": {"type": "string", "encrypted": true}}}
            })),
        )
        .await;
    for (secret, name) in [("alpha", "one"), ("beta", "two")] {
        let (status, _) = server
            .request(
                "POST",
                "/dbs/v/tables/vault/records",
                Some(&root),
                Some(json!({"secret": secret, "name": name})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, body) = server
        .request(
            "POST",
            "/dbs/v/tables/vault/records/query",
            Some(&root),
            Some(json!({"filter": {"secret": "beta"}, "fields": ["name"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(1));
    assert_eq!(body["records"][0]["name"], "two");
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
    assert_eq!(status, StatusCode::OK, "login should not revoke other keys");
    let (status, keys) = server
        .request(
            "GET",
            &format!("/users/{bob_id}/api-keys"),
            Some(&root),
            None,
        )
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
    assert_eq!(status, StatusCode::FORBIDDEN, "bob cannot see carol's keys");

    let (status, self_created) = server
        .request(
            "POST",
            "/auth/api-keys",
            Some(&ci_key),
            Some(json!({"name": "personal"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    let self_key = self_created["key"].as_str().unwrap().to_string();
    let (status, _) = server.request("GET", "/me", Some(&self_key), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, keys) = server
        .request("GET", "/auth/api-keys", Some(&ci_key), None)
        .await;
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
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "revoking again should conflict"
    );
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
            Some(json!({"name": "new"})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(server.dir.path().join("dbs/m/new.msgpack").exists());
    let raw = server.disk_bytes("dbs/m/new.msgpack");
    assert!(
        !raw.is_empty() && raw[0] != b'{',
        "msgpack should not be JSON"
    );

    let legacy = json!({"name": "legacy", "records": {"abc": {"field": "value"}}});
    std::fs::write(
        server.dir.path().join("dbs/m/legacy.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();

    let (status, tables) = server
        .request("GET", "/dbs/m/tables", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let names = tables["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"legacy") && names.contains(&"new"));
    assert_eq!(names.len(), 2, "no duplicates by extension");

    let (status, body) = server
        .request("GET", "/dbs/m/tables/legacy/records", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["records"][0]["field"], "value");

    let (status, _) = server
        .request(
            "POST",
            "/dbs/m/tables/legacy/records",
            Some(&root),
            Some(json!({"extra": true})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(server.dir.path().join("dbs/m/legacy.msgpack").exists());
    assert!(!server.dir.path().join("dbs/m/legacy.json").exists());

    let report = lunadb::engine::migrate_tables(&server.paths, lunadb::storage::Format::Json)
        .await
        .unwrap();
    assert_eq!(report.converted.len(), 2);
    assert!(server.dir.path().join("dbs/m/new.json").exists());
    assert!(!server.dir.path().join("dbs/m/new.msgpack").exists());
    let (status, _) = server
        .request("GET", "/dbs/m/tables/new/records", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn record_patch_list_total_and_aggregate() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;
    let (_, _) = server
        .request("POST", "/dbs", Some(&root), Some(json!({"name": "agg"})))
        .await;
    let (_, _) = server
        .request(
            "POST",
            "/dbs/agg/tables",
            Some(&root),
            Some(json!({"name": "people"})),
        )
        .await;

    for record in [
        json!({"name": "Ana", "city": "SP", "age": 30, "address": {"city": "SP", "zip": "1"}, "tags": ["a"]}),
        json!({"name": "Caio", "city": "SP", "age": 40}),
        json!({"name": "Bia", "city": "RJ", "age": 25}),
    ] {
        let (status, _) = server
            .request(
                "POST",
                "/dbs/agg/tables/people/records",
                Some(&root),
                Some(record),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // list returns total and count
    let (status, body) = server
        .request("GET", "/dbs/agg/tables/people/records", Some(&root), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], json!(3));
    assert_eq!(body["count"], json!(3));

    // patch does shallow/nested merge and null removes the key
    let first_id = body["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["name"] == "Ana")
        .unwrap()["_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, patched) = server
        .request(
            "PATCH",
            &format!("/dbs/agg/tables/people/records/{first_id}"),
            Some(&root),
            Some(json!({"age": 31, "name": null, "address": {"city": "RJ"}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(patched.get("name").is_none(), "null removes the key");
    assert_eq!(patched["age"], json!(31));
    assert_eq!(patched["address"]["city"], json!("RJ"));
    assert_eq!(
        patched["address"]["zip"],
        json!("1"),
        "merge preserves siblings"
    );
    assert_eq!(patched["tags"], json!(["a"]));

    // aggregate by group
    let (status, body) = server
        .request(
            "POST",
            "/dbs/agg/tables/people/records/aggregate",
            Some(&root),
            Some(json!({
                "group_by": ["city"],
                "aggregates": {
                    "n": {"op": "count"},
                    "sum": {"op": "sum", "field": "age"},
                    "avg": {"op": "avg", "field": "age"},
                    "min": {"op": "min", "field": "age"},
                    "max": {"op": "max", "field": "age"}
                }
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], json!(2));
    assert_eq!(body["total"], json!(3));
    let sp = body["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|group| group["key"]["city"] == "SP")
        .unwrap();
    assert_eq!(sp["n"], json!(2));
    assert_eq!(sp["sum"], json!(71));
    assert_eq!(sp["avg"], json!(35.5));
    assert_eq!(sp["min"], json!(31));
    assert_eq!(sp["max"], json!(40));

    // aggregation without required field is invalid
    let (status, _) = server
        .request(
            "POST",
            "/dbs/agg/tables/people/records/aggregate",
            Some(&root),
            Some(json!({"aggregates": {"x": {"op": "sum"}}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn login_is_rate_limited() {
    let server = TestServer::spawn().await;
    for _ in 0..5 {
        let (status, _) = server
            .request(
                "POST",
                "/auth/login",
                None,
                Some(json!({"username": "root", "password": "wrong"})),
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _) = server
        .request(
            "POST",
            "/auth/login",
            None,
            Some(json!({"username": "root", "password": server.password})),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "should block after failures"
    );
}

#[tokio::test]
async fn password_change_flow() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;
    let (_, bob) = server
        .request(
            "POST",
            "/users",
            Some(&root),
            Some(json!({"username": "bob", "password": "bob12345"})),
        )
        .await;
    let bob_id = bob["id"].as_str().unwrap().to_string();
    let bob_key = server.login("bob", "bob12345").await;

    // short password rejected
    let (status, _) = server
        .request(
            "PUT",
            &format!("/users/{bob_id}/password"),
            Some(&bob_key),
            Some(json!({"password": "123", "current_password": "bob12345"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // self requires current_password
    let (status, _) = server
        .request(
            "PUT",
            &format!("/users/{bob_id}/password"),
            Some(&bob_key),
            Some(json!({"password": "new12345"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // wrong current_password
    let (status, _) = server
        .request(
            "PUT",
            &format!("/users/{bob_id}/password"),
            Some(&bob_key),
            Some(json!({"password": "new12345", "current_password": "wrong"})),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // valid change revokes bob's keys
    let (status, _) = server
        .request(
            "PUT",
            &format!("/users/{bob_id}/password"),
            Some(&bob_key),
            Some(json!({"password": "new12345", "current_password": "bob12345"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = server.request("GET", "/me", Some(&bob_key), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "old keys revoked");
    let _ = server.login("bob", "new12345").await;

    // admin changes another user's password without current_password
    let (status, _) = server
        .request(
            "PUT",
            &format!("/users/{bob_id}/password"),
            Some(&root),
            Some(json!({"password": "admin12345"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let _ = server.login("bob", "admin12345").await;

    // root changes its own password and config is synced/hashed
    let root_key = server.login_root().await;
    let me = server.request("GET", "/me", Some(&root_key), None).await.1;
    let root_id = me["id"].as_str().unwrap().to_string();
    let (status, _) = server
        .request(
            "PUT",
            &format!("/users/{root_id}/password"),
            Some(&root_key),
            Some(json!({"password": "rootnew123"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let saved = server.disk("config.toml");
    assert!(saved.contains("$argon2"));
    assert!(!saved.contains("rootnew123"));
    let _ = server.login("root", "rootnew123").await;
}

#[tokio::test]
async fn metrics_endpoint_exposes_counters() {
    let server = TestServer::spawn().await;
    let root = server.login_root().await;
    let (status, _) = server.request("GET", "/dbs", Some(&root), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = server.request("GET", "/metrics", None, None).await;
    assert_eq!(status, StatusCode::OK);
    let text = body.as_str().unwrap();
    assert!(text.contains("lunadb_requests_total"));
    assert!(text.contains("lunadb_responses_total{status_class=\"2xx\"}"));
}

#[tokio::test]
async fn root_password_is_hashed_in_config() {
    let server = TestServer::spawn().await;
    let saved = server.disk("config.toml");
    assert!(
        saved.contains("$argon2"),
        "config should store the argon2 hash: {saved}"
    );
    assert!(
        !saved.contains("test-secret"),
        "plaintext should not remain in the config"
    );
    let _ = server.login_root().await;
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
            Some(json!({"add": {"role": {"type": "string", "default": "user"}}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(schema["fields"]["role"]["default"], json!("user"));
    let (_, body) = server
        .request("GET", "/dbs/p/tables/t/records", Some(&root), None)
        .await;
    assert_eq!(
        body["records"][0]["role"],
        json!("user"),
        "migration should apply default to existing records"
    );

    let (status, _) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema",
            Some(&root),
            Some(json!({"update": {"role": {"required": true}}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    for (body, label) in [
        (json!({"add": {"role": {"type": "string"}}}), "existing add"),
        (json!({"remove": ["does_not_exist"]}), "nonexistent remove"),
        (
            json!({"update": {"does_not_exist": {"required": true}}}),
            "nonexistent update",
        ),
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
            Some(json!({"unknown": true})),
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (status, _) = server
        .request(
            "POST",
            "/dbs/p/tables/t/records",
            Some(&root),
            Some(json!({"email": "a@x.com", "n": 2, "role": "admin"})),
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
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "duplicate emails should be rejected"
    );
    assert!(
        body["records"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
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
    assert_eq!(status, StatusCode::BAD_REQUEST, "duplicate unique in patch");
    let (status, schema) = server
        .request(
            "PATCH",
            "/dbs/p/tables/t/schema?force=true",
            Some(&root),
            Some(json!({"unique_remove": ["email"], "remove": ["role"], "strict": true})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(schema["unique"], json!([]));
    assert!(schema["fields"].get("role").is_none());
    assert_eq!(schema["strict"], json!(true));

    let (status, _) = server
        .request(
            "PATCH",
            "/dbs/p/tables/does_not_exist/schema",
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
            Some(json!({"add": {"other": {"type": "string"}}})),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "read cannot migrate schema");
}
