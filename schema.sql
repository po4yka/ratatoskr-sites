-- The Sites database, in one file.
--
-- `ratatoskr-sites` applies this at startup, to a fresh database, under an advisory lock. There is
-- no migration ledger and no incremental history: no database holds data that has to survive a
-- schema change (development status). A schema change edits this file in place; the next fresh
-- database has it.
--
-- One schema: `sites`, owned by the role `ratatoskr_sites` in the shared database `ratatoskr`.
-- Sites writes only this schema and reads no other. An owner is an unenforced column that holds the
-- Platform user id from the minted claim, never a cross-schema foreign key. A vault connection is an
-- unenforced id of `ratatoskr-notes` for the same reason.
--
-- The tables are the ones of the workspace change `add-site-publications` (SITE-025), design
-- "Storage". A later task of that change adds the columns that its test needs.
--
-- Conventions, applied to every table and stated once here:
--
--   * Identifiers that the service mints are UUIDv7. No id column has a DEFAULT, so a missing id
--     is an insert error. A site id and a publication id are the validated strings of the manifest
--     contract (`sites/site-manifest.v1`, pattern `^[a-z0-9-]{3,40}$`), not UUIDs.
--
--   * A digest is the lower-case hex of a SHA-256, checked by a CHECK.
--
--   * Closed vocabularies are `text` with a CHECK, not a PostgreSQL enum. One statement alters a
--     CHECK; an enum value cannot be added in the transaction that uses it.
--
--   * Every timestamp is `timestamptz`.
--
--   * No column holds the vault path of a file that a release does not publish. A release holds
--     only what passed the privacy rules (workspace spec `site-release-privacy`).

create schema sites;

comment on schema sites is
    'Sites-owned state: the site registry, the submitted manifests, the approval projection, the '
    'publications, the build queue, the releases with their pages, assets and search index, the '
    'current release of each site, and the outbox and inbox.';

-- The site registry. One row per site id.
create table sites.sites (
    site_id     text        primary key check (site_id ~ '^[a-z0-9-]{3,40}$'),
    owner_id    uuid        not null,
    created_at  timestamptz not null
);

-- Every valid submitted manifest, stored by the SHA-256 of its bytes. The last valid submission of
-- a site is its active manifest (workspace spec `site-manifest`).
create table sites.site_manifests (
    site_id       text        not null references sites.sites (site_id) on delete cascade,
    digest        text        not null check (digest ~ '^[0-9a-f]{64}$'),
    manifest      bytea       not null,
    submitted_at  timestamptz not null,
    is_active     boolean     not null default false,
    primary key (site_id, digest)
);

create unique index site_manifests_one_active
    on sites.site_manifests (site_id) where is_active;

-- The read projection of Platform's `identity.site_approvals`. Only the consumer of
-- `evt.platform.site_approval.changed.v1` writes it; Platform holds the only approval.
create table sites.approval_projection (
    site_id          text        primary key check (site_id ~ '^[a-z0-9-]{3,40}$'),
    approved_digest  text        not null check (approved_digest ~ '^[0-9a-f]{64}$'),
    state            text        not null check (state in ('approved', 'withdrawn')),
    capabilities     text[]      not null default '{}',
    hostname         text        not null,
    changed_at       timestamptz not null
);

-- A publication belongs to one site and selects files from one vault connection.
create table sites.publications (
    site_id           text        not null references sites.sites (site_id) on delete cascade,
    publication_id    text        not null check (publication_id ~ '^[a-z0-9-]{3,40}$'),
    connection_id     uuid        not null,
    mode              text        not null default 'flagged' check (mode in ('all', 'flagged')),
    release_mode      text        not null default 'automatic'
                                  check (release_mode in ('automatic', 'manual')),
    state             text        not null default 'active' check (state in ('active', 'suspended')),
    included_folders  text[]      not null default '{}',
    excluded_folders  text[]      not null default '{}',
    included_tags     text[]      not null default '{}',
    frontmatter_allow text[]      not null default '{}',
    created_at        timestamptz not null,
    primary key (site_id, publication_id)
);

create index publications_by_connection on sites.publications (connection_id);

-- The internal queue of builds (design D6). One worker per site coalesces the rows that arrive
-- while a build runs into one follow-up build.
create table sites.build_requests (
    request_id    uuid        primary key,
    site_id       text        not null references sites.sites (site_id) on delete cascade,
    origin        text        not null check (origin in ('command', 'file_change', 'removal')),
    operation_id  uuid,
    requested_at  timestamptz not null,
    -- The builds of this request that failed for a transient reason; the worker tries at most
    -- three times before it reports the failure.
    attempts      integer     not null default 0 check (attempts >= 0),
    check ((origin = 'command') = (operation_id is not null))
);

create index build_requests_by_site on sites.build_requests (site_id, requested_at);

-- The builds that a command requested and that made no release, for the release history of the
-- owner. The operation of Platform holds the same failure; this copy is what the web client reads
-- from the sites service.
create table sites.failed_builds (
    operation_id  uuid        primary key,
    site_id       text        not null references sites.sites (site_id) on delete cascade,
    failed_at     timestamptz not null,
    code          text        not null,
    message       text        not null
);

create index failed_builds_by_site on sites.failed_builds (site_id, failed_at);

-- An immutable release, named by the SHA-256 of its canonical document (RFC 8785).
create table sites.releases (
    release_id  text        primary key check (release_id ~ '^[0-9a-f]{64}$'),
    site_id     text        not null references sites.sites (site_id) on delete cascade,
    document    bytea       not null,
    page_count  integer     not null check (page_count >= 0),
    built_at    timestamptz not null
);

create index releases_by_site on sites.releases (site_id, built_at);

-- The pages of a release by slug. The page document is a blob of the sites blob root.
create table sites.release_pages (
    release_id   text not null references sites.releases (release_id) on delete cascade,
    slug         text not null,
    title        text not null,
    page_digest  text not null check (page_digest ~ '^[0-9a-f]{64}$'),
    -- The publications that select the note of the page. The delivery serves the page only while
    -- one of them is in the effective set of the site.
    publication_ids text[] not null default '{}',
    primary key (release_id, slug)
);

-- The assets of a release by digest. Only an attachment that a published page uses is here.
create table sites.release_assets (
    release_id    text   not null references sites.releases (release_id) on delete cascade,
    digest        text   not null check (digest ~ '^[0-9a-f]{64}$'),
    media_type    text   not null,
    length_bytes  bigint not null check (length_bytes >= 0),
    -- The publications of the pages that use the asset.
    publication_ids text[] not null default '{}',
    primary key (release_id, digest)
);

-- Each note and each attachment that a release publishes, by vault connection. A change of one of
-- these files requests a build of an automatic publication (workspace spec `site-publications`,
-- "A build starts only from a request or from a selected file change"), and a removal build reads
-- the files from here. Only a published file is here, so the table keeps the rule that no column
-- holds the path of an unpublished file.
create table sites.release_sources (
    release_id     text not null references sites.releases (release_id) on delete cascade,
    connection_id  uuid not null,
    path           text not null,
    -- The file as the notes index gave it to the build (`IndexedFile`, with its revision `BlobRef`):
    -- a removal build rebuilds from these files and never reads the notes index, so it publishes
    -- no note change that the owner did not release.
    file           jsonb not null,
    primary key (release_id, connection_id, path)
);

-- The search index of a release (design D8): one row per page. Search reads only the rows of the
-- current release. The title, the snippet (the start of the page text, at most 300 Unicode scalar
-- values) and the tsvector come from the filtered page tree, so they hold only what a published
-- page shows.
create table sites.release_search (
    release_id  text     not null references sites.releases (release_id) on delete cascade,
    slug        text     not null,
    title       text     not null,
    snippet     text     not null check (char_length(snippet) <= 300),
    document    tsvector not null,
    publication_ids text[] not null default '{}',
    primary key (release_id, slug)
);

create index release_search_document on sites.release_search using gin (document);

-- One row per site: its current release. One UPDATE in the transaction of the outbox row makes a
-- release current.
create table sites.current_releases (
    site_id          text        primary key references sites.sites (site_id) on delete cascade,
    release_id       text        not null references sites.releases (release_id),
    made_current_at  timestamptz not null
);

-- The transactional outbox: `evt.sites.release.published.v1` and the operation reports. The relay
-- publishes the rows in the order of `position`, each on its subject, and marks a row published
-- only after the bus acknowledged it.
create table sites.outbox (
    event_id      uuid        primary key,
    position      bigint      generated always as identity,
    subject       text        not null,
    payload       jsonb       not null,
    created_at    timestamptz not null,
    published_at  timestamptz,

    -- A CLOSED list: a subject that Sites does not publish cannot be inserted.
    constraint outbox_subject_is_known
        check (subject in ('evt.sites.release.published.v1', 'evt.platform.operation.reported.v1'))
);

create index outbox_unpublished on sites.outbox (position) where published_at is null;

-- The inbox: the id of every consumed bus message, so that a redelivery is applied once.
create table sites.inbox (
    message_id   uuid        primary key,
    subject      text        not null,
    consumed_at  timestamptz not null
);
