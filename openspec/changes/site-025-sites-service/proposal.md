## Why

The workspace change `add-site-publications` (SITE-025) creates this repository. Before the sites
service can build a release, it must run on the host with its own deployment profile, its own
database role, and read access to the note bytes of `ratatoskr-notes`. The cross-repository rules are
the workspace specs `postgres-provisioning` (the role `ratatoskr_sites`), `fleet-bus-topology` (the
`SITES` identity, its inbox prefix, the sites upstream 8100 and the operator listener 9472) and
`blob-references` (requirement "A consumer may read a producer's blob root read-only"). This change
implements the part of them that belongs to workspace tasks 5.2 to 5.9, and does not restate them.

## What Changes

- Complete `deploy/systemd/sites.conf.example`, so that the configuration loader reads from it the
  domain listener 8100, the operator listener 9472, the nkey seed of the `SITES` identity and the
  inbox prefix `_INBOX_sites`.
- Complete `deploy/postgres/role.sql`, so that it creates `ratatoskr_sites` without a password and
  without privileges, grants `connect` and `create` on the database `ratatoskr`, and revokes all on
  the schema `public`.
- Add the user `ratatoskr-sites` to the read group `ratatoskr-notes-blobs` in
  `deploy/sysusers.d/ratatoskr-sites.conf`. OBS-023 creates the group, the setgid root and the
  umask; this change only joins the group.
- Add the reader of the notes blob root. It reads note bytes read-only and verifies the owner, the
  digest algorithm, the digest, the length and a fresh hash before it returns the bytes. A failure
  has a stable class.
- Later, in the same workspace change: selection, slugs, the privacy filter, releases, the build
  triggers, the manifest, the route classes, delivery, search and the owner guide (workspace tasks
  5.10 to 5.36). Each gets its own local change.

## Capabilities

### New Capabilities

- `sites-deployment`: the deployment profile of the service, its database role, its membership in
  the read group of the note bytes, and the verified read of a note blob.

### Modified Capabilities

None.

## Impact

`deploy/systemd/sites.conf.example`, `deploy/postgres/role.sql`,
`deploy/sysusers.d/ratatoskr-sites.conf`, `services/sites` (the reader of the notes blob root). The
workspace adds the ports 8100 and 9472 to its port table and enrols the service in its deploy role
(workspace task 9.6).
