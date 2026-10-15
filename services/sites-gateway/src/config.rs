//! Typed configuration from `RATATOSKR__` environment variables, with `__` between nesting levels,
//! for example `RATATOSKR__EDGE__SOURCE_ADDRESS`. Built-in defaults fill every other key. The
//! variable names are an operational contract that `deploy/systemd/sites-gateway.conf.example`
//! uses.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use figment::Figment;
use figment::providers::Env;

/// The environment prefix.
const ENV_PREFIX: &str = "RATATOSKR__";

/// The public listener that `cloudflared` reaches (workspace design D1).
pub const DEFAULT_PUBLIC_PORT: u16 = 8183;

/// The operator listener: health, metrics and version; host only.
pub const DEFAULT_ADMIN_PORT: u16 = 9473;

/// Everything the gateway must know before it serves.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    /// The operator listener.
    #[serde(default)]
    pub admin: AdminConfig,
    /// The public listener.
    #[serde(default)]
    pub public: PublicConfig,
    /// The sites tree that the sites service writes.
    #[serde(default)]
    pub sites: SitesConfig,
    /// Edge, which the gateway forwards `/_r/` to.
    #[serde(default)]
    pub edge: EdgeConfig,
    /// The token bucket and the body cap.
    #[serde(default)]
    pub limits: LimitsConfig,
    /// Logging.
    #[serde(default)]
    pub telemetry: TelemetryConfig,
}

/// `RATATOSKR__ADMIN__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    /// The operator listener address.
    #[serde(default = "default_admin_bind")]
    pub bind: SocketAddr,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            bind: default_admin_bind(),
        }
    }
}

/// `RATATOSKR__PUBLIC__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicConfig {
    /// The public listener address. Only `cloudflared` reaches it.
    #[serde(default = "default_public_listen_address")]
    pub listen_address: SocketAddr,
}

impl Default for PublicConfig {
    fn default() -> Self {
        Self {
            listen_address: default_public_listen_address(),
        }
    }
}

/// `RATATOSKR__SITES__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SitesConfig {
    /// The sites tree, read only, through the group `ratatoskr-sites-bundles`.
    #[serde(default = "default_sites_root")]
    pub root: PathBuf,
}

impl Default for SitesConfig {
    fn default() -> Self {
        Self {
            root: default_sites_root(),
        }
    }
}

/// `RATATOSKR__EDGE__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeConfig {
    /// The origin of Edge's public listener, `http://127.0.0.1:8080` on the host.
    #[serde(default = "default_edge_url")]
    pub url: String,
    /// The loopback source address of every connection to Edge. Edge trusts
    /// `X-Ratatoskr-Client-IP` only on a connection from it (`RATATOSKR__SITES__GATEWAY_PEER` of
    /// Edge).
    #[serde(default = "default_edge_source_address")]
    pub source_address: IpAddr,
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            url: default_edge_url(),
            source_address: default_edge_source_address(),
        }
    }
}

/// `RATATOSKR__LIMITS__*`.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    /// The tokens that each client key gains per second.
    #[serde(default = "default_rate_per_second")]
    pub rate_per_second: u32,
    /// The most tokens that a client key holds.
    #[serde(default = "default_burst")]
    pub burst: u32,
    /// The largest body of a forwarded request.
    #[serde(default = "default_forwarded_body_bytes")]
    pub forwarded_body_bytes: u64,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            rate_per_second: default_rate_per_second(),
            burst: default_burst(),
            forwarded_body_bytes: default_forwarded_body_bytes(),
        }
    }
}

/// `RATATOSKR__TELEMETRY__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    /// `text` or `json`.
    #[serde(default)]
    pub log_format: LogFormat,
    /// A tracing `EnvFilter` directive string.
    #[serde(default = "default_log_filter")]
    pub log_filter: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_format: LogFormat::default(),
            log_filter: default_log_filter(),
        }
    }
}

/// The format of the log lines on stderr.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// Human-readable lines.
    #[default]
    Text,
    /// One JSON object per line.
    Json,
}

fn default_admin_bind() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_ADMIN_PORT)
}

fn default_public_listen_address() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PUBLIC_PORT)
}

fn default_sites_root() -> PathBuf {
    PathBuf::from("/mnt/nvme/ratatoskr/sites")
}

fn default_edge_url() -> String {
    "http://127.0.0.1:8080".to_owned()
}

const fn default_edge_source_address() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, 83))
}

const fn default_rate_per_second() -> u32 {
    20
}

const fn default_burst() -> u32 {
    60
}

const fn default_forwarded_body_bytes() -> u64 {
    1024 * 1024
}

fn default_log_filter() -> String {
    "info".to_owned()
}

/// A startup rule that the configuration breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The environment variable.
    pub env_var: &'static str,
    /// The rule.
    pub rule: &'static str,
}

/// Why the configuration was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A value could not be read.
    #[error("configuration could not be read: {0}")]
    Source(String),
    /// A startup rule failed.
    #[error("configuration is invalid: {} problem(s)", .0.len())]
    Invalid(Vec<Violation>),
}

impl ConfigError {
    /// A report for stderr. It names variables and rules, never a value.
    #[must_use]
    pub fn report(&self) -> String {
        match self {
            Self::Source(detail) => {
                format!("ratatoskr-sites-gateway: refusing to start; {detail}")
            }
            Self::Invalid(violations) => violations.iter().fold(
                "ratatoskr-sites-gateway: refusing to start; invalid configuration:".to_owned(),
                |report, violation| {
                    format!("{report}\n  {}: {}", violation.env_var, violation.rule)
                },
            ),
        }
    }

    /// `78`, `EX_CONFIG`.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        78
    }
}

/// Read the process environment and produce a validated configuration.
///
/// # Errors
///
/// [`ConfigError`] when a value cannot be read or a startup rule fails.
pub fn load() -> Result<GatewayConfig, ConfigError> {
    load_from(&Figment::new().merge(Env::prefixed(ENV_PREFIX).split("__")))
}

/// Extract and validate from any figment.
///
/// # Errors
///
/// As [`load`].
pub fn load_from(figment: &Figment) -> Result<GatewayConfig, ConfigError> {
    let config: GatewayConfig = figment.extract().map_err(|error| {
        ConfigError::Source(format!("{} ({})", error.kind, error.path.join(".")))
    })?;
    let violations = validate(&config);
    if violations.is_empty() {
        Ok(config)
    } else {
        Err(ConfigError::Invalid(violations))
    }
}

fn validate(config: &GatewayConfig) -> Vec<Violation> {
    let mut found = Vec::new();
    let mut rule = |broken: bool, env_var: &'static str, rule: &'static str| {
        if broken {
            found.push(Violation { env_var, rule });
        }
    };
    rule(
        !config.public.listen_address.ip().is_loopback(),
        "RATATOSKR__PUBLIC__LISTEN_ADDRESS",
        "must be a loopback address: only cloudflared reaches the gateway",
    );
    rule(
        !config.admin.bind.ip().is_loopback(),
        "RATATOSKR__ADMIN__BIND",
        "must be a loopback address",
    );
    rule(
        config.admin.bind == config.public.listen_address,
        "RATATOSKR__PUBLIC__LISTEN_ADDRESS",
        "must differ from the operator listener",
    );
    rule(
        !config.edge.source_address.is_loopback(),
        "RATATOSKR__EDGE__SOURCE_ADDRESS",
        "must be a loopback address, 127.0.0.83 on the host",
    );
    rule(
        !is_loopback_origin(&config.edge.url),
        "RATATOSKR__EDGE__URL",
        "must be the http:// origin of Edge on loopback, e.g. http://127.0.0.1:8080",
    );
    rule(
        config.limits.rate_per_second == 0 || config.limits.burst == 0,
        "RATATOSKR__LIMITS__RATE_PER_SECOND",
        "the rate and the burst must be at least 1",
    );
    rule(
        config.limits.forwarded_body_bytes == 0,
        "RATATOSKR__LIMITS__FORWARDED_BODY_BYTES",
        "must be at least 1",
    );
    rule(
        tracing_subscriber::EnvFilter::try_new(&config.telemetry.log_filter).is_err(),
        "RATATOSKR__TELEMETRY__LOG_FILTER",
        "must parse as a tracing-subscriber EnvFilter directive string, e.g. info",
    );
    found
}

/// Whether `url` is an `http://` origin on a loopback host, with no path and no query.
fn is_loopback_origin(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.contains(['/', '?', '#', '@']) {
        return false;
    }
    let host = rest
        .rsplit_once(':')
        .map_or(rest, |(host, _)| host)
        .trim_start_matches('[')
        .trim_end_matches(']');
    host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}
