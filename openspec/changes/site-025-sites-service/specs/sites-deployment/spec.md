## ADDED Requirements

### Requirement: The shipped example enables the deployment profile

`deploy/systemd/sites.conf.example` SHALL set `RATATOSKR__API__LISTEN_ADDRESS=127.0.0.1:8100`,
`RATATOSKR__ADMIN__BIND=127.0.0.1:9472` and
`RATATOSKR__BUS__NKEY_SEED_PATH=/etc/ratatoskr/sites.nkey`, and the configuration loader SHALL load
it with those values and the inbox prefix `_INBOX_sites`, as the workspace spec `fleet-bus-topology`
requires.

#### Scenario: The example is loaded through the configuration loader

- **WHEN** the test loads `deploy/systemd/sites.conf.example` through `config::load_from`
- **THEN** the domain listener is `127.0.0.1:8100`, the operator listener is `127.0.0.1:9472`, the nkey seed path is `/etc/ratatoskr/sites.nkey` and the inbox prefix is `_INBOX_sites`, verified by `services/sites/tests/deployment_profile.rs::shipped_example_enables_the_flow`

### Requirement: The role file creates an unprivileged sites role

`deploy/postgres/role.sql` SHALL create the role `ratatoskr_sites` without a password, with
`nosuperuser nocreatedb nocreaterole noreplication nobypassrls`, SHALL grant `connect` and `create`
on the database `ratatoskr`, and SHALL revoke all on the schema `public`. The database URL of the
example SHALL be `postgres://ratatoskr_sites:CHANGE-ME@127.0.0.1:5432/ratatoskr`, as the workspace
spec `postgres-provisioning` requires.

#### Scenario: The role file and the example agree

- **WHEN** the test reads `deploy/postgres/role.sql` and the example
- **THEN** the role file holds each statement above and the example holds the database URL above, verified by `services/sites/tests/deploy_files.rs::role_file_creates_the_unprivileged_sites_role`

### Requirement: The sites user joins the read group of the note bytes

`deploy/sysusers.d/ratatoskr-sites.conf` SHALL hold the line `m ratatoskr-sites
ratatoskr-notes-blobs` and SHALL NOT create a group or a user with the name
`ratatoskr-notes-blobs`.

#### Scenario: The sysusers file joins the group

- **WHEN** the test reads `deploy/sysusers.d/ratatoskr-sites.conf`
- **THEN** it holds the membership line and no `g` or `u` line for `ratatoskr-notes-blobs`, verified by `services/sites/tests/deploy_files.rs::sysusers_adds_sites_to_the_notes_blob_group`

#### Scenario: The unit names the read group

- **WHEN** the test reads `deploy/systemd/ratatoskr-sites.service`
- **THEN** its `SupplementaryGroups=` names `ratatoskr-notes-blobs` and `ratatoskr`, verified by `services/sites/tests/deploy_files.rs::unit_names_the_notes_blob_group`

### Requirement: A note blob is verified before it is used

The reader of the notes blob root SHALL verify the owner, the digest algorithm, the digest, the
length and a fresh hash of the bytes of a `BlobRef` before it returns them, as the workspace spec
`blob-references` requires. A reference of another owner SHALL fail with the class `blob_owner` and
SHALL read nothing. Bytes that do not hash to the digest of the reference SHALL fail with the class
`blob_digest`.

#### Scenario: The bytes do not match the digest

- **WHEN** the file at the path of a reference holds bytes of the right length that do not hash to its digest
- **THEN** the read fails with the class `blob_digest` and returns no bytes, verified by `services/sites/tests/notes_blob_read.rs::notes_blob_ref_is_verified_before_use`

#### Scenario: The reference names another owner

- **WHEN** the reader gets a `BlobRef` whose owner is not `ratatoskr-notes`
- **THEN** the read fails with the class `blob_owner` and reads nothing under the root, verified by `services/sites/tests/notes_blob_read.rs::foreign_owner_ref_is_refused`

#### Scenario: No file is at the path of the digest

- **WHEN** the reader gets a reference whose file does not exist under the root
- **THEN** the read fails with the class `blob_missing`, verified by `services/sites/tests/notes_blob_read.rs::missing_blob_is_blob_missing`

#### Scenario: The file has another length

- **WHEN** the file at the path of a reference is longer or shorter than `length_bytes`
- **THEN** the read fails with the class `blob_length`, verified by `services/sites/tests/notes_blob_read.rs::blob_of_another_length_is_blob_length`
