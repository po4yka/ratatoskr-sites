# Sites requirements

The requirements that more than one repository can see are specs in the `ratatoskr-workspace`
store. This file lists them and does not restate them.

| Spec (workspace change `add-site-publications`) | What it requires of Sites |
|---|---|
| `site-publications` | The publication, its selection precedence, the release and its id, the atomic switch, the release event, the build triggers, the first explicit release, the coalescing, the suspension on disconnect. |
| `site-release-privacy` | No data from an unselected file; links, embeds, comments, frontmatter and attachments; the HTML allowlist; a full rebuild on every build. |
| `site-manifest` | The manifest shape, its storage by digest, the approval projection, the effective set and the one answer for every absence. |
| `site-route-classes` | The refusal of an anonymous claim outside `content/`. |
| `site-content-delivery` | The four delivery routes, the cache lifetimes, the conditional requests and the search over the current release. |
| `blob-references` | The verification of every `BlobRef` before the bytes of a note are used, and no write to the notes blob root. |
| `postgres-provisioning` | The role `ratatoskr_sites` and the database URL of the deploy example. |
| `fleet-bus-topology` | The `SITES` identity, its four durables, its inbox prefix, the sites upstream 8100 and the operator listener 9472. |

Requirements that only this repository can see grow as specs in `openspec/specs/` from the changes
that need them.
