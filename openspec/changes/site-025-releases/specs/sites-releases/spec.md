## ADDED Requirements

### Requirement: A frontmatter slug outside the grammar fails the build

When the frontmatter `slug` of a selected note is not a string of the grammar of `Slug`, the build
SHALL fail with the class `sites.slug_invalid`, SHALL name the file in the operation detail, and
SHALL make no release.

#### Scenario: A slug with a space

- **WHEN** a selected note has the frontmatter `slug: "Not A Slug"`
- **THEN** the build fails with `sites.slug_invalid` and the detail names the note, verified by `crates/publications/tests/slug.rs::frontmatter_slug_outside_the_grammar_fails`

### Requirement: The release id is the SHA-256 of the RFC 8785 form of the release document

The release id SHALL be the lower-case hex SHA-256 of the RFC 8785 serialisation of the
`sites/release.v1` document of the release.

#### Scenario: The reported id names the stored document

- **WHEN** a build makes a release
- **THEN** the id that the build reports is the SHA-256 of the stored document bytes, and those bytes are the RFC 8785 form of the document, verified by `services/sites/tests/release_build.rs::build_reports_its_release_id`

### Requirement: A page read names its release

A page SHALL be read with the release id of the release document that the reader fetched, and
SHALL be given only while that release is the current release of the site.

#### Scenario: A reader never sees a mixed release

- **WHEN** a reader fetches the release document and then each page it lists while a new release becomes current
- **THEN** every page answer belongs to the release id that the reader fetched or is not found, verified by `services/sites/tests/release_switch.rs::readers_see_one_release_or_the_other`
