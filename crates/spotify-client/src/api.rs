use std::sync::{Arc, RwLock};

use anyhow::{Context, Result, bail};
use reqwest::{
    Method, RequestBuilder, StatusCode,
    header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, HeaderMap},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::clienttoken::{self, ClientToken};

pub const APP_PLATFORM: &str = if cfg!(target_arch = "aarch64") {
    "OSX_ARM64"
} else {
    "OSX"
};
pub const APP_VERSION: &str = "1.3.10.2586.g81a118a22460";
const DEFAULT_LANGUAGE: &str = "en";
const PROTOBUF: &str = "application/x-protobuf";
const CLIENT_TOKEN: &str = "client-token";

#[derive(Clone)]
pub struct Api {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    spclient: String,
    token: RwLock<String>,
    language: RwLock<String>,
    client_token: ClientToken,
}

#[derive(Debug)]
pub struct HttpStatus(pub StatusCode);

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Spotify answered {}", self.0)
    }
}

impl std::error::Error for HttpStatus {}

#[derive(Debug, PartialEq, Eq)]
pub struct GrpcStatus {
    pub code: u32,
    pub message: String,
}

impl std::fmt::Display for GrpcStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Spotify answered gRPC status {}", self.code)?;
        if !self.message.is_empty() {
            write!(formatter, " ({})", self.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for GrpcStatus {}

fn grpc_status(headers: &HeaderMap) -> Option<GrpcStatus> {
    let code = headers
        .get("grpc-status")?
        .to_str()
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|code| *code != 0)?;
    let message = headers
        .get("grpc-message")
        .and_then(|message| message.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    Some(GrpcStatus { code, message })
}

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
        Self::new(http, &format!("https://{spclient}"), token)
    }

    pub fn new(http: reqwest::Client, spclient: &str, token: String) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                spclient: spclient.trim_end_matches('/').to_owned(),
                token: RwLock::new(token),
                language: RwLock::new(DEFAULT_LANGUAGE.to_owned()),
                client_token: ClientToken::new(clienttoken::default_path()),
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

    pub fn language(&self) -> String {
        self.inner
            .language
            .read()
            .map(|language| language.clone())
            .unwrap_or_default()
    }

    pub fn set_language(&self, language: String) {
        if let Ok(mut current) = self.inner.language.write() {
            *current = language;
        }
    }

    pub async fn desktop_headers(&self, url: &str) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            ("app-platform", APP_PLATFORM.to_owned()),
            ("spotify-app-version", APP_VERSION.to_owned()),
        ];
        if clienttoken::allowed(url)
            && let Some(token) = self.inner.client_token.get(&self.inner.http).await
        {
            headers.push((CLIENT_TOKEN, token));
        }
        headers
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with("https://") {
            path.to_owned()
        } else {
            format!("{}/{}", self.inner.spclient, path.trim_start_matches('/'))
        }
    }

    pub(crate) fn prepare(
        &self,
        method: Method,
        path: &str,
        body: Body,
        accept: &str,
        headers: &[(&'static str, String)],
    ) -> RequestBuilder {
        let url = self.url(path);
        let token_allowed = clienttoken::allowed(&url);
        let mut request = self
            .inner
            .http
            .request(method, url)
            .bearer_auth(self.token())
            .header(ACCEPT, accept);
        let language = self.language();
        if !language.is_empty() {
            request = request.header(ACCEPT_LANGUAGE, language);
        }
        for (name, value) in headers {
            if *name != CLIENT_TOKEN || token_allowed {
                request = request.header(*name, value);
            }
        }
        match body {
            Body::Empty => request,
            Body::Json(value) => request.json(&value),
            Body::Bytes(bytes, content_type) => {
                request.header(CONTENT_TYPE, content_type).body(bytes)
            }
        }
    }

    pub async fn send(
        &self,
        method: Method,
        path: &str,
        body: Body,
        accept: &str,
    ) -> Result<(StatusCode, Vec<u8>)> {
        self.send_with(method, path, body, accept, &[]).await
    }

    pub async fn send_with(
        &self,
        method: Method,
        path: &str,
        body: Body,
        accept: &str,
        headers: &[(&'static str, String)],
    ) -> Result<(StatusCode, Vec<u8>)> {
        let response = self
            .prepare(method, path, body, accept, headers)
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
        self.request(method, path, Body::Bytes(body, PROTOBUF), PROTOBUF)
            .await
    }

    pub async fn grpc(&self, path: &str, body: Vec<u8>) -> Result<Vec<u8>> {
        let response = self
            .prepare(
                Method::POST,
                path,
                Body::Bytes(body, PROTOBUF),
                PROTOBUF,
                &[],
            )
            .send()
            .await
            .with_context(|| format!("Could not reach Spotify ({path})."))?;
        let status = response.status();
        let grpc = grpc_status(response.headers());
        let bytes = response.bytes().await.unwrap_or_default().to_vec();
        if !status.is_success() {
            bail!(HttpStatus(status));
        }
        if let Some(grpc) = grpc {
            bail!(grpc);
        }
        Ok(bytes)
    }
}

pub enum Body {
    Empty,
    Json(Value),
    Bytes(Vec<u8>, &'static str),
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    fn api() -> Api {
        Api::new(
            reqwest::Client::new(),
            "https://spclient.wg.spotify.com/",
            "access".into(),
        )
    }

    #[test]
    fn sends_accept_language_on_every_request() {
        let api = api();
        let request = api
            .prepare(
                Method::GET,
                "/context-resolve/v1/x",
                Body::Empty,
                "application/json",
                &[],
            )
            .build()
            .unwrap();
        assert_eq!(
            request.url().as_str(),
            "https://spclient.wg.spotify.com/context-resolve/v1/x"
        );
        assert_eq!(request.headers()[ACCEPT_LANGUAGE], "en");
        assert_eq!(request.headers()["authorization"], "Bearer access");
        api.set_language("de-DE".into());
        let request = api
            .prepare(Method::GET, "/x", Body::Empty, "application/json", &[])
            .build()
            .unwrap();
        assert_eq!(request.headers()[ACCEPT_LANGUAGE], "de-DE");
    }

    #[test]
    fn keeps_the_client_token_on_spotify_hosts() {
        let api = api();
        let headers = [(CLIENT_TOKEN, "secret".to_owned())];
        let spotify = api
            .prepare(Method::GET, "/x", Body::Empty, "*/*", &headers)
            .build()
            .unwrap();
        assert_eq!(spotify.headers()[CLIENT_TOKEN], "secret");
        let elsewhere = api
            .prepare(
                Method::GET,
                "https://i.scdn.co/image/x",
                Body::Empty,
                "*/*",
                &headers,
            )
            .build()
            .unwrap();
        assert!(elsewhere.headers().get(CLIENT_TOKEN).is_none());
    }

    #[test]
    fn reads_grpc_status_headers() {
        let mut headers = HeaderMap::new();
        assert_eq!(grpc_status(&headers), None);
        headers.insert("grpc-status", HeaderValue::from_static("0"));
        assert_eq!(grpc_status(&headers), None);
        headers.insert("grpc-status", HeaderValue::from_static("7"));
        headers.insert("grpc-message", HeaderValue::from_static("denied"));
        assert_eq!(
            grpc_status(&headers),
            Some(GrpcStatus {
                code: 7,
                message: "denied".into()
            })
        );
    }
}
