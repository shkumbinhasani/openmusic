use std::{collections::HashMap, io::Read, time::Duration};

use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::{
    sync::{mpsc, watch},
    time::{Instant, sleep, timeout},
};
use tokio_tungstenite::tungstenite::Message as Frame;

use crate::api;

const PING_EVERY: Duration = Duration::from_secs(30);
const PONG_WITHIN: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub enum DealerEvent {
    Connected { connection_id: String },
    Message(Message),
    Disconnected,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub uri: String,
    pub headers: HashMap<String, String>,
    pub payloads: Vec<Value>,
}

impl Message {
    pub fn json(&self) -> Option<&Value> {
        self.payloads.first().filter(|payload| payload.is_object())
    }

    pub fn bytes(&self) -> Option<Vec<u8>> {
        let encoded = self.payloads.first()?.as_str()?;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        let gzip = self
            .headers
            .get("Transfer-Encoding")
            .is_some_and(|encoding| encoding.eq_ignore_ascii_case("gzip"))
            || raw.starts_with(&[0x1f, 0x8b]);
        if gzip {
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(raw.as_slice())
                .read_to_end(&mut decoded)
                .ok()?;
            Some(decoded)
        } else {
            Some(raw)
        }
    }
}

pub fn spawn(
    http: reqwest::Client,
    token: watch::Receiver<String>,
) -> (
    mpsc::UnboundedReceiver<DealerEvent>,
    tokio::task::JoinHandle<()>,
) {
    let (events, receiver) = mpsc::unbounded_channel();
    let task = tokio::spawn(run(http, token, events));
    (receiver, task)
}

async fn run(
    http: reqwest::Client,
    token: watch::Receiver<String>,
    events: mpsc::UnboundedSender<DealerEvent>,
) {
    let mut failures = 0u32;
    loop {
        let started = Instant::now();
        let hosts = api::resolve(&http, "dealer-g2").await;
        let host = hosts
            .into_iter()
            .next()
            .unwrap_or_else(|| "dealer.spotify.com".into());
        let access = token.borrow().clone();
        session(&host, &access, &events).await;
        if events.is_closed() {
            return;
        }
        let _ = events.send(DealerEvent::Disconnected);
        failures = if started.elapsed() > Duration::from_secs(10) {
            0
        } else {
            failures + 1
        };
        let delay = Duration::from_secs(u64::from(failures.min(4)).pow(2).clamp(1, 15));
        sleep(delay).await;
    }
}

async fn session(host: &str, token: &str, events: &mpsc::UnboundedSender<DealerEvent>) {
    let url = format!("wss://{host}/?access_token={token}");
    let Ok(Ok((socket, _))) = timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(url),
    )
    .await
    else {
        return;
    };
    let (mut sink, mut stream) = socket.split();
    let mut next_ping = Instant::now() + PING_EVERY;
    let mut pong_deadline: Option<Instant> = None;
    let mut announced = false;
    loop {
        let wake = pong_deadline.map_or(next_ping, |deadline| deadline.min(next_ping));
        tokio::select! {
            frame = stream.next() => {
                let Some(Ok(frame)) = frame else { return };
                let text = match frame {
                    Frame::Text(text) => text.to_string(),
                    Frame::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Frame::Ping(data) => {
                        let _ = sink.send(Frame::Pong(data)).await;
                        continue;
                    }
                    Frame::Close(_) => return,
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                match value["type"].as_str() {
                    Some("pong") => pong_deadline = None,
                    Some("message") => {
                        let headers: HashMap<String, String> = value["headers"]
                            .as_object()
                            .map(|headers| {
                                headers
                                    .iter()
                                    .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let uri = value["uri"].as_str().unwrap_or_default().to_owned();
                        if !announced && uri.starts_with("hm://pusher/v1/connections/") {
                            let connection_id = headers
                                .get("Spotify-Connection-Id")
                                .cloned()
                                .or_else(|| {
                                    uri.rsplit('/').next().map(|id| {
                                        url::form_urlencoded::parse(format!("x={id}").as_bytes())
                                            .next()
                                            .map(|(_, value)| value.into_owned())
                                            .unwrap_or_default()
                                    })
                                })
                                .unwrap_or_default();
                            announced = true;
                            if events.send(DealerEvent::Connected { connection_id }).is_err() {
                                return;
                            }
                            continue;
                        }
                        let payloads = value["payloads"].as_array().cloned().unwrap_or_default();
                        if events
                            .send(DealerEvent::Message(Message { uri, headers, payloads }))
                            .is_err()
                        {
                            return;
                        }
                    }
                    Some("request") => {
                        if let Some(key) = value["key"].as_str() {
                            let reply = serde_json::json!({"type": "reply", "key": key, "payload": {"success": true}});
                            let _ = sink.send(Frame::Text(reply.to_string().into())).await;
                        }
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep_until(wake) => {
                if pong_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return;
                }
                if Instant::now() >= next_ping {
                    if sink.send(Frame::Text("{\"type\":\"ping\"}".into())).await.is_err() {
                        return;
                    }
                    next_ping = Instant::now() + PING_EVERY;
                    pong_deadline = Some(Instant::now() + PONG_WITHIN);
                }
            }
        }
    }
}
