## 1. Deployment profile (workspace tasks 5.2, 5.3)

- [x] 1.1 RED: add `services/sites/tests/deployment_profile.rs::shipped_example_enables_the_flow`, which loads `deploy/systemd/sites.conf.example` through the configuration loader and asserts `RATATOSKR__API__LISTEN_ADDRESS=127.0.0.1:8100`, the operator listener `127.0.0.1:9472`, `RATATOSKR__BUS__NKEY_SEED_PATH=/etc/ratatoskr/sites.nkey` and the inbox prefix `_INBOX_sites`. Run it and confirm that the listen-address assertion fails because the example does not set the key.
- [x] 1.2 GREEN: complete the example and the configuration loader. Verify 1.1.

## 2. Database role (workspace tasks 5.4, 5.5)

- [x] 2.1 RED: add `services/sites/tests/deploy_files.rs::role_file_creates_the_unprivileged_sites_role`, which asserts the statements of `deploy/postgres/role.sql` and the database URL of the example. Run it and confirm that the `nobypassrls` assertion fails because the scaffolded file creates only the role.
- [x] 2.2 GREEN: complete `deploy/postgres/role.sql` and the database URL of the example. Verify 2.1.

## 3. Read group membership (workspace tasks 5.6, 5.7)

- [x] 3.1 RED: add `services/sites/tests/deploy_files.rs::sysusers_adds_sites_to_the_notes_blob_group`, which asserts the line `m ratatoskr-sites ratatoskr-notes-blobs` and no group of that name. Run it and confirm that the line assertion fails because the scaffolded file has no membership line.
- [x] 3.2 GREEN: add the membership line. Verify 3.1.

## 4. Verified note reads (workspace tasks 5.8, 5.9)

- [x] 4.1 RED: add `services/sites/tests/notes_blob_read.rs` with `notes_blob_ref_is_verified_before_use` and `foreign_owner_ref_is_refused`, and a stub reader that returns the bytes unchecked. Run them and confirm that the class assertion of the first fails because the reader returns the bytes unchecked.
- [x] 4.2 GREEN: read note bytes read-only from the notes root and verify the owner, the algorithm, the digest, the length and a fresh hash, as `blob-references` requires. Verify 4.1.

## 5. The unit names the read group (review of workspace task 5.7)

- [x] 5.1 RED: add `services/sites/tests/deploy_files.rs::unit_names_the_notes_blob_group`, which asserts that `SupplementaryGroups=` of `deploy/systemd/ratatoskr-sites.service` names `ratatoskr-notes-blobs` and `ratatoskr`. Run it and confirm that the group assertion fails because the unit names only `ratatoskr`.
- [x] 5.2 GREEN: name `ratatoskr-notes-blobs` in `SupplementaryGroups=`, as the Knowledge unit does. Verify 5.1.

## 6. Pin the other classes of the note blob reader (review of workspace task 5.9)

- [ ] 6.1 Add `services/sites/tests/notes_blob_read.rs::missing_blob_is_blob_missing` and `blob_of_another_length_is_blob_length`. No failing test first: the code exists, so these are pin tests; each is proven by a mutation of the reader that makes it fail. `blob_algorithm` gets no test: `DigestAlgorithm` has only `Sha256`, so no `BlobRef` can name another algorithm.
