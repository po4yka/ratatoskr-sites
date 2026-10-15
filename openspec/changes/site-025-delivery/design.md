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
second test reads `sites.release_sources`, the notes and attachments that a release publishes,
which the release transaction writes. It holds only published files, so a change of a published
attachment requests a build too.

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
  released publication, write one build request of the origin `removal`, in each release mode,
  unless a removal request already waits that no build has taken. A build of removal requests only
  reads the publications of the current release that stay in the effective set, from the files
  that the current release was built from (design 13), never from the notes index.
- Each request row counts its own attempts before the build; a transient failure is reported only
  to the operations whose row reached three attempts. A `DELETE` retry refreshes the servable set.
  The pause seam of `Servable` exists only with the feature `test-support`.
- The production index is the HTTP client of the notes internal index route (design 9).

### 13. A removal build releases no new content (workspace tasks 5.51, 5.52)

`sites.release_sources` keeps, for each note and attachment that a release publishes, a projection
of the `IndexedFile` that the build read: the links and embeds whose target the release publishes,
the frontmatter keys that the selecting publications allowed (with `slug`, and `publish` for this
site only), the tags that they include, and the revision `BlobRef`; no heading and no block id. An
attachment is there only by a path that a published note links to or embeds. A build that only removal requests asked for reads these files of the current release
instead of the notes index, applies the publications that stay, and reads the bytes of each
revision from the notes blob root, as every build does. So in each release mode it publishes no
note change that the owner did not release. A removal request that joins a command or a file change
makes an ordinary build, because the command or the automatic mode releases new content anyway.

### 14. Third review (workspace tasks 5.53 to 5.61)

- A build that no command requested reads each manual publication from the files of the current
  release, and only each automatic publication from the notes index. Only a build command releases
  an edit of a manual publication.
- The worker marks the rows that a build takes (`taken_at`); a removal request is deduplicated only
  against rows that wait.
- A 404 of the notes index is a missing connection only with the code `notes.connection.not_found`.
  `RATATOSKR__NOTES__BASE_URL` is a loopback `http://` origin with no path and no query.
- A build from the release reads only the frontmatter keys that the stored files hold. When the
  owner widens the frontmatter allowlist of a publication, a build from the release cannot add the
  new keys: the stored files do not hold them, so the change cannot expose a key that the release
  did not publish. For an automatic publication the new keys appear with the next build that reads
  it from the notes index (a command or a change of one of its notes); for a manual publication
  only with a build command, because every other build reads it from the release.

### 15. Fourth review (workspace tasks 5.62 to 5.66)

- Each source row holds the publications that published it. A publication that a build reads from
  the release gets only the rows that name it (corrected in design 16: it then selects among them
  with its current configuration). The projection keeps only what rendering reads.
- Each note in the sources keeps only the links and embeds that its rendered page resolved, without
  display text; a link inside a comment never reaches them.
- A note that two publications of a site select, for example a manual and an automatic one over the
  same folder, is taken once: from the first of them in the order of the publication ids. In a build
  that no command requested, the served text of that note is the released text when the manual
  publication comes first, and the text of the notes index when the automatic one comes first.

### 16. Fifth review (workspace tasks 5.67 to 5.73)

- A released row is eligible for a publication when the publication published it at release time
  (`publication_ids`) and its current configuration still selects it. Each note row holds the
  selection inputs as published facts: `names_site` (its `publish` value named the site) and
  `selection_tags` (its tags, lower case without `#`, that matched an included tag of a selecting
  publication). A build from the release gives the note `publish: true` when `names_site`, and these
  tags, and runs the selection with the current mode, folders and included tags. A publication read
  from the release can only shrink.
- An attachment row names the publications that selected a note that uses it and that do not
  exclude it; the asset of the release carries the same publications. A schema CHECK refuses a row
  without publications.
- The stored frontmatter of a note holds only the keys that its page rendered in the release (and
  `slug`; never `publish`); corrected in design 17.

### 17. Seventh review (workspace tasks 5.81 to 5.86)

- Whenever a build makes a release whose row exists, current or earlier, the transaction of the
  switch writes its provenance again (source rows, and the publications of pages, search rows and
  assets), after a lock of the current-release row of the site.
- The stored frontmatter of a note holds only the keys that its page rendered, and `slug`. A build
  from the release renders those that the current allowlist of the rendering publication still
  allows: never a key that the page did not show. A wider allowlist applies with the next build
  that reads the notes index.
- The selection inputs of a note row (`names_site`, `selection_tags`) come from the revision that
  the row stores, the one of the first selecting publication, never from the version that another
  publication read.
- A publication joins the publications of a note row, and of its page, only when its own file is
  the revision that the row stores. When a manual publication reads an older revision from the
  release and an automatic one reads a newer one, only the publication whose id comes first, and
  whose revision the page renders, publishes the note in that release.
- An automatic publication that did not join a note row (it read another revision than the one
  that the row stores) loses the note in a removal build, until the next build that reads it from
  the notes index.
- Known limit: when two publications read two revisions of one attachment path, an attachment
  source is kept only for the revision that the rendered page uses and that a publication has
  among its own files; any other revision gets no source and is not served. This fails closed.
- A build that finds its release current and, under the lock of the current-release row, finds
  another release current, writes, switches and announces nothing (workspace task 5.93).
- On a rewrite the publications of every asset of the release are cleared before those of this
  build are set: a guard, because each asset of a page comes from an attachment that the page
  resolved, so no build makes an asset without a source.
