## Why

Workspace tasks 5.10 to 5.21 of `add-site-publications` (SITE-025) give this repository its first
release: the selection of a publication, the slug of each page, the privacy filter, the sanitised
HTML, the release id, the current-release switch and the release event. The cross-repository rules
are the workspace specs `site-publications` and `site-release-privacy`. This change implements
them as tasks 5.10 to 5.21 word them, and does not restate them. It records only the decisions that
are local to this repository.

## What Changes

- Add the crate `crates/publications`: the shape of the notes index that a publication reads, the
  selection precedence, the modes `all` and `flagged`, the `publish` site list, and the slug of a
  page with the collision check.
- Add the crate `crates/release`: the comment pass, the parse into one tree, the privacy filter on
  that tree, the AST serialiser, the HTML renderer with a closed allowlist, the asset collector and
  the release document.
- Add the build of a site in `services/sites`: the release id is the SHA-256 of the RFC 8785 form
  of the `sites/release.v1` document; an unchanged build makes no release; the current-release
  switch and the outbox row of `evt.sites.release.published.v1` are one transaction; the outbox
  relay publishes the row.
- A frontmatter `slug` outside the grammar of `Slug` fails the build with the new class
  `sites.slug_invalid`. The workspace spec names no class for it.
- The notes internal index route (workspace tasks 4.1 to 4.5) does not exist yet. The build reads
  the index through a trait; the tests use a stub, and the shape follows the task text of 4.1.

## Capabilities

### New Capabilities

- `sites-releases`: the local decisions of the release build: the invalid slug class, the release
  id, the read of a page pinned to a release, and the bound of a note read.

### Modified Capabilities

None.

## Impact

`crates/publications`, `crates/release`, `services/sites`, `schema.sql`, `fixtures/`. New
dependencies: `pulldown-cmark` (as in `ratatoskr-notes`), `ammonia` for the HTML allowlist,
`serde_jcs` for RFC 8785, and `async-nats` for the relay (as in `ratatoskr-notes`).
