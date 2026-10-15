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
| `ratatoskr-sites` | `services/sites` | Configuration, the process lifecycle, the operator plane and the domain routes. |

## 3. Storage

| What | Where |
|---|---|
| Database | schema `sites`, role `ratatoskr_sites`, database `ratatoskr` |
| Release pages and assets | `/mnt/nvme/ratatoskr/blobs/ratatoskr-sites/sha256/<2 hex>/<rest>` |
| Note bytes, read-only | `/mnt/nvme/ratatoskr/blobs/ratatoskr-notes/sha256/<2 hex>/<rest>` |
| NATS seed | `/etc/ratatoskr/sites.nkey` |
