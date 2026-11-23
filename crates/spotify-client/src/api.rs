use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};
use reqwest::{Method, StatusCode};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

#[derive(Clone)]
pub struct Api {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    spclient: String,
    token: RwLock<String>,
}

#[derive(Debug)]
pub struct HttpStatus(pub StatusCode);

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Spotify answered {}", self.0)
    }
}

impl std::error::Error for HttpStatus {}

pub async fn resolve(http: &reqwest::Client, kind: &str) -> Vec<String> {
    let resolved = async {
        http.get(format!("https://apresolve.spotify.com/?type={kind}"))
            .send()
            .await
            .ok()?
            .json::<Value>()
            .await
            .ok()
    }
    .await;
    resolved
        .and_then(|value| value.get(kind).cloned())
        .and_then(|hosts| serde_json::from_value::<Vec<String>>(hosts).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|host| host.trim_end_matches(":443").to_owned())
        .collect()
}

impl Api {
    pub async fn connect(http: reqwest::Client, token: String) -> Self {
        let spclient = resolve(&http, "spclient")
            .await
            .into_iter()
            .next()
            .unwrap_or_else(|| "spclient.wg.spotify.com".into());
        Self {
            inner: Arc::new(Inner {
                http,
                spclient: format!("https://{spclient}"),
                token: RwLock::new(token),
            }),
        }
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    pub fn base(&self) -> &str {
        &self.inner.spclient
    }

    pub fn token(&self) -> String {
        self.inner
            .token
            .read()
            .map(|token| token.clone())
            .unwrap_or_default()
    }

    pub fn set_token(&self, token: String) {
        if let Ok(mut current) = self.inner.token.write() {
            *current = token;
        }
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with("https://") {
            path.to_owned()
        } else {
            format!("{}/{}", self.inner.spclient, path.trim_start_matches('/'))
        }
    }

    pub async fn send(
        &self,
        method: Method,
        path: &str,
        body: Body,
        accept: &str,
    ) -> Result<(StatusCode, Vec<u8>)> {
        let mut request = self
            .inner
            .http
            .request(method, self.url(path))
            .bearer_auth(self.token())
            .header("accept", accept);
        request = match body {
            Body::Empty => request,
            Body::Json(value) => request.json(&value),
            Body::Bytes(bytes, content_type) => {
                request.header("content-type", content_type).body(bytes)
            }
        };
        let response = request
            .send()
            .await
            .with_context(|| format!("Could not reach Spotify ({path})."))?;
        let status = response.status();
        let bytes = response.bytes().await.unwrap_or_default().to_vec();
        Ok((status, bytes))
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Body,
        accept: &str,
    ) -> Result<Vec<u8>> {
        let (status, bytes) = self.send(method, path, body, accept).await?;
        if !status.is_success() {
            bail!(HttpStatus(status));
        }
        Ok(bytes)
    }

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let bytes = self
            .request(Method::GET, path, Body::Empty, "application/json")
            .await?;
        serde_json::from_slice(&bytes).with_context(|| format!("Unreadable reply from {path}."))
    }

    pub async fn json<T: DeserializeOwned, B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let bytes = self
            .request(
                method,
                path,
                Body::Json(serde_json::to_value(body)?),
                "application/json",
            )
            .await?;
        if bytes.is_empty() {
            return Ok(serde_json::from_value(Value::Null)?);
        }
        serde_json::from_slice(&bytes).with_context(|| format!("Unreadable reply from {path}."))
    }

    pub async fn protobuf(&self, method: Method, path: &str, body: Vec<u8>) -> Result<Vec<u8>> {
        self.request(
            method,
            path,
            Body::Bytes(body, "application/x-protobuf"),
            "application/x-protobuf",
        )
        .await
    }
}

pub enum Body {
    Empty,
    Json(Value),
    Bytes(Vec<u8>, &'static str),
}
