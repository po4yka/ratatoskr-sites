## Why

Workspace tasks 5.22 to 5.36 of `add-site-publications` (SITE-025) make the releases of this
repository reachable: what starts a build, what a disconnected vault does to a publication, the
manifest and the read projection of the approvals that Platform holds, the effective set of a site,
the refusal of the anonymous claim outside `content/`, the four routes of the Content Delivery API,
search over the current release, and the owner guide. The cross-repository rules are the workspace
specs `site-publications`, `site-manifest`, `site-route-classes` and `site-content-delivery`. This
change implements them as the tasks word them, and records only the decisions that are local to
this repository.

## What Changes

- Consume the four durables that Edge provisions: `ratatoskr_sites_release_builds`,
  `ratatoskr_sites_note_changes`, `ratatoskr_sites_vault_disconnections` and
  `ratatoskr_sites_approvals`. The service never creates a stream or a consumer.
- One build worker coalesces the build requests of a site, applies the first-release rule and the
  release modes, and refuses a build of a suspended publication.
- Add the crate `crates/manifest`: the parse and the validation of `ratatoskr-site.toml`.
- Add the domain routes on the listener `127.0.0.1:8100`: the management routes that Edge forwards
  (`GET /v1/sites`, `GET` and `PUT .../manifest`, `GET .../releases`), the loopback route
  `GET /internal/sites/{site}/manifest` that Edge reads, and the four delivery routes under
  `.../content/`.
- Publish `openapi/content-delivery.json`, the description of the four delivery routes, which
  references the contract schemas `sites/release.v1` and `sites/page.v1`. The site SDK pins it.
- Add `docs/OWNER_GUIDE.md`.
- The notes internal index route (workspace tasks 4.1 to 4.5) is not built yet. The service keeps
  the trait `NotesIndex`; in production it uses an index that is always unavailable, so a build
  fails with `sites.notes_index_unavailable` until the route exists.

## Capabilities

### New Capabilities

- `sites-delivery`: the local decisions of the build triggers, the effective set, the delivery
  routes and search.

### Modified Capabilities

None.

## Impact

`services/sites`, a new crate `crates/manifest`, `schema.sql`, `openapi/`, `docs/`. New
dependencies: `toml` (the manifest), `futures-util` (the consumer stream, as in `ratatoskr-notes`),
`ratatoskr-notes-contracts` (the two notes events) at the one contracts revision of the fleet.
