# Sites architecture

> Status: target architecture, partly implemented. README.md under Project status lists what
> exists. The cross-repository design is `ratatoskr-workspace` ADR-0011, `docs/NOTES_AND_SITES.md`
> and the change `add-site-publications`, design D1 to D11. This file does not repeat it; it shows
> where each part lives in this repository.

## 1. Components

```
 Edge ──loopback 8100──► ratatoskr-sites ──► PostgreSQL schema `sites`
                             │   │   │
                             │   │   └──► outbox ──► NATS (evt.sites.*, operation reports)
                             │   └──► NATS durables: builds, note changes, disconnections, approvals
                             │
                             ├──read-only──► /mnt/nvme/ratatoskr/blobs/ratatoskr-notes
                             └──read/write─► /mnt/nvme/ratatoskr/blobs/ratatoskr-sites
```

- One process, `ratatoskr-sites`, with two listeners: the domain API on 8100 and the operator plane
  on 9472.
- One schema, `sites`, in the shared database `ratatoskr`. `schema.sql` is the whole schema.
- The service reads note bytes from the notes blob root through the read group
  `ratatoskr-notes-blobs`, and writes the pages and assets of each release to its own blob root.

## 2. Packages

| Package | Path | Holds |
|---|---|---|
| `ratatoskr-sites-persistence` | `crates/persistence` | The pool, the embedded schema, the disposable test database. |
| `ratatoskr-sites-publications` | `crates/publications` | The shape of the notes index, the selection of a publication, the slug of a page. |
| `ratatoskr-sites-deploys` | `crates/deploys` | The receipt of a bundle archive, the deploy store, the activation of a deploy and the retention of old deploys (SITE-026). |
| `ratatoskr-sites-release` | `crates/release` | The comment pass, the parser, the privacy filter, the page tree and HTML renderers, the HTML allowlist and the release document. It reads no file and no database. |
| `ratatoskr-sites` | `services/sites` | Configuration, the process lifecycle, the operator plane, the release build, the release read, the outbox relay and the domain routes. |

The release builder runs in a fixed order: frontmatter and comments are cut from the text, the text
is parsed into one tree, the privacy filter runs on that tree, and only the filtered tree reaches
the renderers and the collectors of links and assets (workspace design D5). The release id is the
SHA-256 of the RFC 8785 form of the release document (design D4).

## 3. Storage

| What | Where |
|---|---|
| Database | schema `sites`, role `ratatoskr_sites`, database `ratatoskr` |
| Release pages and assets | `/mnt/nvme/ratatoskr/blobs/ratatoskr-sites/sha256/<2 hex>/<rest>` |
| Note bytes, read-only | `/mnt/nvme/ratatoskr/blobs/ratatoskr-notes/sha256/<2 hex>/<rest>` |
| NATS seed | `/etc/ratatoskr/sites.nkey` |
| Site bundles and the gateway projection | `/mnt/nvme/ratatoskr/sites/<site id>/`, below |

### 3.1 The sites tree (SITE-026)

The sites service is the only writer of `/mnt/nvme/ratatoskr/sites/`. The sites gateway reads it
through the group `ratatoskr-sites-bundles` and never writes it.

```text
/mnt/nvme/ratatoskr/sites/<site id>/
├── site.json                  the gateway projection: hostnames and approval state
├── bundles/<commit>/          one verified deploy, never changed after it is stored
│   ├── ratatoskr-bundle.json  the bundle manifest
│   └── files/<path>           each file that the manifest lists
├── current  -> bundles/<c2>   the deploy that the gateway serves
├── previous -> bundles/<c1>   the deploy that was current before
└── .staging/<id>/             a receipt that runs; removed on a refusal
```

- A receipt checks the path rules of each entry of the archive before it writes a byte of it,
  hashes each file while it writes it into `.staging/`, checks the manifest at the end, and renames
  the staging directory to `bundles/<commit>` only when every check passed. A deploy is idempotent
  on the site and the commit.
- An activation replaces `current`, then `previous`, each by a new link under a temporary name and
  `rename(2)`. A reader that resolves `current` once sees one complete deploy.
- After an activation the store keeps `current`, `previous` and the newest other deploys up to the
  retention count (default 5), and removes the rest.
- A deploy and an activation need the site to be approved in the approval projection.
