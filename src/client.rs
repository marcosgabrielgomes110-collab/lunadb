use anyhow::{Context, anyhow};
use serde_json::Value;

pub struct Client {
    base_url: String,
    api_key: Option<String>,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key,
            agent,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn set_api_key(&mut self, api_key: String) {
        self.api_key = Some(api_key);
    }

    pub fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.finish(self.authorize(self.agent.get(self.url(path))).call())
    }

    pub fn delete(&self, path: &str) -> anyhow::Result<Value> {
        self.finish(self.authorize(self.agent.delete(self.url(path))).call())
    }

    pub fn post(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let request = self
            .authorize(self.agent.post(self.url(path)))
            .header("content-type", "application/json");
        self.finish(request.send(payload(&body)))
    }

    pub fn put(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let request = self
            .authorize(self.agent.put(self.url(path)))
            .header("content-type", "application/json");
        self.finish(request.send(payload(&body)))
    }

    pub fn patch(&self, path: &str, body: Value) -> anyhow::Result<Value> {
        let request = self
            .authorize(self.agent.patch(self.url(path)))
            .header("content-type", "application/json");
        self.finish(request.send(payload(&body)))
    }

    pub fn login(&mut self, username: &str, password: &str) -> anyhow::Result<Value> {
        let value = self.post(
            "/auth/login",
            serde_json::json!({ "username": username, "password": password }),
        )?;
        let key = value
            .get("api_key")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("resposta de login sem api_key"))?;
        self.set_api_key(key.to_string());
        Ok(value)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn authorize<B>(
        &self,
        mut request: ureq::RequestBuilder<B>,
    ) -> ureq::RequestBuilder<B> {
        if let Some(key) = &self.api_key {
            request = request.header("authorization", format!("Bearer {key}"));
        }
        request
    }

    fn finish(
        &self,
        response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> anyhow::Result<Value> {
        let mut response = response
            .map_err(|err| anyhow!("falha de conexao com {}: {err}", self.base_url))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .context("falha ao ler resposta")?;
        if (200..300).contains(&status) {
            if text.is_empty() {
                return Ok(Value::Null);
            }
            return Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)));
        }
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or(text);
        Err(anyhow!("HTTP {status}: {message}"))
    }
}

fn payload(body: &Value) -> String {
    serde_json::to_string(body).unwrap_or_else(|_| "null".to_string())
}
