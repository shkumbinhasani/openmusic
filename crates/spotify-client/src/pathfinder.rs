use anyhow::{Result, bail};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::api::{Api, Body, HttpStatus};

pub const URL: &str = "https://api-partner.spotify.com/pathfinder/v2/query";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Query,
    Mutation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Operation {
    pub name: &'static str,
    pub hash: &'static str,
    pub kind: Kind,
}

impl Operation {
    pub const fn query(name: &'static str, hash: &'static str) -> Self {
        Self {
            name,
            hash,
            kind: Kind::Query,
        }
    }

    pub const fn mutation(name: &'static str, hash: &'static str) -> Self {
        Self {
            name,
            hash,
            kind: Kind::Mutation,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PathfinderError {
    StaleHash(&'static str),
    GraphQl(Vec<String>),
    NoData,
    Forbidden,
}

impl std::fmt::Display for PathfinderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleHash(name) => write!(
                formatter,
                "Spotify changed how {name} works. Update OpenMusic to use it again."
            ),
            Self::GraphQl(messages) if messages.is_empty() => {
                write!(formatter, "Spotify could not answer the request.")
            }
            Self::GraphQl(messages) => {
                write!(
                    formatter,
                    "Spotify could not answer: {}",
                    messages.join("; ")
                )
            }
            Self::NoData => write!(formatter, "Spotify sent an empty answer."),
            Self::Forbidden => write!(formatter, "Spotify refused the request."),
        }
    }
}

impl std::error::Error for PathfinderError {}

pub fn body(operation: Operation, variables: Value) -> Value {
    json!({
        "variables": variables,
        "operationName": operation.name,
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": operation.hash,
            }
        }
    })
}

pub fn interpret(operation: Operation, status: StatusCode, bytes: &[u8]) -> Result<Value> {
    match status {
        StatusCode::PRECONDITION_FAILED => bail!(PathfinderError::StaleHash(operation.name)),
        StatusCode::FORBIDDEN => bail!(PathfinderError::Forbidden),
        status if !status.is_success() => bail!(HttpStatus(status)),
        _ => {}
    }
    let mut reply: Value = serde_json::from_slice(bytes).map_err(|_| PathfinderError::NoData)?;
    let data = reply
        .get_mut("data")
        .map(Value::take)
        .unwrap_or(Value::Null);
    let messages: Vec<String> = reply["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|error| {
            error["message"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| error.to_string())
        })
        .collect();
    if operation.kind == Kind::Mutation {
        if !messages.is_empty() {
            bail!(PathfinderError::GraphQl(messages));
        }
        if let Some(refusal) = refusal(&data) {
            bail!(PathfinderError::GraphQl(vec![refusal]));
        }
    }
    if !data.is_null() {
        return Ok(data);
    }
    if reply["errors"].is_array() {
        bail!(PathfinderError::GraphQl(messages));
    }
    bail!(PathfinderError::NoData)
}

fn refusal(data: &Value) -> Option<String> {
    data.as_object()?.values().find_map(|reply| {
        let kind = reply["__typename"].as_str()?;
        kind.ends_with("Error").then(|| {
            reply["message"]
                .as_str()
                .filter(|message| !message.is_empty())
                .unwrap_or(kind)
                .to_owned()
        })
    })
}

async fn run(api: &Api, operation: Operation, variables: Value) -> Result<Value> {
    let headers = api.desktop_headers(URL).await;
    let (status, bytes) = api
        .send_with(
            Method::POST,
            URL,
            Body::Json(body(operation, variables)),
            "application/json",
            &headers,
        )
        .await?;
    interpret(operation, status, &bytes)
}

pub async fn query(api: &Api, operation: Operation, variables: Value) -> Result<Value> {
    if operation.kind != Kind::Query {
        bail!("{} is a mutation; use pathfinder::mutate.", operation.name);
    }
    run(api, operation, variables).await
}

pub async fn mutate(api: &Api, operation: Operation, variables: Value) -> Result<Value> {
    if operation.kind != Kind::Mutation {
        bail!("{} is a query; use pathfinder::query.", operation.name);
    }
    run(api, operation, variables).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH: Operation = Operation::query(
        "searchDesktop",
        "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03",
    );
    const ADD: Operation = Operation::mutation(
        "addToLibrary",
        "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
    );

    fn error(result: Result<Value>) -> PathfinderError {
        result
            .unwrap_err()
            .downcast::<PathfinderError>()
            .expect("a pathfinder error")
    }

    #[test]
    fn builds_the_persisted_query_body() {
        let value = body(SEARCH, json!({"searchTerm": "radiohead", "limit": 10}));
        assert_eq!(
            value,
            json!({
                "variables": {"searchTerm": "radiohead", "limit": 10},
                "operationName": "searchDesktop",
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03"
                    }
                }
            })
        );
    }

    #[tokio::test]
    async fn sends_the_desktop_headers() {
        let api = Api::new(
            reqwest::Client::new(),
            "https://spclient.example",
            "access".into(),
        );
        api.set_language("de".into());
        let headers = api.desktop_headers("https://example.com/").await;
        assert_eq!(
            headers,
            vec![
                ("app-platform", crate::api::APP_PLATFORM.to_owned()),
                ("spotify-app-version", crate::api::APP_VERSION.to_owned()),
            ]
        );
        let request = api
            .prepare(
                Method::POST,
                URL,
                Body::Json(body(SEARCH, json!({}))),
                "application/json",
                &headers,
            )
            .build()
            .unwrap();
        assert_eq!(request.method(), Method::POST);
        assert_eq!(request.url().as_str(), URL);
        let headers = request.headers();
        assert_eq!(headers["authorization"], "Bearer access");
        assert_eq!(headers["accept"], "application/json");
        assert_eq!(headers["accept-language"], "de");
        assert_eq!(headers["content-type"], "application/json");
        assert_eq!(headers["app-platform"], crate::api::APP_PLATFORM);
        assert_eq!(headers["spotify-app-version"], "1.3.10.2586.g81a118a22460");
        let sent: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(sent["operationName"], "searchDesktop");
    }

    #[test]
    fn returns_the_data_object() {
        let data = interpret(
            SEARCH,
            StatusCode::OK,
            br#"{"data":{"searchV2":{"query":"x"}},"extensions":{"requestIds":{}}}"#,
        )
        .unwrap();
        assert_eq!(data, json!({"searchV2": {"query": "x"}}));
    }

    #[test]
    fn fails_a_write_that_comes_back_with_errors() {
        let result = interpret(
            ADD,
            StatusCode::OK,
            br#"{"data":{"addLibraryItems":null},"errors":[{"message":"not allowed"}]}"#,
        );
        assert_eq!(
            error(result),
            PathfinderError::GraphQl(vec!["not allowed".into()])
        );
        let result = interpret(
            ADD,
            StatusCode::OK,
            br#"{"data":{"addItemsToPlaylist":{"__typename":"GenericError","message":"Too many"}}}"#,
        );
        assert_eq!(
            error(result),
            PathfinderError::GraphQl(vec!["Too many".into()])
        );
        let result = interpret(
            ADD,
            StatusCode::OK,
            br#"{"data":{"removeItems":{"__typename":"NotFoundError"}}}"#,
        );
        assert_eq!(
            error(result),
            PathfinderError::GraphQl(vec!["NotFoundError".into()])
        );
        let data = interpret(
            ADD,
            StatusCode::OK,
            br#"{"data":{"addLibraryItems":{"__typename":"AddLibraryItemsResponse"}},"errors":[]}"#,
        )
        .unwrap();
        assert_eq!(
            data,
            json!({"addLibraryItems": {"__typename": "AddLibraryItemsResponse"}})
        );
        assert!(interpret(ADD, StatusCode::OK, br#"{"data":{"x":null}}"#).is_ok());
    }

    #[test]
    fn keeps_data_that_comes_with_errors() {
        let data = interpret(
            SEARCH,
            StatusCode::OK,
            br#"{"data":{"lookup":[]},"errors":[{"message":"partial"}]}"#,
        )
        .unwrap();
        assert_eq!(data, json!({"lookup": []}));
    }

    #[test]
    fn maps_a_stale_hash() {
        let result = interpret(
            SEARCH,
            StatusCode::PRECONDITION_FAILED,
            b"Invalid query hash",
        );
        assert_eq!(error(result), PathfinderError::StaleHash("searchDesktop"));
    }

    #[test]
    fn maps_graphql_errors_without_data() {
        let result = interpret(
            SEARCH,
            StatusCode::OK,
            br#"{"data":null,"errors":[{"message":"Variable searchTerm is required"},{"path":["x"]}]}"#,
        );
        assert_eq!(
            error(result),
            PathfinderError::GraphQl(vec![
                "Variable searchTerm is required".into(),
                r#"{"path":["x"]}"#.into()
            ])
        );
    }

    #[test]
    fn maps_missing_data() {
        assert_eq!(
            error(interpret(SEARCH, StatusCode::OK, br#"{"data":null}"#)),
            PathfinderError::NoData
        );
        assert_eq!(
            error(interpret(SEARCH, StatusCode::OK, b"")),
            PathfinderError::NoData
        );
    }

    #[test]
    fn maps_forbidden() {
        let result = interpret(
            SEARCH,
            StatusCode::FORBIDDEN,
            br#"{"message":"Client/request not allowed"}"#,
        );
        assert_eq!(error(result), PathfinderError::Forbidden);
    }

    #[test]
    fn leaves_unauthorized_as_a_plain_status() {
        let error = interpret(SEARCH, StatusCode::UNAUTHORIZED, b"").unwrap_err();
        assert!(error.downcast_ref::<PathfinderError>().is_none());
        assert_eq!(
            error.downcast_ref::<HttpStatus>().map(|status| status.0),
            Some(StatusCode::UNAUTHORIZED)
        );
    }

    #[tokio::test]
    async fn refuses_the_wrong_kind() {
        let api = Api::new(
            reqwest::Client::new(),
            "https://spclient.example",
            String::new(),
        );
        let refused = query(&api, ADD, json!({})).await.unwrap_err();
        assert!(refused.to_string().contains("addToLibrary is a mutation"));
        let refused = mutate(&api, SEARCH, json!({})).await.unwrap_err();
        assert!(refused.to_string().contains("searchDesktop is a query"));
    }
}
