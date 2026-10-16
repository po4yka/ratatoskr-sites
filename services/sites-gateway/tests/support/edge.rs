//! A stub of Edge on a random loopback port: it records each request that reaches it and answers
//! JSON, or a server-sent event stream on `.../events`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use futures_util::stream;

use super::Failure;

/// One request that reached the stub.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: HeaderMap,
    pub(crate) peer: SocketAddr,
}

#[derive(Debug, Clone, Default)]
struct Records {
    seen: Arc<Mutex<Vec<Seen>>>,
    sent: Arc<Mutex<Vec<std::time::Instant>>>,
}

/// The running stub.
pub(crate) struct StubEdge {
    pub(crate) url: String,
    records: Records,
    task: tokio::task::JoinHandle<()>,
}

impl StubEdge {
    pub(crate) async fn start() -> Result<Self, Failure> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let records = Records::default();
        let app = Router::new()
            .fallback(answer)
            .with_state(records.clone())
            .into_make_service_with_connect_info::<SocketAddr>();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { url, records, task })
    }

    /// The requests that reached the stub.
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.records
            .seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }

    /// When the stub sent each event of the event stream.
    pub(crate) fn sent(&self) -> Vec<std::time::Instant> {
        self.records
            .sent
            .lock()
            .map(|sent| sent.clone())
            .unwrap_or_default()
    }
}

impl Drop for StubEdge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn answer(
    State(records): State<Records>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let path = request
        .uri()
        .path_and_query()
        .map(ToString::to_string)
        .unwrap_or_default();
    if let Ok(mut seen) = records.seen.lock() {
        seen.push(Seen {
            method: request.method().to_string(),
            path: path.clone(),
            headers: request.headers().clone(),
            peer,
        });
    }
    if path.ends_with("?slow-header") {
        tokio::time::sleep(Duration::from_secs(31)).await;
    }
    if path.contains("/events?keepalive") {
        return events(keep_alive());
    }
    if path.ends_with("/events") {
        return events(three_events(records.sent));
    }
    (
        StatusCode::CREATED,
        [
            ("x-edge-answer", "stub"),
            ("x-ratatoskr-user-id", "private-claim"),
            ("content-type", "application/json"),
        ],
        r#"{"from":"edge"}"#,
    )
        .into_response()
}

fn events(body: Body) -> Response {
    (
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-store"),
        ],
        body,
    )
        .into_response()
}

/// Three events one second apart; the time of each is recorded when it is sent.
fn three_events(sent: Arc<Mutex<Vec<std::time::Instant>>>) -> Body {
    Body::from_stream(stream::unfold(0_u32, move |count| {
        let sent = Arc::clone(&sent);
        async move {
            if count == 3 {
                return None;
            }
            if count > 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            if let Ok(mut sent) = sent.lock() {
                sent.push(std::time::Instant::now());
            }
            let event = format!("id: {count}\nevent: tick\ndata: {count}\n\n");
            Some((Ok::<_, std::io::Error>(event), count + 1))
        }
    }))
}

/// A keep-alive comment every fifteen seconds, eight times: two minutes.
fn keep_alive() -> Body {
    Body::from_stream(stream::unfold(0_u32, |count| async move {
        if count == 8 {
            return None;
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
        Some((Ok::<_, std::io::Error>(": keep-alive\n\n"), count + 1))
    }))
}
