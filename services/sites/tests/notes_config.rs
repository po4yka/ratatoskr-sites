//! SITE-025 task 5.59 (review L3 of 5.49): the notes base URL is a loopback origin. The index route
//! of notes answers only loopback and the reply is trusted, so another host, a path or a query
//! refuses to start.

use figment::Figment;
use ratatoskr_sites::config;

/// The configuration with `base_url` as `RATATOSKR__NOTES__BASE_URL`.
fn load(base_url: &str) -> Result<config::SitesConfig, config::ConfigError> {
    config::load_from(
        &Figment::new()
            .merge((
                "database.url",
                "postgres://sites:sites@127.0.0.1:5432/sites",
            ))
            .merge(("notes.base_url", base_url)),
    )
}

#[test]
fn notes_base_url_is_a_loopback_origin() {
    for accepted in [
        "http://127.0.0.1:8099",
        "http://127.0.0.1:8099/",
        "http://[::1]:8099",
        "http://localhost:8099",
    ] {
        assert!(load(accepted).is_ok(), "{accepted} is accepted");
    }
    for refused in [
        "http://192.0.2.10:8099",
        "http://notes.example.test:8099",
        "http://127.0.0.1:8099/v1",
        "http://127.0.0.1:8099/?all=1",
        "https://127.0.0.1:8099",
        "127.0.0.1:8099",
    ] {
        let error = load(refused).err();
        assert!(
            error
                .as_ref()
                .is_some_and(|error| error.report().contains("RATATOSKR__NOTES__BASE_URL")),
            "{refused} is refused and the report names the variable: {error:?}"
        );
    }
}
