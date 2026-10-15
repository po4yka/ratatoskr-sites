# Selection fixture vault

`vault/` holds the files of one vault connection. `index.json` holds what the notes internal index
route (`GET /internal/connections/{connection}/files`) gives for each file, without the revision:
the test loader computes the revision `BlobRef` from the bytes in `vault/`. The two publications
and their sites are in `crates/publications/tests/selection.rs`.
