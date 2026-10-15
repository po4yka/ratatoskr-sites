//! SITE-025 tasks 5.4 and 5.6: the shipped deploy files hold what the workspace specs
//! `postgres-provisioning` and `blob-references` require. The files run on the host through `psql`
//! and `systemd-sysusers`, never in CI, so the test reads them as text.

const ROLE_SQL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../deploy/postgres/role.sql"
);
const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../deploy/systemd/sites.conf.example"
);
const SYSUSERS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../deploy/sysusers.d/ratatoskr-sites.conf"
);

/// The read group of the notes blob root. `ratatoskr-notes` owns it; OBS-023 creates it.
const NOTES_BLOB_GROUP: &str = "ratatoskr-notes-blobs";

/// The entries of a sysusers.d file: one list of fields for each line that is not a comment. A
/// quoted GECOS field can hold spaces, so only the first fields of a line are reliable; the test
/// reads the type, the name and the group of an `m` line, which come first.
fn sysusers_entries(text: &str) -> Vec<Vec<&str>> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_whitespace().collect())
        .collect()
}

/// The statements of an SQL file: comment lines removed, whitespace collapsed, lower case.
fn statements(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[test]
fn role_file_creates_the_unprivileged_sites_role() -> Result<(), Box<dyn std::error::Error>> {
    let role = statements(&std::fs::read_to_string(ROLE_SQL)?);

    assert!(
        role.contains("create role ratatoskr_sites login;"),
        "the file creates the role ratatoskr_sites: {role}"
    );
    assert!(
        !role.contains("password"),
        "the role has no password in the repository: {role}"
    );
    assert!(
        role.contains("nobypassrls"),
        "the role cannot bypass row-level security: {role}"
    );
    assert!(
        role.contains(
            "alter role ratatoskr_sites nosuperuser nocreatedb nocreaterole noreplication \
             nobypassrls;"
        ),
        "the role has no privilege: {role}"
    );
    assert!(
        role.contains("grant connect, create on database ratatoskr to ratatoskr_sites;"),
        "the role may connect to ratatoskr and create its schema: {role}"
    );
    assert!(
        role.contains("\\connect ratatoskr revoke all on schema public from ratatoskr_sites;"),
        "the role gets nothing in the schema public of ratatoskr: {role}"
    );

    let example = std::fs::read_to_string(EXAMPLE)?;
    assert!(
        example.lines().any(|line| line.trim()
            == "RATATOSKR__DATABASE__URL=postgres://ratatoskr_sites:CHANGE-ME@127.0.0.1:5432/ratatoskr"),
        "the example connects as ratatoskr_sites to the shared database ratatoskr"
    );
    Ok(())
}

#[test]
fn sysusers_adds_sites_to_the_notes_blob_group() -> Result<(), Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(SYSUSERS)?;
    let entries = sysusers_entries(&text);

    assert!(
        entries
            .iter()
            .any(|fields| fields.as_slice() == ["m", "ratatoskr-sites", NOTES_BLOB_GROUP]),
        "the file holds the line `m ratatoskr-sites {NOTES_BLOB_GROUP}`: {entries:?}"
    );
    assert!(
        !entries.iter().any(|fields| matches!(
            fields.as_slice(),
            ["g" | "u", name, ..] if *name == NOTES_BLOB_GROUP
        )),
        "the file joins {NOTES_BLOB_GROUP} and creates no group of that name: {entries:?}"
    );
    assert!(
        entries
            .iter()
            .any(|fields| fields.starts_with(&["u", "ratatoskr-sites"])),
        "the file still creates the user ratatoskr-sites: {entries:?}"
    );
    Ok(())
}
