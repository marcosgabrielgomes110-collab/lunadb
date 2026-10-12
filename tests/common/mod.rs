#![allow(dead_code)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use lunadb::bootstrap;
use lunadb::config::{Config, Paths};
use lunadb::crypto::Keyring;
use lunadb::state::AppState;
use serde_json::Value;
use tower::ServiceExt;

pub struct TestServer {
    pub router: Router,
    pub dir: tempfile::TempDir,
    pub password: String,
    pub paths: Paths,
}

impl TestServer {
    pub async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("create tempdir");
        let mut config = Config::default();
        config.storage.path = dir.path().to_string_lossy().to_string();
        config.root.password = "test-secret".to_string();
        let paths = Paths::from_config(&config);
        let config_path = dir.path().join("config.toml");
        bootstrap::init(&paths, &mut config, &config_path)
            .await
            .expect("bootstrap");
        let keyring = Arc::new(Keyring::from_master([7u8; 32]));
        let state = AppState::new(paths.clone(), keyring, config, config_path);
        Self {
            router: lunadb::api::router(state),
            dir,
            password: "test-secret".to_string(),
            paths,
        }
    }

    pub async fn request(
        &self,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let body = match body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&value).unwrap())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()))
        };
        (status, value)
    }

    pub async fn login(&self, username: &str, password: &str) -> String {
        let (status, body) = self
            .request(
                "POST",
                "/auth/login",
                None,
                Some(serde_json::json!({"username": username, "password": password})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "login failed: {body}");
        body["api_key"].as_str().unwrap().to_string()
    }

    pub async fn login_root(&self) -> String {
        let password = self.password.clone();
        self.login("root", &password).await
    }

    pub fn disk_bytes(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.dir.path().join(relative)).unwrap()
    }

    pub fn disk(&self, relative: &str) -> String {
        String::from_utf8_lossy(&self.disk_bytes(relative)).into_owned()
    }

    pub fn disk_json(&self, relative: &str) -> Value {
        serde_json::from_str(&self.disk(relative)).unwrap()
    }
}
