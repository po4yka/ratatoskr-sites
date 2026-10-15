# Ratatoskr Sites

`ratatoskr-sites` is the sites bounded context of Ratatoskr. It selects the notes of a connected
Obsidian vault that the owner publishes, builds an immutable release from them after the privacy
rules, and serves the current release of each approved site through the Content Delivery API. Each
site is an independent UI in its own repository, and reads a release with `@ratatoskr/site-sdk`.

> **Status:** in development. The service selects the notes of each publication, builds an
> immutable release through the privacy filter, makes it current in one transaction and announces
> it, coalesces the builds that commands and note changes request, keeps the manifest and the
> projection of the approvals of Platform, and serves the current release of each approved site
> through the Content Delivery API. It reads the index of each vault connection from the notes
> internal index route.

> [!IMPORTANT]
> **Ratatoskr is in development.** No database holds data that has to survive a schema change.
> While this status holds, these two rules replace what the documents below plan:
>
> - the API and the database keep their first version. There is no `v2` and no later major
>   version.
> - the database has no migrations. One schema definition exists, and a schema change edits it in
>   place.
>
> Only the repository owner changes this status.

## Role in Ratatoskr

| Term | Meaning |
|---|---|
| Site | An independent UI in its own repository, named by a site id. |
| Site manifest | `ratatoskr-site.toml` of the site repository: the site id, the hostname, the publications and the requested capabilities. The owner approves it in Platform. |
| Publication | A filter over one vault connection: folders, tags and the frontmatter key `publish`. |
| Release | An immutable snapshot that the release builder makes from the publications of a site, named by the digest of its content. |

Sites owns the site registry, the submitted manifests, a read projection of the approvals, the
publications, the releases and the Content Delivery API. It does not own the approval (Platform),
the vault connections and their files (`ratatoskr-notes`), or the owner's knowledge (Knowledge).

The design is in `ratatoskr-workspace`: ADR-0011, `docs/NOTES_AND_SITES.md`, and the change
`openspec/changes/add-site-publications/`.

## Interfaces

| Interface | Address | Reached by |
|---|---|---|
| Domain API `/v1/sites` | `127.0.0.1:8100` (`RATATOSKR__API__LISTEN_ADDRESS`) | Edge only |
| Operator plane `/health/live`, `/health/ready`, `/metrics`, `/version` | `127.0.0.1:9472` (`RATATOSKR__ADMIN__BIND`) | The host only |
| Database | schema `sites`, role `ratatoskr_sites`, database `ratatoskr` | This service only |
| Note bytes | `/mnt/nvme/ratatoskr/blobs/ratatoskr-notes`, read-only, through the group `ratatoskr-notes-blobs` | This service reads; only `ratatoskr-notes` writes |
| Release pages and assets | `/mnt/nvme/ratatoskr/blobs/ratatoskr-sites` | This service only |

The domain API:

| Route | Class at Edge | Answer |
|---|---|---|
| `GET /v1/sites` | management | The sites, at most 100 |
| `GET`, `PUT /v1/sites/{site}/manifest` | management | The active manifest; a submission stores one |
| `GET /v1/sites/{site}/releases` | management | The current release id, the releases and the failed builds, at most 100 each |
| `GET /v1/sites/{site}/publications`, `GET`, `PUT`, `DELETE .../publications/{publication}` | management | The publications of the site |
| `GET`, `HEAD /v1/sites/{site}/content/*` | site-origin, `public` | The Content Delivery API, `openapi/content-delivery.json` |
| `GET /internal/sites/{site}/manifest` | none; loopback only | The active manifest, for Edge's approval |

Every route outside `content/` refuses Edge's anonymous claim. The owner guide is
[docs/OWNER_GUIDE.md](docs/OWNER_GUIDE.md).

## The HTML of a page

The `html` of a page (`sites/page.v1`) passes a closed allowlist in this service
(`crates/release/src/sanitise.rs`). A site that inserts it should still set these headers on its
own pages, because the allowlist is one line of defence and not the only one:

- `Content-Security-Policy` with `img-src 'self' https:` and no `script-src` that allows inline
  script. A page image is either an asset of the release (`assets/<sha256>`, which the site serves
  from its own origin through the gateway) or an `https:` URL; the allowlist removes the source of
  any other image.
- `Referrer-Policy: no-referrer` or `strict-origin`, so an `https:` image or link of a note does not
  learn the path of the page that shows it.

What the allowlist does to markup that a note writes:

- An element outside the allowlist loses its tags and keeps its text. An attribute outside the
  allowlist is removed, also an attribute that hides content (`hidden`, `style`, `aria-hidden`), so
  text that the note hid with raw HTML becomes visible on the site. Do not hide text in a published
  note with HTML; put it in a `%%` comment or in a note that the publication does not select.
- `script`, `style`, `iframe`, `object`, `template`, `form`, `svg`, `math` and their kin are removed
  with their content.
- At most 256 elements are open at once, and a page whose HTML is over 1 MiB fails the build with
  `sites.page_too_large`.

## Project status

- [x] Service skeleton: strict configuration, logging, operator plane, embedded schema, CI gate.
- [x] Deployment profile, database role and the read group of the note bytes (tasks 5.2 to 5.7).
- [x] Verified reads of note bytes (tasks 5.8 and 5.9): `notes_blobs::NotesBlobReader`.
- [x] Selection, slugs, privacy filter and sanitised HTML (tasks 5.10 to 5.17).
- [x] Releases, the release id and the current-release switch with its event (tasks 5.18 to 5.21).
- [x] Build triggers and the suspension of a disconnected vault (tasks 5.22 to 5.25).
- [x] Manifest, effective set, route classes, delivery and search (tasks 5.26 to 5.35).
- [x] Owner guide (task 5.36).
- [x] The HTTP client of the notes internal index route (workspace tasks 4.1 to 4.5).

## Development

See [DEVELOPMENT.md](DEVELOPMENT.md). The local development database is a convenience of
`compose.yaml`; on the host the service uses the shared database `ratatoskr`.

## Licence

BSD-3-Clause. See [LICENSE](LICENSE).
