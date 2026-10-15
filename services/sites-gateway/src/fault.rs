//! The answers that the gateway makes itself, as contract envelopes. A forwarded answer of Edge is
//! never one of them.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use ratatoskr_error_contracts::{ErrorCode, ErrorEnvelope};
use ratatoskr_identifiers::SafeMessage;

/// Every refusal of the gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// No approved site names the host, or two do.
    UnknownHost,
    /// The site of the host has no current deploy.
    NoDeploy,
    /// The current deploy has no file for the path.
    PageNotFound,
    /// A static path was asked with another method than `GET` or `HEAD`.
    MethodNotAllowed,
    /// The `/_r/` path is not an API path of the site of the host.
    NotForwarded,
    /// Edge or the sites tree cannot be read now.
    Unavailable,
}

impl Fault {
    const fn fields(self) -> (StatusCode, &'static str, &'static str, bool) {
        match self {
            Self::UnknownHost => (
                StatusCode::NOT_FOUND,
                "sites.gateway.unknown_host",
                "No site is served on this host.",
                false,
            ),
            Self::NoDeploy => (
                StatusCode::NOT_FOUND,
                "sites.gateway.no_deploy",
                "The site has no deploy yet.",
                false,
            ),
            Self::PageNotFound => (
                StatusCode::NOT_FOUND,
                "sites.page_not_found",
                "The site has no such page.",
                false,
            ),
            Self::MethodNotAllowed => (
                StatusCode::METHOD_NOT_ALLOWED,
                "sites.request.method_not_allowed",
                "The path accepts GET and HEAD only.",
                false,
            ),
            Self::NotForwarded => (
                StatusCode::NOT_FOUND,
                "sites.gateway.route_not_forwarded",
                "The route is not forwarded for this site.",
                false,
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "sites.service_unavailable",
                "The site is temporarily unavailable.",
                true,
            ),
        }
    }
}

impl IntoResponse for Fault {
    fn into_response(self) -> Response {
        let (status, code, message, retryable) = self.fields();
        let (Ok(code), Ok(message)) = (ErrorCode::parse(code), SafeMessage::parse(message)) else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        let mut response =
            (status, Json(ErrorEnvelope::new(code, message, retryable))).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        crate::serve::security_headers(headers);
        response
    }
}
