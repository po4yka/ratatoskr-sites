//! SITE-025 task 5.26: a manifest that names a capability outside the closed set is refused, and
//! the refusal names it (workspace spec `site-manifest`).

use ratatoskr_sites_manifest::{MANIFEST_INVALID, parse};

#[test]
fn unknown_capability_is_refused() {
    let manifest = b"site_id = \"docs\"\nhostname = \"docs.example.test\"\n\
        publications = [\"handbook\"]\ncapabilities = [\"pages.read\", \"notes.delete\"]\n";
    let refused = parse(manifest).err();
    assert_eq!(
        refused.as_ref().map(|refusal| refusal.class()),
        Some(MANIFEST_INVALID),
        "the manifest is refused"
    );
    let detail = refused.map(|refusal| refusal.detail).unwrap_or_default();
    assert!(
        detail.contains("notes.delete"),
        "the refusal names the capability: {detail:?}"
    );

    let valid = b"site_id = \"docs\"\nhostname = \"docs.example.test\"\n\
        publications = [\"handbook\"]\ncapabilities = [\"pages.read\"]\n";
    assert!(parse(valid).is_ok(), "a valid manifest is accepted");
}
