## Context

The workspace design `add-site-publications` D1, D3 and the table "Storage" decide the
cross-repository shape. This file records only the decisions that are local to this repository.

## Decisions

### 1. The test reads the shipped example through the real loader

`services/sites/tests/deployment_profile.rs` parses `deploy/systemd/sites.conf.example` as a systemd
`EnvironmentFile` and gives its entries to `config::load_from`, the same function that reads the
process environment. So the example cannot name a key that the loader refuses, and a key that the
example leaves out is found by the test, not on the host. The test asserts each key in the file
before it asserts the loaded value, because a compiled default can hide a key that the file does not
set.

### 2. The deploy-file tests read the shipped files as text

`deploy/postgres/role.sql` runs on the host through `psql` (it uses `\connect`), and
`deploy/sysusers.d/ratatoskr-sites.conf` runs through `systemd-sysusers`. Neither runs in CI. The
tests in `services/sites/tests/deploy_files.rs` read both files and assert the exact statements and
lines that the workspace specs require. Before task 2.2 is ticked, the role file is also run once
against a PostgreSQL 17 container.

### 3. The sysusers file joins the group and does not create it

The group `ratatoskr-notes-blobs` belongs to `ratatoskr-notes` and the deploy role of the workspace
creates it (OBS-023). A `g` line here would create the group on a host where notes is not installed,
and the root would then have a group that notes does not expect. So the file holds only the line
`m ratatoskr-sites ratatoskr-notes-blobs`, and a test fails when the file creates a group of that
name.

### 4. The reader checks everything before it reads, and hashes what it read

`NotesBlobReader::read` takes a `BlobRef` and the notes blob root. In this order it:

1. refuses an owner other than `ratatoskr-notes` with `blob_owner`, and reads nothing;
2. refuses an algorithm other than SHA-256 with `blob_algorithm`;
3. reads the metadata of `<root>/sha256/<2 hex>/<62 hex>`: a missing file is `blob_missing`, a path
   that is not a regular file is `blob_unreadable`, and a size other than `length_bytes` is
   `blob_length`, so a large file is never read into memory;
4. reads the file and hashes what it read: a different length is `blob_length`, and a different
   digest is `blob_digest`.

The reader opens the file read-only and never writes, renames or deletes under the root.
