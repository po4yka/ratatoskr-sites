//! A disposable NATS server whose authorization holds the vendored `SITES` stanza of
//! `deploy/nats/identity.conf` and an administrator. It runs as a Docker container of the image
//! that CI uses, on a free loopback port, and the container is removed when the value is dropped.

use std::process::Command;
use std::time::Duration;

use nkeys::KeyPair;
use uuid::Uuid;

use super::Failure;

/// `nats:2.15.0-alpine` as CI pins it in `.github/workflows/ci.yml`.
const IMAGE: &str = "nats@sha256:ac8f88a6494bffc2c2a5289a0ca61cb28a9145c11ba5677cf24265d07f46d8d4";

/// The placeholder of the public key in the vendored stanza.
const PLACEHOLDER: &str = "UREPLACE_ME_WITH_THE_PUBLIC_NKEY_OF_RATATOSKR_SITES_XXXXXXXX";

/// The vendored stanza.
const STANZA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../deploy/nats/identity.conf"
);

/// One disposable server.
pub(crate) struct IdentityBroker {
    name: String,
    /// The client URL on the host loopback.
    pub(crate) url: String,
    /// The seed of the administrator, which may do everything.
    pub(crate) admin_seed: String,
    /// The directory of the configuration and of the seed file of the `SITES` identity.
    pub(crate) directory: tempfile::TempDir,
}

impl IdentityBroker {
    /// Start the server and wait until it accepts the administrator.
    pub(crate) async fn start() -> Result<Self, Failure> {
        let admin = KeyPair::new_user();
        let sites = KeyPair::new_user();
        let stanza = std::fs::read_to_string(STANZA)?;
        let stanza: String = stanza
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            .replace(PLACEHOLDER, &sites.public_key());
        let config = format!(
            "jetstream {{ store_dir: /tmp/jetstream }}\n\
             authorization {{\n  users = [\n    {{ nkey: {} }}\n    {stanza}\n  ]\n}}\n",
            admin.public_key()
        );
        let directory = tempfile::tempdir()?;
        std::fs::write(directory.path().join("nats.conf"), config)?;
        std::fs::write(directory.path().join("sites.nkey"), sites.seed()?)?;
        let name = format!("ratatoskr-sites-identity-{}", Uuid::now_v7().simple());
        let mount = format!("{}:/etc/nats-fixture:ro", directory.path().display());
        let started = Command::new("docker")
            .args(["run", "--detach", "--name", &name])
            .args(["--publish", "127.0.0.1::4222", "--volume", &mount, IMAGE])
            .args(["-c", "/etc/nats-fixture/nats.conf"])
            .output()?;
        if !started.status.success() {
            return Err(format!(
                "the disposable NATS server did not start: {}",
                String::from_utf8_lossy(&started.stderr)
            )
            .into());
        }
        let mut broker = Self {
            name,
            url: String::new(),
            admin_seed: admin.seed()?,
            directory,
        };
        broker.url = format!("nats://127.0.0.1:{}", broker.port()?);
        broker.ready().await?;
        Ok(broker)
    }

    /// The path of the seed file of the `SITES` identity.
    pub(crate) fn sites_seed(&self) -> std::path::PathBuf {
        self.directory.path().join("sites.nkey")
    }

    fn port(&self) -> Result<String, Failure> {
        let port = Command::new("docker")
            .args(["port", &self.name, "4222/tcp"])
            .output()?;
        let binding = String::from_utf8(port.stdout)?;
        binding
            .lines()
            .next()
            .and_then(|line| line.trim().rsplit_once(':'))
            .map(|(_, port)| port.to_owned())
            .ok_or_else(|| "docker did not report the NATS port".into())
    }

    async fn ready(&self) -> Result<(), Failure> {
        for _ in 0..100 {
            if self.admin().await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("the disposable NATS server did not accept the administrator".into())
    }

    /// A connection of the administrator.
    pub(crate) async fn admin(&self) -> Result<async_nats::Client, Failure> {
        Ok(
            async_nats::ConnectOptions::with_nkey(self.admin_seed.clone())
                .connection_timeout(Duration::from_secs(2))
                .connect(&self.url)
                .await?,
        )
    }
}

impl Drop for IdentityBroker {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "--force", &self.name])
            .output();
    }
}
