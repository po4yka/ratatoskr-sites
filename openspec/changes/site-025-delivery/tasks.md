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

- [ ] 6.1 RED: add `services/sites/tests/delivery.rs` with `the_four_routes_answer_for_an_approved_site`, `page_of_earlier_release_is_not_found`, `every_kind_of_absence_is_the_same_answer`, `unchanged_page_answers_304` and `cache_lifetimes_are_bounded`. Confirm that the status assertion of the first fails with 404 because the routes do not exist.
- [ ] 6.2 GREEN: implement the four delivery routes on the domain listener, with `ETag` and `Cache-Control` as the workspace spec `site-content-delivery` states, and publish `openapi/content-delivery.json`. Verify 6.1.

## 7. Search (workspace tasks 5.34, 5.35)

- [ ] 7.1 RED: add `services/sites/tests/search.rs` with `search_never_returns_unselected_content`, `search_follows_the_current_release` and `over_long_query_is_refused_before_search`. Confirm that the status assertion of the last fails because the route does not exist.
- [ ] 7.2 GREEN: build `sites.release_search` per release and serve search from the current release id only. Verify 7.1.

## 8. Owner guide (workspace task 5.36)

- [ ] 8.1 Write `docs/OWNER_GUIDE.md`: the selection precedence, the `publish` site list, the explicit first release, the release modes, the manifest and its approval in Platform, the effective set, and the cache bounds after a withdrawal. No test: documentation. Verify that each capability name and each cache bound matches the workspace specs.
