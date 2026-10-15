## 1. Selection (workspace tasks 5.10, 5.11)

- [x] 1.1 RED: add `fixtures/selection-vault/` and `crates/publications/tests/selection.rs` with the seven tests of workspace task 5.10, each asserting the exact selected path set. Confirm that each set assertion fails because the stub selector returns the empty set.
- [x] 1.2 GREEN: implement the selection precedence, the modes `all` and `flagged`, and the `publish` site list. Verify 1.1.

## 2. Slugs (workspace tasks 5.12, 5.13)

- [x] 2.1 RED: add `crates/publications/tests/slug.rs` with the three tests of workspace task 5.12 and `frontmatter_slug_outside_the_grammar_fails`. Confirm that the class assertion of `colliding_paths_fail_the_build` fails because the build succeeds with one page.
- [x] 2.2 GREEN: implement slug derivation, the collision check and `sites.slug_invalid`. Verify 2.1.

## 3. Privacy filter (workspace tasks 5.14, 5.15)

- [x] 3.1 RED: add `fixtures/privacy-vault/` and `crates/release/tests/privacy.rs` with the twelve tests of workspace task 5.14. Confirm that the canary assertion of the first fails because the builder publishes the source unfiltered.
- [x] 3.2 GREEN: implement the filter pass on the parsed tree before every renderer and collector, and render every selected page on every build. Verify 3.1.

## 4. Sanitised HTML (workspace tasks 5.16, 5.17)

- [x] 4.1 RED: add `crates/release/tests/sanitise.rs` with the two tests of workspace task 5.16. Confirm that the first fails on the assertion that the HTML holds no `script` element.
- [x] 4.2 GREEN: implement the closed HTML allowlist and render plugin query blocks as code. Verify 4.1.

## 5. Release build (workspace tasks 5.18, 5.19)

- [x] 5.1 RED: add `services/sites/tests/release_build.rs` with the three tests of workspace task 5.18, against a stub notes index and a test database. Confirm that the release-count assertion of the first fails because every build stores a new release row.
- [x] 5.2 GREEN: implement the release document, the release id with `serde_jcs`, the full rebuild and the operation report. Verify 5.1.

## 6. Release switch (workspace tasks 5.20, 5.21)

- [x] 6.1 RED: add `services/sites/tests/release_switch.rs` with the two tests of workspace task 5.20. Confirm that the release-id assertion of the first fails because pages are switched one at a time.
- [x] 6.2 GREEN: make the current-release switch and the outbox row one transaction, pin a page read to its release, and relay `evt.sites.release.published.v1`. Verify 6.1.

## 7. The bound of a note read (review L1 of workspace task 5.9)

- [x] 7.1 RED: add `services/sites/tests/notes_blob_read.rs::reference_over_the_bound_reads_nothing`, which gives a reader with a bound of 4 bytes a reference of 7 bytes and a root that does not exist, and asserts `blob_length`. Confirm that the class assertion fails because the reader ignores the bound.
- [x] 7.2 GREEN: refuse a reference over the bound before the root is touched. The build of task 5.2 reads every note and attachment with the bound of 64 MiB (local design 5). Verify 7.1.

## 8. Review corrections (workspace tasks 5.21a to 5.21j)

- [x] 8.1 RED (5.21a): add `services/sites/tests/release_build.rs::blob_over_the_bound_fails_with_blob_length`, which builds through a `ReleaseBuilder` whose reader has no bound of its own, with an attachment of 64 MiB and one byte. Confirm that the class assertion fails because the build reads it and succeeds.
- [x] 8.2 GREEN (5.21a): `ReleaseBuilder::new` applies the production bound of 64 MiB itself. Verify 8.1.
- [x] 8.3 RED (5.21b): add `selection.rs::excluded_folder_matches_without_case` (the excluded folder `Private` excludes `private/x.md`) and `selection.rs::folder_matches_in_nfc` (a folder written in NFD includes and excludes the NFC path). Confirm that the set assertions fail because folders compare byte for byte.
- [x] 8.4 GREEN (5.21b): compare folders and paths by the path key of `ratatoskr-notes` (NFC, then Unicode default case folding), in the selection and in the attachments that a publication may publish. Verify 8.3.
- [x] 8.5 RED (5.21c): add `release_build.rs::long_failure_detail_is_still_reported` (a failure detail over 1024 characters is still reported, with a safe message) and `release_build.rs::report_failure_keeps_the_build_error` (an outbox that cannot take the report does not replace the class of the build). Confirm that the first fails on the report count and the second on the class.
- [x] 8.6 GREEN (5.21c): cut the message of a report to a safe message, and log a failed report instead of returning it. Verify 8.5.
- [ ] 8.7 RED (5.21d): add `privacy.rs::asset_metadata_comes_from_the_used_attachment`, with a second attachment of the same bytes and another media type that no page uses. Confirm that the media-type assertion fails because the release takes the metadata of the other attachment.
- [ ] 8.8 GREEN (5.21d): take the media type, the size and the blob of an asset from the attachment that the filter resolved for the page. Verify 8.7.
