## Why

Workspace group 9 of `add-site-owner-editing-and-live` (SITE-027) gives the owner of a site a way
to read and edit the source notes of the site's publications from the site itself. Edge serves the
owner session, enforces the capabilities and publishes the edit command. The sites service holds
what only it knows: which notes the publications of a site select, the opaque `source_ref` of each
of them, and which file changes the owner sessions must hear about. The cross-repository rules are
the workspace specs `site-owner-editing`, `site-route-classes`, `site-live-events` and
`fleet-bus-topology`. This change implements group 9 as the workspace tasks word it, and records
only the decisions that are local to this repository.

## What Changes

- Add the loopback scope query: `GET /internal/sites/{site}/sources/{source_ref}` answers
  `{connection_id, path}`, and `GET /internal/sites/{site}/scope?connection_id=&path=` answers
  `{source_ref}`. Both answer 404 for a note that no publication of the site selects.
- Add the draft routes `GET /v1/sites/{site}/drafts/pages/{slug}` and
  `GET /v1/sites/{site}/drafts/sources/{source_ref}`, which answer the Markdown source, the
  `source_ref`, the current revision and the slug to a site owner session.
- Refuse the anonymous claim on the draft routes, as on every route outside `content/`.
- Publish `evt.sites.draft.changed.v1` through the outbox when a note that a publication of the site
  selects changes in the vault.
- Copy the SITES stanza of Platform, which allows `evt.sites.draft.changed.v1`, into
  `deploy/nats/identity.conf`.

## Capabilities

### New Capabilities

- `sites-owner-editing`: the local decisions of the scope query, the draft routes and the draft
  change event.

### Modified Capabilities

None.

## Impact

`services/sites`, `schema.sql` (a table of source references, edited in place), `deploy/nats/`,
`README.md`. The `uuid` feature `v4` (no new crate).
