## 1. Build triggers (workspace tasks 5.22, 5.23)

- [x] 1.1 RED: add `services/sites/tests/build_triggers.rs` with `unrelated_change_starts_no_build`, `changes_during_a_build_coalesce_into_one`, `manual_publication_ignores_file_changes` and `first_release_needs_an_explicit_request`, against a real JetStream (`SITES_TEST_NATS_URL`) with streams and durables that the test provisions as Edge does. The consumers and a build worker that builds every request exist; no rule decides yet. Confirm that the build-count assertion of the last fails because a file change builds a publication that has no release.
- [x] 1.2 GREEN: consume `ratatoskr_sites_note_changes` and `ratatoskr_sites_release_builds`, and implement the coalescing worker of workspace design D6 with the first-release rule and the release modes. Verify 1.1.

## 2. A disconnected vault (workspace tasks 5.24, 5.25)

- [x] 2.1 RED: add `services/sites/tests/disconnect.rs` with `suspended_publication_does_not_build` and `withdrawn_site_is_indistinguishable_from_unknown`. Confirm that the class assertion of the first fails because the build runs.
- [x] 2.2 GREEN: consume `ratatoskr_sites_vault_disconnections`, suspend the publications of the connection, and fail a build of a suspended publication with `sites.publication_suspended`. Verify 2.1.

## 3. Manifest and approval projection (workspace tasks 5.26, 5.27)

- [x] 3.1 RED: add `crates/manifest/tests/validate.rs::unknown_capability_is_refused` and `services/sites/tests/manifest_projection.rs` with `submitted_manifest_alone_serves_nothing`, `sites_service_has_no_approval_route`, `withdrawal_event_stops_the_site` and `internal_manifest_route_serves_loopback_only`. Confirm that the status assertion of `withdrawal_event_stops_the_site` fails because nothing consumes the approval event.
- [x] 3.2 GREEN: implement manifest submission and storage by digest, `GET /v1/sites`, `GET` and `PUT .../manifest`, `GET .../releases`, the internal manifest route, the read projection that consumes `ratatoskr_sites_approvals`, and the in-memory set of servable sites. Verify 3.1.

## 3a. The publication routes (the management routes that the web client reads; no workspace task names them)

- [x] 3.3 RED: add `services/sites/tests/publications.rs` with `owner_writes_reads_and_removes_a_publication` and `invalid_publication_is_refused`, against `GET .../publications` and `GET`, `PUT`, `DELETE .../publications/{publication}`. Confirm that the status assertion of the first fails with 404 because the routes do not exist.
- [x] 3.4 GREEN: implement the four publication routes with the body `{connection_id, mode, release_mode, included_folders, excluded_folders, included_tags, frontmatter_allow}`. Verify 3.3.

## 4. The effective set (workspace tasks 5.28, 5.29)

- [x] 4.1 RED: add `services/sites/tests/effective_set.rs` with `added_capability_waits_for_approval`, `removed_capability_stops_without_approval`, `new_hostname_and_publication_wait_for_approval` and `unrequested_public_capability_is_not_found`. Confirm that the status assertion of the first fails because the active manifest is served without approval.
- [x] 4.2 GREEN: compute the effective set as the workspace spec `site-manifest` defines it, build only its publications, and keep the in-memory set of servable sites from it. Verify 4.1.

## 5. Route classes (workspace tasks 5.30, 5.31)

- [x] 5.1 RED: add `services/sites/tests/route_classes.rs::anonymous_claim_is_refused_outside_content`, which sends an anonymous claim to `GET /v1/sites/{site}/releases` and asserts the unauthenticated error and a body that holds no release data. Confirm that the status assertion fails because the service answers 200 with the release list.
- [x] 5.2 GREEN: refuse an anonymous claim, and a request without the owner claim, on every route outside `content/`. Verify 5.1.

## 6. Delivery (workspace tasks 5.32, 5.33)

- [x] 6.1 RED: add `services/sites/tests/delivery.rs` with `the_four_routes_answer_for_an_approved_site`, `page_of_earlier_release_is_not_found`, `every_kind_of_absence_is_the_same_answer`, `unchanged_page_answers_304` and `cache_lifetimes_are_bounded`. Confirm that the status assertion of the first fails with 404 because the routes do not exist.
- [x] 6.2 GREEN: implement the four delivery routes on the domain listener, with `ETag` and `Cache-Control` as the workspace spec `site-content-delivery` states, and publish `openapi/content-delivery.json`. Verify 6.1.

## 7. Search (workspace tasks 5.34, 5.35)

- [x] 7.1 RED: add `services/sites/tests/search.rs` with `search_never_returns_unselected_content`, `search_follows_the_current_release` and `over_long_query_is_refused_before_search`. Confirm that the status assertion of the last fails because the route does not exist.
- [x] 7.2 GREEN: build `sites.release_search` per release and serve search from the current release id only. Verify 7.1.

## 8. Owner guide (workspace task 5.36)

- [x] 8.1 Write `docs/OWNER_GUIDE.md`: the selection precedence, the `publish` site list, the explicit first release, the release modes, the manifest and its approval in Platform, the effective set, and the cache bounds after a withdrawal. No test: documentation. Verify that each capability name and each cache bound matches the workspace specs.

## 9. Review corrections (workspace tasks 5.37 to 5.42)

- [x] 9.1 RED (5.37): add `services/sites/tests/servable_race.rs::withdrawal_is_not_overwritten_by_a_stale_refresh`, which holds a refresh between its read and its write (the pause seam of `Servable`), withdraws the approval and refreshes the site, then lets the first refresh write. Confirm that the servable assertion fails because the stale refresh writes the approved state back.
- [x] 9.2 GREEN (5.38): hold one lock across the read and the write of every refresh. Verify 9.1.
- [x] 9.3 RED (5.39): add `delivery.rs::removed_publication_stops_at_once`: a publication removed from the approved manifest, and another removed with `DELETE`, leave no page, asset, release entry or search hit, with no build. Confirm that the status assertion fails because the page of the removed publication answers 200.
- [x] 9.4 GREEN (5.40): record the publications of each page, asset and search row of a release, keep the effective publications of each servable site, and filter the release document, the pages, the assets and the search results by them; a publication removed with `DELETE` leaves the effective set. Verify 9.3.
- [x] 9.5 RED (5.41): add `build_triggers.rs::transient_failure_is_retried` (L1), `build_triggers.rs::redelivered_command_builds_once` (L2), `delivery.rs::active_content_is_never_sniffed` (L5) and `delivery.rs::refusals_of_content_routes_are_not_stored` (L7), and set `page_of_earlier_release_is_not_found` back to `publish: false` (L9). Confirm that each fails on its assertion.
- [x] 9.6 GREEN (5.42): keep the request rows of a build that failed for a transient reason and retry it up to three times, record a build command in `sites.inbox`, send `X-Content-Type-Options: nosniff` on every delivery answer and `Content-Disposition: attachment` for an HTML, SVG or XML asset, send `Cache-Control: no-store` on every refusal of a content route, and make the stub index read `publish` from a changed note. Verify 9.5.
- [x] 9.7 Document `HEAD` and 503 in `openapi/content-delivery.json` (L6), and state in `docs/OWNER_GUIDE.md` that the service serves one owner and does not scope sites by owner (L3). No test: documentation.

## 10. Second review and the notes index (workspace tasks 5.44 to 5.50)

- [x] 10.1 RED (5.44): add `services/sites/tests/removal_build.rs` with `removed_publication_is_built_out_of_the_kept_pages` (a publication removed from the manifest requests a build, also in the release mode `manual`, and the new release holds no slug or title of it in the kept pages) and `deleted_and_narrowed_publications_request_a_build` (`DELETE` and a `PUT` that changes a publication request one). Confirm that the request assertions fail because nothing requests a build.
- [x] 10.2 GREEN (5.45): when the effective publications of a released site shrink, or a released publication is written or deleted, write a build request of the origin `removal`, in each release mode. A removal build reads the publications of the current release that stay in the effective set. Verify 10.1.
- [x] 10.3 RED (5.46): add `servable_race.rs::delete_retry_refreshes_the_servable_sites` (a `DELETE` of a publication whose row is gone answers 404 and refreshes the servable sites) and `build_triggers.rs::each_request_counts_its_own_attempts` (a new command that joins a request with two failed attempts is tried again, not reported as failed after one try). Confirm that each fails on its assertion.
- [x] 10.4 GREEN (5.47): refresh the servable sites before the 404 of a `DELETE`; count the attempt of each request row before the build, build nothing when the count cannot be written, and report a transient failure only to the operations whose row reached three attempts; build the pause seam of `Servable` only with the feature `test-support`. Verify 10.3.
- [x] 10.5 RED (5.48): add `services/sites/tests/notes_index_http.rs` against a stub HTTP server: `files_of_a_connection_are_read`, `unknown_connection_is_not_found`, `slow_index_is_unavailable` and `oversized_index_is_unavailable`. Confirm that the first fails because the HTTP index is a stub that reads nothing.
- [x] 10.6 GREEN (5.49): implement `HttpNotesIndex` over `GET /internal/connections/{connection}/files` with a timeout and a body bound, `RATATOSKR__NOTES__BASE_URL` (default `http://127.0.0.1:8099`), the class `sites.connection_not_found` for a 404, and make it the production index. Verify 10.5.
- [x] 10.7 State in `docs/OWNER_GUIDE.md` and the local design that one owner owns every vault connection and that neither service checks the owner of a connection. No test: documentation.

## 11. A removal build releases no new content (workspace tasks 5.51, 5.52)

- [x] 11.1 RED (5.51): add `removal_build.rs::removal_build_keeps_the_released_text`: in the release mode `manual`, the owner changes a note after the release and removes another publication; the removal release holds the released text of the changed note. Confirm that the text assertion fails because the removal build reads the notes index as it is now.
- [x] 11.2 GREEN (5.52): store in `sites.release_sources` the indexed file (with its revision) of each note and each attachment that a release publishes, and build a release that only removal requests asked for from that snapshot of the current release, minus the removed publications, without the notes index. Verify 11.1.

## 12. Third review (workspace tasks 5.53 to 5.61)

- [x] 12.1 RED (5.53): add `services/sites/tests/release_sources.rs::sources_hold_nothing_of_an_unselected_file`, which builds the privacy fixture with two copies of a published image in `private/` and `.obsidian/` and scans every column of every `sites.release_sources` row for each canary and private path. Confirm that it fails because a row holds a path or a field of a file that the release does not publish.
- [x] 12.2 GREEN (5.54): store a projection of each published file: a link or embed target only when the release publishes it, only the frontmatter keys that the selecting publications allow (with `slug` and the `publish` value for this site), and an attachment only by a path that a published note links to or embeds. Verify 12.1.
- [x] 12.3 RED (5.55): add `removal_build.rs::removal_during_a_build_is_not_lost`: a removal build waits at the gate, a second publication is deleted, the gate opens; a second build runs and the current release has no trace of the second publication. Confirm that the build-count assertion fails because the second removal request is not written.
- [x] 12.4 GREEN (5.56): mark the request rows that a build takes (`taken_at`), and dedupe a removal request only against rows that no build has taken. Verify 12.3.
- [x] 12.5 RED (5.57): add `build_triggers.rs::automatic_change_keeps_manual_text`: an automatic and a manual publication; the owner edits a note of the manual one, then a note of the automatic one changes; the new release holds the released text of the manual note. Confirm that the text assertion fails because the build reads the manual note from the notes index.
- [x] 12.6 GREEN (5.58): a build that no command requested reads each manual publication from the files of the current release, never the notes index. Verify 12.5.
- [x] 12.7 RED (5.59): add `notes_index_http.rs::other_not_found_is_unavailable` and `redirect_is_unavailable`, and `services/sites/tests/notes_config.rs::notes_base_url_is_a_loopback_origin`. Confirm that each fails on its assertion, or say which one pins behaviour that exists.
- [x] 12.8 GREEN (5.60): a 404 is `ConnectionNotFound` only with the body code `notes.connection.not_found`; `notes.base_url` must be an `http://` URL of a loopback host with no path and no query; the servable set is written before a removal request. Verify 12.7.
- [x] 12.9 (5.61) Correct design 12 (a removal build reads the files of the current release), and state that a wider frontmatter allowlist applies to a removal build only for the keys that the stored files hold. No test: documentation.

## 13. Fourth review (workspace tasks 5.62 to 5.66)

- [x] 13.1 RED (5.62): add `removal_build.rs::flagged_notes_stay_published_after_builds_from_the_release`: a flagged manual publication publishes a note with `publish: "site-r"` and a note with the tag `#public` (included as `Public`); both stay published after a removal build and after an automatic build that reads the manual publication from the release. Confirm that the slug assertion fails because the build from the release selects again on the projection, which lost both inputs.
- [x] 13.2 GREEN (5.63): store in each source row the publications that selected it, and build a publication from the release from exactly its rows, without selecting again; keep in the projection only what rendering reads. Verify 13.1.
- [x] 13.3 RED (5.64): add comment canaries to `release_sources.rs` (an alias and a heading subpath of a link inside a `%%` comment). Confirm that the canary assertion fails because the projection keeps every link whose target is published.
- [x] 13.4 GREEN (5.65): store only the links and embeds that the rendered page resolved, without `display_text`. Verify 13.3.
- [x] 13.5 (5.66) Correct the comment on the removal requests that wait, design 14 for manual publications, and document a note that a manual and an automatic publication both select. No test: documentation.

## 14. Fifth review (workspace tasks 5.67 to 5.73)

A released row is eligible for a publication when the publication selected it at release time and
its current configuration still selects it: a build from the release can only shrink a publication.

- [x] 14.1 RED (5.67): add `services/sites/tests/release_narrowing.rs::narrowed_publication_drops_its_pages_in_a_build_from_the_release`: a PUT that adds `team` to the excluded folders of a released manual publication, and a PUT that drops its included tag `Public`, each remove their notes in the removal build. Confirm that the slug assertion fails because the build from the release takes every stored row of the publication.
- [x] 14.2 GREEN (5.68): store with each note row whether its `publish` value names the site and the tags (lower case, without `#`) that matched an included tag of a selecting publication, and select its stored rows again with the current configuration of each publication. Verify 14.1.
- [x] 14.3 RED (5.69): add `release_narrowing.rs::deleted_publication_takes_its_attachment`: `secret/x.png`, embedded by a note that `pub-a` (`shared`) and `pub-r` (excludes `secret`) both select, is no longer served at once after `DELETE` of `pub-a`, and the removal release has no trace of it. Confirm that the asset assertion fails because the asset carries the publications of the page that uses it.
- [x] 14.4 GREEN (5.70): give each attachment and asset the publications that selected a page that uses it and that do not exclude it, and give a released attachment only to those publications. Verify 14.3.
- [x] 14.5 RED (5.71): add `release_sources.rs::every_source_row_names_its_publications` (no row without publications; the schema refuses one) and `release_sources.rs::stored_frontmatter_holds_no_publish` (also when the allowlist names `publish`). Confirm that each fails on its assertion.
- [x] 14.6 GREEN (5.72): a CHECK that `publication_ids` is not empty, and no `publish` key in the stored frontmatter. Verify 14.5.
- [x] 14.7 (5.73) Document that the stored frontmatter keys are the union of the allowlists of the selecting publications and that a page renders only the current allowlist of the publication that renders it. No test: documentation.

## 15. Sixth review (workspace tasks 5.74 to 5.80)

- [x] 15.1 RED (5.74): add `release_narrowing.rs::widened_publication_gains_no_attachment_from_the_release` (a PUT that widens and narrows a manual publication) and `release_narrowing.rs::file_change_build_gives_a_manual_publication_no_attachment` (an automatic and a manual publication): the manual publication, read from the release, gains no attachment that it did not publish, and the image stops with the publication that published it. Confirm that the publication assertion fails because the attachment gets every publication that does not exclude it.
- [x] 15.2 GREEN (5.75): an attachment names only the publications that have it among their own files and may publish it, by one function that the release builder shares. Verify 15.1.
- [x] 15.3 RED (5.76): add `release_narrowing.rs::non_ascii_tag_survives_a_build_from_the_release` (`#Öffentlich`). Confirm that the slug assertion fails because the stored tag is lower case and the selection compares ASCII case only.
- [x] 15.4 GREEN (5.77): store a tag in the form that the selection compares, through one normaliser that the selection and the storage share. Verify 15.3.
- [x] 15.5 RED (5.78): add `release_narrowing.rs::unchanged_document_rewrites_its_publications`: when a narrowing leaves the release document unchanged, the sources, pages and assets lose the publication, and after `DELETE` of the other one the page answers 404. Confirm that the publication assertion fails because a build that makes the current release again stores nothing.
- [x] 15.6 GREEN (5.79): a build whose release id is the current one rewrites the publications of the sources, pages, assets and search rows, and the worker refreshes the servable set after it. Verify 15.5.
- [x] 15.7 (5.80): pin tests: the page never renders `publish` (`release_sources.rs::page_frontmatter_holds_no_publish`), and the refused source row fails with SQLSTATE 23514. No failing test first: both pin behaviour that exists.

## 16. Seventh review (workspace tasks 5.81 to 5.86)

- [x] 16.1 RED (5.81): add `services/sites/tests/release_provenance.rs::release_made_again_rewrites_its_provenance`: a build that makes an earlier, not current release id again gets the provenance of that build, and a later widening gives no publication a note that it did not publish. Confirm that the publication assertion fails because the earlier rows stay (`on conflict do nothing`).
- [x] 16.2 GREEN (5.82): whenever the release row exists, write its provenance again in the transaction of the switch, under a lock of the current-release row of the site. Verify 16.1.
- [x] 16.3 RED (5.83): add `release_provenance.rs::deleted_publication_shows_no_new_frontmatter_key` and `release_provenance.rs::widened_allowlist_shows_no_new_key_in_a_build_from_the_release`: after the publication that rendered a page leaves, or widens its allowlist, a build from the release does not show a frontmatter key that the page did not show. Confirm that the text assertion fails because the stored frontmatter holds the union of the allowlists.
- [x] 16.4 GREEN (5.84): store with a note only the frontmatter keys that its page rendered (and `slug`), so a build from the release renders only those that the current allowlist still allows. Verify 16.3.
- [x] 16.5 RED (5.85): add `release_provenance.rs::selection_inputs_come_from_the_stored_revision`: a manual publication that reads an old revision does not keep a note on the `publish` value of the newer revision that an automatic publication read. Confirm that the publication assertion fails because `names_site` comes from the last selecting publication.
- [x] 16.6 GREEN (5.86): compute `names_site` and `selection_tags` from the revision that the row stores. Verify 16.5.
