//! The index of a vault connection, as the notes internal index route gives it.
//!
//! `ratatoskr-notes` serves `GET /internal/connections/{connection}/files` on its loopback domain
//! listener (workspace change `add-site-publications`, tasks 4.1 to 4.5): `{"files": [...]}` in the
//! order of the paths, or 404 `notes.connection.not_found`. The build reads the index through the
//! trait [`NotesIndex`]; production uses [`HttpNotesIndex`], and a test gives a stub. The route
//! checks no owner: one owner owns every vault connection of the installation.

use std::future::Future;
use std::time::Duration;

use http_body_util::{BodyExt as _, Empty, Limited};
use hyper::StatusCode;
use hyper::body::Bytes;
use hyper::http::Uri;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use ratatoskr_sites_publications::IndexedFile;
use uuid::Uuid;

/// Why the index of a connection could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// The notes service did not answer, answered late, or answered something that is not an
    /// index. A later read can succeed.
    #[error("the notes index of the connection is unavailable")]
    Unavailable,
    /// The connection does not exist in the notes service, or it failed or was removed.
    #[error("the vault connection does not exist")]
    ConnectionNotFound,
}

/// The source of the index of each vault connection.
pub trait NotesIndex: Send + Sync {
    /// Every file of the connection `connection_id`, with its current revision.
    fn files(
        &self,
        connection_id: Uuid,
    ) -> impl Future<Output = Result<Vec<IndexedFile>, IndexError>> + Send;
}

/// The index that the notes internal index route serves.
#[derive(Debug, Clone)]
pub struct HttpNotesIndex {
    base_url: String,
    timeout: Duration,
    max_bytes: usize,
    client: Client<HttpConnector, Empty<Bytes>>,
}

/// The answer of the notes internal index route.
#[derive(Debug, serde::Deserialize)]
struct FilesAnswer {
    files: Vec<IndexedFile>,
}

impl HttpNotesIndex {
    /// An index read from the notes domain listener at `base_url`, for example
    /// `http://127.0.0.1:8099`, with `timeout` for one read and at most `max_bytes` of answer.
    #[must_use]
    pub fn new(base_url: &str, timeout: Duration, max_bytes: usize) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            timeout,
            max_bytes,
            client: Client::builder(TokioExecutor::new()).build_http(),
        }
    }

    async fn read(&self, connection_id: Uuid) -> Result<Vec<IndexedFile>, IndexError> {
        let uri: Uri = format!(
            "{}/internal/connections/{connection_id}/files",
            self.base_url
        )
        .parse()
        .map_err(|_| IndexError::Unavailable)?;
        let response = self
            .client
            .get(uri)
            .await
            .map_err(|_| IndexError::Unavailable)?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Err(not_found(response).await),
            status => {
                tracing::warn!(%status, %connection_id, "the notes index answered an error");
                return Err(IndexError::Unavailable);
            }
        }
        let body = Limited::new(response.into_body(), self.max_bytes)
            .collect()
            .await
            .map_err(|_| IndexError::Unavailable)?
            .to_bytes();
        let answer: FilesAnswer =
            serde_json::from_slice(&body).map_err(|_| IndexError::Unavailable)?;
        Ok(answer.files)
    }
}

/// The largest error answer that the index reads.
const MAX_ERROR_BYTES: usize = 4096;

/// The code of the 404 of notes for a connection that does not exist.
const CONNECTION_NOT_FOUND: &str = "notes.connection.not_found";

/// What a 404 of notes means: a missing connection only when its body is the envelope with the
/// code `notes.connection.not_found`. Any other 404, for example the empty answer of an unknown
/// route, says nothing about the connection.
async fn not_found(response: hyper::Response<hyper::body::Incoming>) -> IndexError {
    let Ok(body) = Limited::new(response.into_body(), MAX_ERROR_BYTES)
        .collect()
        .await
    else {
        return IndexError::Unavailable;
    };
    let code = serde_json::from_slice::<serde_json::Value>(&body.to_bytes())
        .ok()
        .and_then(|envelope| envelope.get("code")?.as_str().map(str::to_owned));
    if code.as_deref() == Some(CONNECTION_NOT_FOUND) {
        IndexError::ConnectionNotFound
    } else {
        IndexError::Unavailable
    }
}

impl NotesIndex for HttpNotesIndex {
    async fn files(&self, connection_id: Uuid) -> Result<Vec<IndexedFile>, IndexError> {
        tokio::time::timeout(self.timeout, self.read(connection_id))
            .await
            .unwrap_or(Err(IndexError::Unavailable))
    }
}
