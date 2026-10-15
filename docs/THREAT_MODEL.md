# Sites threat model

## Assets

- The content of every connected vault that the service can read: the note bytes in the notes blob
  root and the index that `ratatoskr-notes` serves.
- The notes that a publication does not select. They must never reach a release.
- The releases, their pages, assets and search index in the sites blob root and the `sites` schema.
- The nkey seed `/etc/ratatoskr/sites.nkey`.

## Trust boundaries

| Boundary | Rule |
|---|---|
| Visitor to Edge to Sites | Edge mints an anonymous claim for the `public` route class. Sites serves only the current release of an approved site, and refuses an anonymous claim outside `content/`. |
| Edge to Sites | Sites listens on loopback only and trusts only the claims that Edge mints. |
| Sites to Notes | Sites reads note bytes read-only through the read group `ratatoskr-notes-blobs`, and verifies each `BlobRef` before it uses the bytes. Only Notes writes. |
| Platform to Sites | Platform holds the only approval. Sites keeps a projection of the approval event and changes it from no other input. |

## Threats and controls

| Threat | Control |
|---|---|
| An unselected note leaks into a release | One privacy filter on the parsed tree before every renderer; a canary test over every byte of the release. |
| A link reveals the title or path of an unselected note | A link to an unselected note renders exactly like a link to a missing note. |
| A note carries a script | The HTML body passes a closed element and attribute allowlist. |
| Changed or foreign bytes are read as a note | The owner, the algorithm, the digest, the length and a fresh hash of the bytes are verified before use. |
| A response shows whether a site exists | An unknown, unapproved or withdrawn site takes one code path and gives one answer. |
| A widened manifest takes effect without the owner | The effective set is the approved manifest intersected with the active one. |

## Residual risk

A shared cache can keep withdrawn content for its cache lifetime: 60 seconds for pages and 3600
seconds for assets (workspace spec `site-content-delivery`).
