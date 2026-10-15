//! The hostname table: which site each host selects, read from the `site.json` of every site of the
//! sites tree (workspace design D2). The gateway reads it again every two seconds, so a rewritten
//! projection takes effect within the five seconds that the spec allows.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The content of `site.json` that the sites service writes.
#[derive(Debug, serde::Deserialize)]
struct SiteProjection {
    site_id: String,
    approved: bool,
    hostnames: Vec<String>,
}

/// The site of each host that exactly one approved site names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HostTable {
    sites: BTreeMap<String, String>,
}

impl HostTable {
    /// The table of the tree at `root`. A directory whose `site.json` is missing, cannot be parsed,
    /// names another site or is not approved adds nothing; a host that two sites name selects
    /// neither.
    pub(crate) fn read(root: &Path) -> Self {
        let mut sites: BTreeMap<String, String> = BTreeMap::new();
        let mut shared: BTreeSet<String> = BTreeSet::new();
        let Ok(entries) = std::fs::read_dir(root) else {
            return Self::default();
        };
        for entry in entries.flatten() {
            let directory = entry.file_name().to_string_lossy().into_owned();
            let Ok(bytes) = std::fs::read(entry.path().join("site.json")) else {
                continue;
            };
            let Ok(projection) = serde_json::from_slice::<SiteProjection>(&bytes) else {
                continue;
            };
            if !projection.approved || projection.site_id != directory {
                continue;
            }
            for host in projection.hostnames {
                let host = normalised(&host);
                if sites.insert(host.clone(), directory.clone()).is_some() {
                    shared.insert(host);
                }
            }
        }
        for host in shared {
            tracing::warn!(%host, "two approved sites name one host; it selects neither");
            sites.remove(&host);
        }
        Self { sites }
    }

    /// The site that `host` selects.
    pub(crate) fn site(&self, host: &str) -> Option<&str> {
        self.sites.get(&normalised(host)).map(String::as_str)
    }
}

/// `host` in lower case, without a port and without a trailing dot.
pub(crate) fn normalised(host: &str) -> String {
    let host = host.trim();
    let without_port = match host.rsplit_once(':') {
        Some((name, port)) if !name.contains(':') && port.bytes().all(|b| b.is_ascii_digit()) => {
            name
        }
        _ => host,
    };
    without_port.trim_end_matches('.').to_ascii_lowercase()
}
