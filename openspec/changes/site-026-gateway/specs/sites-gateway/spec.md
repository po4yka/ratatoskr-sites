## ADDED Requirements

### Requirement: A refused bundle leaves nothing in the site directory

When the receipt refuses a bundle, no file of that bundle SHALL remain under the directory of the
site, whatever the order of the entries in the archive.

#### Scenario: An unsafe entry after the safe ones

- **WHEN** an archive holds the listed files and then an entry `../../etc/passwd` or a symbolic link
- **THEN** the receipt answers `sites.bundle.unsafe_path` and the site directory holds no file, verified by `crates/deploys/tests/receipt.rs::an_unsafe_path_writes_nothing`
