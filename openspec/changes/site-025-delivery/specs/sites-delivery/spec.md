## ADDED Requirements

### Requirement: A build from a file change reads only released publications

A build that only file changes requested SHALL read only the publications that the current release
of the site contains. A build that a command requested SHALL read every publication of the
effective set.

#### Scenario: A file change does not release a new publication

- **WHEN** an automatic publication has no release and a note that it selects changes
- **THEN** no build starts and no release exists until a build command arrives, verified by `services/sites/tests/build_triggers.rs::first_release_needs_an_explicit_request`

### Requirement: A build with a suspended publication fails

A build of a site SHALL fail with the class `sites.publication_suspended` when one of the
publications that it reads is suspended, and the current release SHALL not change.

#### Scenario: The only publication is suspended

- **WHEN** the vault of the only publication of a site is disconnected and the owner requests a build
- **THEN** the operation fails with `sites.publication_suspended` and the current release does not change, verified by `services/sites/tests/disconnect.rs::suspended_publication_does_not_build`

### Requirement: The internal manifest route answers a loopback peer only

`GET /internal/sites/{site}/manifest` SHALL answer only a peer on a loopback address, and SHALL
refuse any other peer with 403 and the code `sites.request.forbidden`.

#### Scenario: A peer on another address

- **WHEN** a peer that is not on a loopback address calls the internal manifest route
- **THEN** the answer is 403 with `sites.request.forbidden` and no manifest data, verified by `services/sites/tests/manifest_projection.rs::internal_manifest_route_serves_loopback_only`

### Requirement: The answer of an absence is never stored

The one answer of every absence of the delivery routes SHALL carry `Cache-Control: no-store`, so
that a shared cache does not keep it after the site becomes servable.

#### Scenario: The four kinds of absence

- **WHEN** a client requests the release route of a missing, an unapproved and a withdrawn site, and the search route of a site that does not request `search.publication`
- **THEN** each answer is 404 `sites.site_not_found` with `Cache-Control: no-store` and the same body, verified by `services/sites/tests/delivery.rs::every_kind_of_absence_is_the_same_answer`

### Requirement: A removal requests a build in each release mode

When a publication leaves the effective set of a site that has a current release, or a released
publication is changed or deleted, the sites service SHALL write a build request of the origin
`removal`, in each release mode. A removal build SHALL read only the publications of the current
release that stay in the effective set.

#### Scenario: A deleted or changed publication

- **WHEN** the owner deletes a publication or changes a released one
- **THEN** a build request of the origin `removal` exists, verified by `services/sites/tests/removal_build.rs::deleted_and_narrowed_publications_request_a_build`

#### Scenario: A manual site loses a publication

- **WHEN** the owner removes a publication from the manifest of a site whose publications are in the release mode `manual`
- **THEN** a build runs, and the new release holds no slug or title of the removed publication in the pages that stay, verified by `services/sites/tests/removal_build.rs::removed_publication_is_built_out_of_the_kept_pages`

### Requirement: A removal build releases no new content

A build that only removal requests asked for SHALL read the indexed files that the current release
was built from, and SHALL NOT read the notes index. It publishes no note change that the owner did
not release.

#### Scenario: A manual site with an unreleased change loses a publication

- **WHEN** the owner changes a note of a manual publication after the release, and then removes another publication
- **THEN** the release of the removal build holds the released text of the changed note, verified by `services/sites/tests/removal_build.rs::removal_build_keeps_the_released_text`

### Requirement: The sources of a release hold only what it publishes

`sites.release_sources` SHALL hold, for each note and attachment that a release publishes, only a
projection of its indexed file, which holds only what rendering reads, the publications that
published it, and the selection inputs of a note limited to published facts. An attachment SHALL be
there only by a path that a published note links to or embeds.

#### Scenario: The privacy fixture with copies of a published image

- **WHEN** the privacy fixture, with copies of a published image in `private/` and `.obsidian/`, is built
- **THEN** no column of any source row holds a canary, a private path or a field outside the allowlist, verified by `services/sites/tests/release_sources.rs::sources_hold_nothing_of_an_unselected_file`

### Requirement: A build from the release can only shrink a publication

A build from the release SHALL give a publication only the rows that name it, and SHALL select among
them with the current configuration of the publication.

#### Scenario: Flagged notes in a build from the release

- **WHEN** a flagged manual publication publishes a note with `publish: "site-r"` and a note with the tag `#public`, and a removal build and then an automatic build of the site read it from the release
- **THEN** both notes stay published, verified by `services/sites/tests/removal_build.rs::flagged_notes_stay_published_after_builds_from_the_release`

#### Scenario: A narrowed publication

- **WHEN** the owner adds an excluded folder to a released manual publication, or removes an included tag
- **THEN** the removal build drops the notes that the current configuration no longer selects, verified by `services/sites/tests/release_narrowing.rs::narrowed_publication_drops_its_pages_in_a_build_from_the_release`
