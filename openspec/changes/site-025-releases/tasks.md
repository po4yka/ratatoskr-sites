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
- [x] 8.7 RED (5.21d): add `privacy.rs::asset_metadata_comes_from_the_used_attachment`, with a second attachment of the same bytes and another media type that no page uses. Confirm that the media-type assertion fails because the release takes the metadata of the other attachment.
- [x] 8.8 GREEN (5.21d): take the media type, the size and the blob of an asset from the attachment that the filter resolved for the page. Verify 8.7.
- [x] 8.9 (5.21e): add `services/sites/tests/outbox_relay.rs::relay_publishes_each_row_once` (against a real JetStream; CI starts one) and `release_switch.rs::failed_switch_changes_nothing`. No failing test first: the code exists, so these are pin tests; each is proven by a mutation (no `Nats-Msg-Id`: 4 messages instead of 2; commit before the outbox row: the current release changes).
- [x] 8.10 RED (5.21f): add `sanitise.rs::self_closing_non_void_elements_are_balanced` (balanced and idempotent output for `<a … />`, `<div/>`, `<table/>`, `<p/>`, `<b/>`) and `sanitise.rs::void_and_raw_elements_leave_no_text` (`embed` is dropped and its text kept; the content of `<script/>` and `<style/>` is removed). Confirm that the balance assertion fails because a self-closed element is written open and never closed.
- [x] 8.11 GREEN (5.21f): open every non-void start tag whatever its self-closing flag, drop `embed` as a void element, and skip the content of a removed element also when it is self-closed. Verify 8.10.
- [x] 8.12 RED (5.21g): add `sanitise.rs::deep_nesting_is_bounded_and_fast` (at most 256 open elements; 50 000 open elements and 50 000 stray end tags in under 3 s) and `sanitise.rs::html_over_one_mebibyte_fails_the_build` (the class `sites.page_too_large`). Confirm that the depth assertion and the class assertion fail.
- [x] 8.13 GREEN (5.21g): drop a start tag deeper than 256 open elements and keep its text, and refuse a page body over 1 MiB of HTML before it is tokenised. Verify 8.12.
- [x] 8.14 RED (5.21h): add `crates/release/tests/contract_markup.rs` with `contract_fixture_markup_is_kept` (the allowlist keeps the `html` of the four valid `sites/page.v1` fixtures of the contracts at 2c72602) and `renderer_matches_the_contract_fixtures` (notes that hold the same content render to the same `html`). Confirm that the first fails on `data-wikilink` and the second on the link markup.
- [x] 8.15 GREEN (5.21h): align the renderer and the allowlist with the fixtures: `ul`, `data-callout-fold`, `data-wikilink`, `data-embed` with the title of the target page, a slug link without a leading `/`, link and image titles, a line break after each top-level block and after `<br>`, and bare boolean attributes. Verify 8.14.
- [x] 8.16 RED (5.21i): add `crates/release/tests/allowlist.rs` with `attributes_follow_the_allowlist` (a table of cases for class values, heading ids, image sources and link targets) and `sanitising_twice_changes_nothing` (500 fixed random fragments), and `sanitise.rs::heading_ids_are_lower_case_ascii`. Confirm that the table fails on the first unknown class and the heading test on the id `café-au-lait!`. The idempotence property passes already after 5.21f.
- [x] 8.17 GREEN (5.21i): keep only the classes of the renderer (`callout`, `callout-title`, `callout-content`, one `language-<lang>` with a lang of at most 32 safe characters), heading ids of `[a-z0-9-]` made by one function for the renderer and the allowlist, and an image source that is `assets/<sha256>` or `https:`. Verify 8.16.
- [x] 8.18 (5.21j): document in README.md the `Content-Security-Policy` `img-src` and the `Referrer-Policy` that a site sets, and that the allowlist removes hiding attributes so that hidden text becomes visible. No test: documentation.
