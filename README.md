# Ratatoskr Sites

`ratatoskr-sites` is the sites bounded context of Ratatoskr. It selects the notes of a connected
Obsidian vault that the owner publishes, builds an immutable release from them after the privacy
rules, and serves the current release of each approved site through the Content Delivery API. Each
site is an independent UI in its own repository, and reads a release with `@ratatoskr/site-sdk`.

> **Status:** in development. The service selects the notes of each publication, builds an
> immutable release through the privacy filter, makes it current in one transaction and announces
> it. The manifest, the build triggers and the Content Delivery API are planned in the workspace
> change `add-site-publications` (SITE-025), group 5.

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

The domain API has no routes yet.

## Project status

- [x] Service skeleton: strict configuration, logging, operator plane, embedded schema, CI gate.
- [x] Deployment profile, database role and the read group of the note bytes (tasks 5.2 to 5.7).
- [x] Verified reads of note bytes (tasks 5.8 and 5.9): `notes_blobs::NotesBlobReader`.
- [x] Selection, slugs, privacy filter and sanitised HTML (tasks 5.10 to 5.17).
- [x] Releases, the release id and the current-release switch with its event (tasks 5.18 to 5.21).
- [ ] Build triggers and the suspension of a disconnected vault (tasks 5.22 to 5.25).
- [ ] Manifest, effective set, route classes, delivery and search (tasks 5.26 to 5.35).
- [ ] Owner guide (task 5.36).

## Development

See [DEVELOPMENT.md](DEVELOPMENT.md). The local development database is a convenience of
`compose.yaml`; on the host the service uses the shared database `ratatoskr`.

## Licence

BSD-3-Clause. See [LICENSE](LICENSE).
