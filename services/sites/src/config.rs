//! Typed configuration: the tree, the loader, and the startup rules.
//!
//! One source: `RATATOSKR__` environment variables, with `__` between nesting levels, for example
//! `RATATOSKR__DATABASE__URL`. Built-in defaults fill every other key. There is no configuration
//! file. The variable names are an operational contract that the deploy example
//! `deploy/systemd/sites.conf.example` uses.

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use figment::Figment;
use figment::providers::Env;
use secrecy::SecretString;
use tracing_subscriber::EnvFilter;

/// The environment prefix.
const ENV_PREFIX: &str = "RATATOSKR__";

/// The domain API listener. Allocated in `ratatoskr-workspace/docs/DEPLOYMENT_TARGET.md`;
/// reached only through Edge.
pub const DEFAULT_API_PORT: u16 = 8100;

/// The operator listener: health, metrics and version. Allocated in
/// `ratatoskr-workspace/docs/DEPLOYMENT_TARGET.md`; host only.
pub const DEFAULT_ADMIN_PORT: u16 = 9472;

/// The inbox prefix of every NATS connection of this service. The `SITES` stanza of the deployed
/// ACL (`deploy/nats/identity.conf`) lets the identity subscribe to `_INBOX_sites.>` only, so a
/// reply under another prefix never arrives. The workspace check `bus_acl.rs` reads this literal
/// and its use in `custom_inbox_prefix`.
pub const INBOX_PREFIX: &str = "_INBOX_sites";

/// Everything a Sites process must know before it serves.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SitesConfig {
    /// The operator listener: `/health/live`, `/health/ready`, `/metrics`, `/version`.
    #[serde(default)]
    pub admin: AdminConfig,

    /// The domain API listener that Edge forwards `/v1/sites` to.
    #[serde(default)]
    pub api: ApiConfig,

    /// The `PostgreSQL` connection that holds the `sites` schema. Required.
    pub database: DatabaseConfig,

    /// The NATS connection of the `SITES` identity.
    #[serde(default)]
    pub bus: BusConfig,

    /// The sites blob root and the notes blob root.
    #[serde(default)]
    pub blobs: BlobsConfig,

    /// The notes domain listener that serves the internal index route.
    #[serde(default)]
    pub notes: NotesConfig,

    /// The sites tree of the deploys and the gateway projection (SITE-026).
    #[serde(default)]
    pub deploys: DeploysConfig,

    /// The graceful stop.
    #[serde(default)]
    pub shutdown: ShutdownConfig,

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

/// `RATATOSKR__API__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
    /// The domain API listener address.
    #[serde(default = "default_api_listen_address")]
    pub listen_address: SocketAddr,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            listen_address: default_api_listen_address(),
        }
    }
}

/// `RATATOSKR__DEPLOYS__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploysConfig {
    /// Root-owned proof tools and the approved workflow revision. Absence keeps deploy unavailable.
    #[serde(default)]
    pub verifier: Option<ratatoskr_sites_deploys::VerifierConfig>,
    /// The sites tree: the bundles, the `current` and `previous` links and `site.json` of each
    /// site. The service is its only writer; the sites gateway reads it.
    #[serde(default = "default_deploys_root")]
    pub root: PathBuf,
    /// The largest bundle, the sum of its file sizes.
    #[serde(default = "default_max_bundle_bytes")]
    pub max_bundle_bytes: u64,
    /// How many deploys a site keeps, the current and the previous one among them.
    #[serde(default = "default_retention")]
    pub retention: usize,
}

impl Default for DeploysConfig {
    fn default() -> Self {
        Self {
            verifier: None,
            root: default_deploys_root(),
            max_bundle_bytes: default_max_bundle_bytes(),
            retention: default_retention(),
        }
    }
}

fn default_deploys_root() -> PathBuf {
    PathBuf::from("/mnt/nvme/ratatoskr/sites")
}

/// 64 MiB, the default of workspace design D4.
const fn default_max_bundle_bytes() -> u64 {
    64 * 1024 * 1024
}

const fn default_retention() -> usize {
    5
}

/// `RATATOSKR__DATABASE__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    /// SECRET: the URL carries the password. Renders as `[REDACTED]`.
    pub url: SecretString,
    /// The pool bound. The host has four cores and other tenants; the pool is small on purpose.
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    /// How long a request waits for a connection before it fails.
    #[serde(default = "default_acquire_timeout_seconds")]
    pub acquire_timeout_seconds: u64,
}

impl DatabaseConfig {
    /// The acquire timeout as a [`Duration`].
    #[must_use]
    pub const fn acquire_timeout(&self) -> Duration {
        Duration::from_secs(self.acquire_timeout_seconds)
    }
}

/// `RATATOSKR__BUS__*`.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusConfig {
    /// The NATS server, for example `nats://127.0.0.1:4222`. Without it the service connects to
    /// no bus: a developer machine without NATS.
    #[serde(default)]
    pub url: Option<String>,
    /// The file that holds the nkey seed of the `SITES` identity, `/etc/ratatoskr/sites.nkey` on
    /// the host. Without it a connection has no authentication.
    #[serde(default)]
    pub nkey_seed_path: Option<PathBuf>,
}

impl BusConfig {
    /// The inbox prefix of every NATS connection of this service: [`INBOX_PREFIX`].
    #[must_use]
    pub const fn inbox_prefix(&self) -> &'static str {
        INBOX_PREFIX
    }
}

/// `RATATOSKR__BLOBS__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlobsConfig {
    /// The pages and assets of each release. The unit names it in `ReadWritePaths=`.
    #[serde(default = "default_blob_root")]
    pub root: PathBuf,
    /// The note bytes of `ratatoskr-notes`, read only through the group `ratatoskr-notes-blobs`.
    #[serde(default = "default_notes_root")]
    pub notes_root: PathBuf,
}

impl Default for BlobsConfig {
    fn default() -> Self {
        Self {
            root: default_blob_root(),
            notes_root: default_notes_root(),
        }
    }
}

fn default_blob_root() -> PathBuf {
    PathBuf::from("/mnt/nvme/ratatoskr/blobs/ratatoskr-sites")
}

fn default_notes_root() -> PathBuf {
    PathBuf::from("/mnt/nvme/ratatoskr/blobs/ratatoskr-notes")
}

/// `RATATOSKR__NOTES__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotesConfig {
    /// The base URL of the notes domain listener, `http://127.0.0.1:8099` on the host. The service
    /// reads `GET /internal/connections/{connection}/files` there.
    #[serde(default = "default_notes_base_url")]
    pub base_url: String,
}

impl Default for NotesConfig {
    fn default() -> Self {
        Self {
            base_url: default_notes_base_url(),
        }
    }
}

fn default_notes_base_url() -> String {
    "http://127.0.0.1:8099".to_owned()
}

/// `RATATOSKR__SHUTDOWN__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownConfig {
    /// How long in-flight requests may run after the stop signal.
    #[serde(default = "default_grace_seconds")]
    pub grace_seconds: u64,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            grace_seconds: default_grace_seconds(),
        }
    }
}

/// `RATATOSKR__TELEMETRY__*`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    /// `text` for a terminal, `json` for the journal.
    #[serde(default)]
    pub log_format: LogFormat,
    /// A tracing-subscriber `EnvFilter` directive string.
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

/// The log line format.
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

fn default_api_listen_address() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_API_PORT)
}

const fn default_max_connections() -> u32 {
    4
}

const fn default_acquire_timeout_seconds() -> u64 {
    5
}

const fn default_grace_seconds() -> u64 {
    10
}

fn default_log_filter() -> String {
    "info".to_owned()
}

/// Whether `url` is an `http://` origin of a loopback host (`127.0.0.0/8`, `::1` or `localhost`)
/// with no path and no query: the notes index route answers only loopback, and its answer is
/// trusted.
fn is_loopback_origin(url: &str) -> bool {
    let Ok(uri) = url.parse::<hyper::http::Uri>() else {
        return false;
    };
    let host = uri
        .host()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']');
    let loopback = host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    let bare = uri.path_and_query().is_none_or(|path| path.as_str() == "/");
    uri.scheme_str() == Some("http") && loopback && bare
}

/// One startup-rule violation.
///
/// Every member is `&'static str`, so a supplied value cannot appear in a failure report and the
/// report cannot echo a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The dotted configuration path, for example `database.url`.
    pub key: &'static str,
    /// The environment variable that sets it.
    pub env_var: &'static str,
    /// What the rule requires.
    pub rule: &'static str,
}

/// Every reason a Sites process must refuse to start.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// Extraction failed: a wrong type, a missing key, or an unknown key. figment's message is
    /// not interpolated, because it can quote the supplied value.
    #[error("configuration could not be read")]
    Source(#[from] figment::Error),

    /// The configuration parsed but breaks one or more startup rules.
    #[error("configuration is invalid: {} problem(s)", .0.len())]
    Invalid(Vec<Violation>),
}

impl ConfigError {
    /// The operator-facing report, written to stderr before any subscriber exists. It holds no
    /// supplied value.
    #[must_use]
    pub fn report(&self) -> String {
        let mut report = String::from("ratatoskr-sites: refusing to start; ");
        match self {
            Self::Source(error) => {
                // A key name is not a value, so naming it cannot echo a secret.
                let mut path = error.path.clone();
                let kind = match &error.kind {
                    figment::error::Kind::MissingField(field) => {
                        path.push(field.to_string());
                        "a required key is missing"
                    }
                    figment::error::Kind::UnknownField(_, _) => "an unknown key is set",
                    figment::error::Kind::InvalidType(_, _)
                    | figment::error::Kind::InvalidValue(_, _) => {
                        "a value has the wrong type or form"
                    }
                    _ => "a value is not usable",
                };
                let variable = format!("{ENV_PREFIX}{}", path.join("__").to_uppercase());
                let _ = write!(
                    report,
                    "the configuration could not be read: {kind}: {variable}"
                );
            }
            Self::Invalid(violations) => {
                report.push_str("the configuration is invalid:");
                for violation in violations {
                    let _ = write!(
                        report,
                        "\n  {} ({}): {}",
                        violation.key, violation.env_var, violation.rule
                    );
                }
            }
        }
        report
    }

    /// `78`, `EX_CONFIG` from `sysexits.h`. systemd shows it as `EXIT_CONFIG`.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        78
    }
}

/// Read the process environment and produce a validated configuration.
///
/// # Errors
///
/// [`ConfigError::Source`] when extraction fails, [`ConfigError::Invalid`] with every violation
/// when a startup rule fails.
#[allow(
    clippy::result_large_err,
    reason = "figment::Error is the payload of ConfigError::Source; the value is built once, at \
              startup, on the path that then exits"
)]
pub fn load() -> Result<SitesConfig, ConfigError> {
    load_from(&Figment::new().merge(Env::prefixed(ENV_PREFIX).split("__")))
}

/// Extract and validate from any figment. The seam for configuration tests.
///
/// # Errors
///
/// As [`load`].
#[allow(
    clippy::result_large_err,
    reason = "as `load`: the error payload is figment::Error by design"
)]
pub fn load_from(figment: &Figment) -> Result<SitesConfig, ConfigError> {
    let config: SitesConfig = figment.extract()?;
    let violations = validate(&config);
    if violations.is_empty() {
        Ok(config)
    } else {
        Err(ConfigError::Invalid(violations))
    }
}

fn validate(config: &SitesConfig) -> Vec<Violation> {
    let mut found = Vec::new();
    if EnvFilter::try_new(&config.telemetry.log_filter).is_err() {
        found.push(Violation {
            key: "telemetry.log_filter",
            env_var: "RATATOSKR__TELEMETRY__LOG_FILTER",
            rule: "must parse as a tracing-subscriber EnvFilter directive string, e.g. info",
        });
    }
    if config.admin.bind == config.api.listen_address {
        found.push(Violation {
            key: "api.listen_address",
            env_var: "RATATOSKR__API__LISTEN_ADDRESS",
            rule: "must differ from the operator listener admin.bind",
        });
    }
    if !is_loopback_origin(&config.notes.base_url) {
        found.push(Violation {
            key: "notes.base_url",
            env_var: "RATATOSKR__NOTES__BASE_URL",
            rule: "must be the http:// origin of the loopback notes domain listener, with no path \
                   and no query, e.g. http://127.0.0.1:8099",
        });
    }
    if !(1..=16).contains(&config.database.max_connections) {
        found.push(Violation {
            key: "database.max_connections",
            env_var: "RATATOSKR__DATABASE__MAX_CONNECTIONS",
            rule: "must be 1..=16",
        });
    }
    if !(1..=60).contains(&config.database.acquire_timeout_seconds) {
        found.push(Violation {
            key: "database.acquire_timeout_seconds",
            env_var: "RATATOSKR__DATABASE__ACQUIRE_TIMEOUT_SECONDS",
            rule: "must be 1..=60",
        });
    }
    if !(1..=1024 * 1024 * 1024).contains(&config.deploys.max_bundle_bytes) {
        found.push(Violation {
            key: "deploys.max_bundle_bytes",
            env_var: "RATATOSKR__DEPLOYS__MAX_BUNDLE_BYTES",
            rule: "must be 1..=1073741824",
        });
    }
    if !(2..=100).contains(&config.deploys.retention) {
        found.push(Violation {
            key: "deploys.retention",
            env_var: "RATATOSKR__DEPLOYS__RETENTION",
            rule: "must be 2..=100: the current and the previous deploy are always kept",
        });
    }
    if !(1..=60).contains(&config.shutdown.grace_seconds) {
        found.push(Violation {
            key: "shutdown.grace_seconds",
            env_var: "RATATOSKR__SHUTDOWN__GRACE_SECONDS",
            rule: "must be 1..=60",
        });
    }
    found
}
