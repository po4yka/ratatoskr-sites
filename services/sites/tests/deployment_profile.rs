//! SITE-025 task 5.2: the shipped operator example is loadable through the configuration loader
//! and enables the deployment profile, so it cannot rot unnoticed.

use std::collections::BTreeMap;
use std::path::Path;

use figment::Figment;
use ratatoskr_sites::config;

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../deploy/systemd/sites.conf.example"
);

/// Parses a systemd `EnvironmentFile`: `KEY=VALUE` lines, `#` comments, blank lines.
fn parse_environment_file(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {} is not KEY=VALUE", number + 1))?;
        if entries
            .insert(key.trim().to_owned(), value.trim().to_owned())
            .is_some()
        {
            return Err(format!("{key} is set twice"));
        }
    }
    Ok(entries)
}

/// The figment that `config::load` builds from the process environment, built from the entries of
/// the file instead: `RATATOSKR__API__LISTEN_ADDRESS` becomes the key `api.listen_address`.
fn figment_of(entries: &BTreeMap<String, String>) -> Result<Figment, String> {
    let mut figment = Figment::new();
    for (key, value) in entries {
        let path = key
            .strip_prefix("RATATOSKR__")
            .ok_or_else(|| format!("{key} has no RATATOSKR__ prefix"))?
            .split("__")
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
            .join(".");
        figment = figment.merge((path, value.clone()));
    }
    Ok(figment)
}

#[test]
fn shipped_example_enables_the_flow() -> Result<(), Box<dyn std::error::Error>> {
    let entries = parse_environment_file(&std::fs::read_to_string(EXAMPLE)?)?;

    // Each key is asserted in the file before its loaded value: a compiled default would hide a
    // key that the file does not set.
    assert_eq!(
        entries
            .get("RATATOSKR__API__LISTEN_ADDRESS")
            .map(String::as_str),
        Some("127.0.0.1:8100"),
        "the example sets the domain listener that Edge routes /v1/sites to"
    );
    assert_eq!(
        entries.get("RATATOSKR__ADMIN__BIND").map(String::as_str),
        Some("127.0.0.1:9472"),
        "the example sets the operator listener"
    );
    assert_eq!(
        entries
            .get("RATATOSKR__BUS__NKEY_SEED_PATH")
            .map(String::as_str),
        Some("/etc/ratatoskr/sites.nkey"),
        "the example names the seed of the SITES identity"
    );
    assert!(
        entries.contains_key("RATATOSKR__BUS__URL"),
        "the example names the NATS server"
    );

    let loaded = config::load_from(&figment_of(&entries)?).map_err(|error| error.report())?;
    assert_eq!(loaded.api.listen_address.to_string(), "127.0.0.1:8100");
    assert_eq!(loaded.admin.bind.to_string(), "127.0.0.1:9472");
    assert_ne!(
        loaded.api.listen_address, loaded.admin.bind,
        "the domain listener is not the operator listener"
    );
    assert_eq!(
        loaded.bus.nkey_seed_path.as_deref(),
        Some(Path::new("/etc/ratatoskr/sites.nkey"))
    );
    assert_eq!(loaded.bus.url.as_deref(), Some("nats://127.0.0.1:4222"));
    assert_eq!(loaded.bus.inbox_prefix(), "_INBOX_sites");
    Ok(())
}
