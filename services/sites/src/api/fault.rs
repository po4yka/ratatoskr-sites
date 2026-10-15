//! The failures of the domain routes and their error envelopes.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ratatoskr_error_contracts::{ErrorCode, ErrorEnvelope};
use ratatoskr_identifiers::SafeMessage;

/// Every failure that a domain route answers with. Each one renders as the contract envelope.
#[derive(Debug)]
pub(crate) enum Fault {
    /// The request carries the anonymous claim, or no owner claim, outside `content/`.
    Unauthenticated,
    /// The internal route was called by a peer that is not on a loopback address.
    Forbidden,
    /// A parameter or a body is not valid.
    Invalid,
    /// The site does not exist, or does not serve what the request asks for.
    SiteNotFound,
    /// The publication does not exist.
    PublicationNotFound,
    /// The current release holds no page with this slug.
    PageNotFound,
    /// The current release holds no asset with this digest.
    AssetNotFound,
    /// No route has this path.
    RouteNotFound,
    /// The route has no handler for this method.
    MethodNotAllowed,
    /// The submitted manifest is refused; the detail names the key or the value.
    ManifestInvalid(String),
    /// A query or a read failed.
    Unavailable,
    /// A deploy or an activation was refused or failed (SITE-026); the store names the code.
    Deploy {
        /// The status of the answer.
        status: StatusCode,
        /// The stable code.
        code: &'static str,
        /// What the deployer reads, for example the path of a refused file.
        message: String,
    },
}

impl Fault {
    fn fields(&self) -> (StatusCode, &'static str, &str, bool) {
        match self {
            Self::Unauthenticated => (
                StatusCode::UNAUTHORIZED,
                "sites.request.unauthenticated",
                "Authentication is required.",
                false,
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "sites.request.forbidden",
                "This route answers the host only.",
                false,
            ),
            Self::Invalid => (
                StatusCode::BAD_REQUEST,
                "sites.request.invalid",
                "The request is invalid.",
                false,
            ),
            Self::PublicationNotFound => (
                StatusCode::NOT_FOUND,
                "sites.publication_not_found",
                "The publication does not exist.",
                false,
            ),
            Self::PageNotFound => (
                StatusCode::NOT_FOUND,
                "sites.page_not_found",
                "The current release has no such page.",
                false,
            ),
            Self::AssetNotFound => (
                StatusCode::NOT_FOUND,
                "sites.asset_not_found",
                "The current release has no such asset.",
                false,
            ),
            Self::SiteNotFound => (
                StatusCode::NOT_FOUND,
                "sites.site_not_found",
                "The site does not exist.",
                false,
            ),
            Self::RouteNotFound => (
                StatusCode::NOT_FOUND,
                "sites.route_not_found",
                "The route does not exist.",
                false,
            ),
            Self::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                "sites.request.method_not_allowed",
                "The route does not accept this method.",
                false,
            ),
            Self::ManifestInvalid(detail) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "sites.manifest_invalid",
                detail.as_str(),
                false,
            ),
            Self::Deploy {
                status,
                code,
                message,
            } => (*status, code, message.as_str(), status.is_server_error()),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "sites.service_unavailable",
                "The sites service is temporarily unavailable.",
                true,
            ),
        }
    }
}

impl From<sqlx::Error> for Fault {
    fn from(error: sqlx::Error) -> Self {
        tracing::warn!(%error, "a query of a domain route failed");
        Self::Unavailable
    }
}

impl IntoResponse for Fault {
    fn into_response(self) -> Response {
        let (status, code, message, retryable) = self.fields();
        let message = safe(message);
        let (Ok(code), Ok(message)) = (ErrorCode::parse(code), SafeMessage::parse(&message)) else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        (status, Json(ErrorEnvelope::new(code, message, retryable))).into_response()
    }
}

/// The longest message of an envelope, in UTF-8 bytes (`SafeMessage`).
const MAX_MESSAGE_BYTES: usize = 1024;

/// `text` with control characters as spaces, cut to [`MAX_MESSAGE_BYTES`] at a character
/// boundary.
fn safe(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_MESSAGE_BYTES));
    for character in text.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if out.len() + character.len_utf8() > MAX_MESSAGE_BYTES {
            break;
        }
        out.push(character);
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        "The request is refused.".to_owned()
    } else {
        trimmed.to_owned()
    }
}
