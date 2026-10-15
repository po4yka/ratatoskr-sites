## ADDED Requirements

### Requirement: The scope query names only notes that a publication selects

`GET /internal/sites/{site}/sources/{source_ref}` and
`GET /internal/sites/{site}/scope?connection_id=&path=` SHALL answer only a loopback peer, and SHALL
answer 404 for a note that no active publication of the effective set of the site selects now.

#### Scenario: A selected note and a note of the same connection outside the publications

- **WHEN** Edge asks the scope of a note that the publication selects and of a note in its excluded folder
- **THEN** the first resolves to its connection and path, and the second is not found, verified by `services/sites/tests/scope.rs::owned_note_outside_publications_is_out_of_scope`

### Requirement: A draft route refuses every claim but a site owner session

The draft routes SHALL refuse the anonymous claim with `sites.request.unauthenticated` and SHALL
return no source.

#### Scenario: An anonymous claim on a draft route

- **WHEN** a request with the anonymous claim reaches `GET /v1/sites/{site}/drafts/sources/{source_ref}`
- **THEN** the answer is 401 `sites.request.unauthenticated` with no source, verified by `services/sites/tests/route_classes.rs::anonymous_claim_is_refused_on_draft_routes`

### Requirement: A change of a selected note becomes one draft change event

When the service consumes `evt.notes.file.changed.v1` for a note that a publication of a site
selects, it SHALL write one `evt.sites.draft.changed.v1` for that site to its outbox, with the
`source_ref` and the revision of the note. A change of a note that no publication selects SHALL
write none.

#### Scenario: A selected note and an unselected note change

- **WHEN** a note that the publication selects changes, and then a note in its excluded folder changes
- **THEN** the outbox holds one draft change event for the first and none for the second, verified by `services/sites/tests/draft_events.rs::file_change_in_publication_emits_draft_changed` and `file_change_outside_publications_emits_nothing`
