## 1. Selection (workspace tasks 5.10, 5.11)

- [x] 1.1 RED: add `fixtures/selection-vault/` and `crates/publications/tests/selection.rs` with the seven tests of workspace task 5.10, each asserting the exact selected path set. Confirm that each set assertion fails because the stub selector returns the empty set.
- [x] 1.2 GREEN: implement the selection precedence, the modes `all` and `flagged`, and the `publish` site list. Verify 1.1.

## 2. Slugs (workspace tasks 5.12, 5.13)

- [ ] 2.1 RED: add `crates/publications/tests/slug.rs` with the three tests of workspace task 5.12 and `frontmatter_slug_outside_the_grammar_fails`. Confirm that the class assertion of `colliding_paths_fail_the_build` fails because the build succeeds with one page.
- [ ] 2.2 GREEN: implement slug derivation, the collision check and `sites.slug_invalid`. Verify 2.1.

## 3. Privacy filter (workspace tasks 5.14, 5.15)

- [ ] 3.1 RED: add `fixtures/privacy-vault/` and `crates/release/tests/privacy.rs` with the twelve tests of workspace task 5.14. Confirm that the canary assertion of the first fails because the builder publishes the source unfiltered.
- [ ] 3.2 GREEN: implement the filter pass on the parsed tree before every renderer and collector, and render every selected page on every build. Verify 3.1.

## 4. Sanitised HTML (workspace tasks 5.16, 5.17)

- [ ] 4.1 RED: add `crates/release/tests/sanitise.rs` with the two tests of workspace task 5.16. Confirm that the first fails on the assertion that the HTML holds no `script` element.
- [ ] 4.2 GREEN: implement the closed HTML allowlist and render plugin query blocks as code. Verify 4.1.

## 5. Release build (workspace tasks 5.18, 5.19)

- [ ] 5.1 RED: add `services/sites/tests/release_build.rs` with the three tests of workspace task 5.18, against a stub notes index and a test database. Confirm that the release-count assertion of the first fails because every build stores a new release row.
- [ ] 5.2 GREEN: implement the release document, the release id with `serde_jcs`, the full rebuild and the operation report. Verify 5.1.

## 6. Release switch (workspace tasks 5.20, 5.21)

- [ ] 6.1 RED: add `services/sites/tests/release_switch.rs` with the two tests of workspace task 5.20. Confirm that the release-id assertion of the first fails because pages are switched one at a time.
- [ ] 6.2 GREEN: make the current-release switch and the outbox row one transaction, pin a page read to its release, and relay `evt.sites.release.published.v1`. Verify 6.1.
