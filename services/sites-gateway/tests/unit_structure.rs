//! SITE-026 tasks 7.5 and 7.6: the systemd unit of the gateway supervises it with the deployment
//! and holds the hardening of design D8 (workspace change `add-site-gateway-hosting`).

const UNIT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../deploy/systemd/ratatoskr-sites-gateway.service"
);

/// The values of `key` in `unit`, each line `key=value` once.
fn values<'a>(unit: &'a str, key: &str) -> Vec<&'a str> {
    unit.lines()
        .filter_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
        .collect()
}

/// The words of every `key=` line of `unit`.
fn words<'a>(unit: &'a str, key: &str) -> Vec<&'a str> {
    values(unit, key)
        .into_iter()
        .flat_map(str::split_whitespace)
        .collect()
}

#[test]
fn the_gateway_unit_has_the_supervision_and_hardening_properties() {
    // A missing unit reads as empty, so each property fails on its own assertion.
    let unit = std::fs::read_to_string(UNIT).unwrap_or_default();
    assert_eq!(values(&unit, "Type"), ["exec"], "Type=exec");
    assert_eq!(values(&unit, "PartOf"), ["ratatoskr.target"], "PartOf=");
    assert_eq!(values(&unit, "WantedBy"), ["ratatoskr.target"], "WantedBy=");
    assert_eq!(values(&unit, "MemoryMax"), ["128M"], "MemoryMax=");
    assert_eq!(values(&unit, "CPUQuota"), ["100%"], "CPUQuota=");
    assert!(
        words(&unit, "SupplementaryGroups").contains(&"ratatoskr-sites-bundles"),
        "SupplementaryGroups=ratatoskr-sites-bundles"
    );
    assert!(
        words(&unit, "ReadOnlyPaths").contains(&"/mnt/nvme/ratatoskr/sites"),
        "the bundle path is in ReadOnlyPaths"
    );
    assert!(
        !words(&unit, "ReadWritePaths").contains(&"/mnt/nvme/ratatoskr/sites"),
        "the bundle path is not in ReadWritePaths"
    );
}

/// Review L14: the gateway can reach only loopback and cannot change host resources.
#[test]
fn the_gateway_unit_limits_network_and_host_authority() {
    let unit = std::fs::read_to_string(UNIT).unwrap_or_default();
    assert_eq!(values(&unit, "IPAddressDeny"), ["any"]);
    assert_eq!(values(&unit, "IPAddressAllow"), ["localhost"]);
    assert_eq!(values(&unit, "ProtectHostname"), ["yes"]);
    assert!(values(&unit, "SystemCallFilter").contains(&"~@privileged @resources"));
}
