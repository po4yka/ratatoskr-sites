## Context

The workspace design `add-site-publications` D6 to D11 decides the cross-repository shape, and
Platform's ADR-0017 decides the route classes at Edge. This file records only the decisions that
are local to this repository.

## Decisions

### 1. The service only reads the durables that Edge provisions

Each of the four durables is looked up by name on its stream, and its filter, its acknowledgement
policy and its ack wait are compared with the values of Edge (`ratatoskr-platform`,
`crates/eventing/src/stream.rs`): 300 s for the build command, 120 s for a note change, 30 s for
the other two. A durable that is missing or differs is logged, and the consumer tries again after
30 s. A message is acknowledged after its effect is committed. A message that is not a valid
envelope is terminated. Every other failure asks for a redelivery after 5 s. The test provisions
its own streams and durables, with subjects under a prefix of its own, so that tests do not share a
subject with each other or with a stream `ratatoskr_events`.

### 2. One build worker coalesces the requests

A build request is a row of `sites.build_requests`: a command, keyed by its command id so that a
redelivery adds nothing, or a file change. One worker task builds one site at a time. It reads every
request row of the site, runs one build, and then deletes exactly those rows, so the requests that
arrive while a build runs make one more build. A wake-up channel of 256 site ids tells the worker
which site to read. When the channel is full, the worker finds the rows on its sweep every 30 s, and
on its start.

A build that a command requested reads every publication of the effective set. A build that only
file changes requested reads only the publications that the current release contains, so the first
release that contains a publication always comes from a command (workspace design D6).

### 3. Which file change requests a build

For each automatic, active publication over the connection of the change whose site has a current
release that contains the publication: the change requests a build when the publication selects
the file now (from the notes index of the connection) or selected it in the current release. The
second test reads `sites.release_sources`, the vault paths of the notes that a release publishes,
which the release transaction writes. It holds only paths of published notes. A change of an
attachment requests no build; the owner requests one.

### 4. A suspended publication fails the build

A build fails with `sites.publication_suspended` when one of the publications that it reads is
suspended, so the current release stays current and keeps the pages of that publication. Only a
build command reports the failure; a build from a file change is logged.

### 5. The effective set and the servable sites

The effective set is computed from the rows of this service alone: the projection row holds the
approved digest, the approved capabilities and the hostname, and `sites.site_manifests` holds the
bytes of the approved manifest, so its publications are known. A site is servable when its effective
set holds a public capability and the site has a current release. The service keeps the servable
sites, each with its public capabilities and its current release id, in memory. It loads them at
start, and refreshes one site after an approval event, a manifest submission and a new release.
Every absence of a delivery route reads only this set.

A build of a site whose effective set names no publication fails with `sites.publication_not_found`.

### 6. The routes and their answers

| Route | Answer |
|---|---|
| `GET /v1/sites` | `{"sites": [...]}`, in ascending `site_id`, at most 100 sites |
| `GET /v1/sites/{site}/manifest` | the active manifest, or 404 `sites.site_not_found` |
| `PUT /v1/sites/{site}/manifest` | the body is the TOML bytes, at most 64 KiB; 200 with the stored manifest, or 422 `sites.manifest_invalid` |
| `GET /v1/sites/{site}/releases` | `{"releases": [...]}`, newest first, at most 100 releases |
| `GET /internal/sites/{site}/manifest` | as `GET .../manifest`, only to a loopback peer, else 403 `sites.request.forbidden` |
| `GET`, `HEAD .../content/release` | the current `sites/release.v1` document |
| `GET`, `HEAD .../content/pages/{slug}` | one `sites/page.v1` of the current release, or 404 `sites.page_not_found` |
| `GET`, `HEAD .../content/assets/{sha256}` | the bytes and media type of one asset of the current release, or 404 `sites.asset_not_found` |
| `GET`, `HEAD .../content/search` | `{"results": [{"slug", "title", "snippet"}], "has_more"}` |

Every other route is 404 `sites.route_not_found`. A request outside `content/` with the anonymous
claim, or without the owner claim `x-ratatoskr-user-id`, is 401 `sites.request.unauthenticated`
(Edge never relays a 401 of a service; this is the second line of ADR-0017). An invalid request is
400 `sites.request.invalid`. The submission of a manifest names the owner of a new site from the
owner claim.

### 7. Cache and conditional requests

The release, page and search routes carry `Cache-Control: public, max-age=60` and the `ETag`
`"<release id>"`; the asset route carries `Cache-Control: public, max-age=3600` and the `ETag`
`"<sha256>"`. A matching `If-None-Match` is 304 with no body. The one answer of every absence is
404 `sites.site_not_found` with `Cache-Control: no-store`.

### 8. Search

Each page of a release has one row of `sites.release_search`: the title, the start of the page
text (at most 300 Unicode scalar values) as the snippet, and a `tsvector` of the `simple`
configuration over the title and the first 256 KiB of the page text. The page text is every text
value of the page tree, so it holds only what passed the privacy filter. A search reads only the
rows of the current release id, ranks by `ts_rank`, and breaks a tie by slug.

### 9. The notes index in production

The service reads `GET /internal/connections/{connection}/files` of the notes domain listener
(`RATATOSKR__NOTES__BASE_URL`, default `http://127.0.0.1:8099`) with a timeout of 30 s and an
answer of at most 32 MiB. A 404 of notes is `sites.connection_not_found`, which a retry does not
help; a timeout, an error status, an oversized or unreadable answer is
`sites.notes_index_unavailable`, which the worker tries again. A note change of a connection that
notes does not know selects nothing now.

### 10. The tests that observe the servable set

The content routes come in task 6.2, after the tasks of the manifest and the effective set. So the
tests of tasks 3.1 and 4.1 observe `Servable::serving`, the one absence path that the delivery
routes use, and `delivery.rs::every_kind_of_absence_is_the_same_answer` shows that the routes use
it.

### 11. Review corrections (workspace tasks 5.37 to 5.42)

- The servable set is refreshed one site at a time under one lock, held across the read of the
  database and the write of the set.
- Each page, asset and search row of a release records the publications that put it there. The
  delivery serves it only while one of them is in the effective set, and the release route drops the
  entries of a removed publication. A deleted publication leaves the effective set. The `ETag` of the
  release, page and search answers is `"<release id>.<16 hex of the SHA-256 of the effective
  publications>"`.
- A build that fails for a transient reason keeps its rows and is tried again after 5 s, at most
  three times; only the last failure is reported. A build command is recorded in the inbox.
- Every delivery answer carries `X-Content-Type-Options: nosniff`; an HTML, SVG or XML asset is
  served with `Content-Disposition: attachment`; every refusal and failure of a content route carries
  `Cache-Control: no-store`.
- The service serves one owner and does not scope sites by owner. One owner owns every vault
  connection: neither this service nor the notes index route checks the owner of a connection.

### 12. Second review (workspace tasks 5.44 to 5.49)

- A publication that leaves the effective set of a released site, and a change or delete of a
  released publication, write one build request of the origin `removal`, in each release mode. It
  reads the publications of the current release that stay in the effective set, and the notes as
  they are when it runs.
- Each request row counts its own attempts before the build; a transient failure is reported only
  to the operations whose row reached three attempts. A `DELETE` retry refreshes the servable set.
  The pause seam of `Servable` exists only with the feature `test-support`.
- The production index is the HTTP client of the notes internal index route (design 9).
