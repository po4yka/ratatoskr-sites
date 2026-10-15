# Owner guide

This guide is for the owner of a Ratatoskr installation who publishes notes of a connected Obsidian
vault on a site. It tells you which notes a site can show, when a new release is made, how a site
gets its permissions, and how long a cache can keep public content.

The rules come from the workspace change `add-site-publications` (SITE-025): the specs
`site-publications`, `site-release-privacy`, `site-manifest` and `site-content-delivery`. If this
guide and a spec do not agree, the spec is correct.

## One owner

The sites service serves one owner: the owner of the Ratatoskr installation. Edge lets only a
primary session with the `platform.owner` grant reach the management routes, and the service does
not scope sites by owner. Every management request can read and change every site.

One owner also owns every vault connection of the installation. Neither the sites service nor the
notes index route checks who owns a connection: a publication can name any connection id, and the
build reads that connection. A connection that notes does not know, or that failed or was removed,
fails the build with `sites.connection_not_found`.

## Terms

| Term | Meaning |
|---|---|
| Site | Your own UI in its own repository. A site has a site id, for example `docs`. |
| Publication | A filter over one vault connection: folders, tags and the frontmatter key `publish`. A publication belongs to one site. |
| Release | An immutable copy of the selected notes of a site, after the privacy rules. Its id is the SHA-256 of its content. |
| Manifest | The file `ratatoskr-site.toml` of the site repository. |
| Effective set | The capabilities, the publications and the hostname that are in force for a site now. |

## Which notes a publication selects

A vault is private by default. A note goes into a release only when a publication selects it.

A publication decides for each Markdown file with this precedence. The first rule that applies
decides:

1. A file in an excluded folder is not selected.
2. When the publication names included folders, a file outside them is not selected.
3. A file with `publish: false` is not selected.
4. A file with a `publish` list that does not name the site is not selected.
5. A file with `publish: true`, or with a `publish` list that names the site, is selected.
6. A file with an included tag is selected.
7. In the mode `all`, every other file is selected. In the mode `flagged`, no other file is
   selected.

Thus an excluded folder always wins, and `publish: true` works only inside the included folders. A
note that you marked for Obsidian Publish in a private folder does not go to a site.

These files are never selected by a folder or a tag:

- a file under `.obsidian/`;
- a file that the vault connection does not index;
- an attachment (an image or another file that is not Markdown). An attachment goes into a release
  only when a published page uses it.

### The `publish` site list

When one vault feeds more than one site, you can name the sites in the frontmatter of a note:

```yaml
---
publish: [docs, garden]
---
```

The note can then go only to the sites `docs` and `garden`, and only when a publication of that site
selects it by the rules above.

### Slugs

The address of a page is its slug. The frontmatter key `slug` sets it. Without that key, the slug is
the vault path without `.md`, in lower case, with each run of characters other than ASCII letters,
digits and `/` replaced by one `-`. When two selected notes get the same slug, the build fails with
`sites.slug_collision` and the current release stays. Give one of the notes a `slug` key.

### What the privacy rules remove

Before anything is rendered, the build removes from each selected note: `%% comments %%`, the
frontmatter fields that are not on the allowlist of the publication, and every link, embed or title
that comes from a note that the publication does not select. A link to a note that is not selected
becomes the text that you wrote, as for a link to a note that does not exist.

## When a release is made

### The first release is explicit

The first release that contains a publication is made only when you request a build in the web
client. This applies in each release mode. A vault that you used with Obsidian Publish can already
have `publish: true` on many notes; nothing goes to a site until you request the first release.

### The release modes

| Mode | What makes a new release |
|---|---|
| `automatic` (the default) | A change of a note that the publication selects now, or selected in the current release, requests a build. You can also request a build. |
| `manual` | Only a build that you request. |

The service makes one build of a site at a time. The changes that arrive while a build runs make
one more build after it, not one build for each change. A build whose content did not change makes
no new release. A change of an attachment that a release publishes also requests a build.

### A disconnected vault

When you disconnect a vault, every publication over it becomes `suspended`. A build of a site that
reads a suspended publication fails with `sites.publication_suspended`. The current release of the
site stays current until you withdraw the approval of the site or connect the vault again and save
the publication with the new connection.

## The manifest and its approval

The site repository holds `ratatoskr-site.toml`:

```toml
site_id = "docs"
hostname = "docs.example.test"
publications = ["handbook"]
capabilities = ["pages.read", "assets.read", "search.publication"]
```

A manifest with another key, an unknown capability or a second hostname is refused with
`sites.manifest_invalid`, and the refusal names the key or the value.

You submit the manifest in the web client. The last valid submission is the active manifest. A
submission alone grants nothing: you approve the active manifest in the web client, and Platform
keeps the approval. The approval names the SHA-256 of the manifest. You can withdraw the approval
at any time.

### Capabilities

| Capability | Who can use it | What it allows |
|---|---|---|
| `pages.read` | Anyone | Read the pages and the navigation of the current release |
| `assets.read` | Anyone | Read the assets of the current release |
| `search.publication` | Anyone | Search the current release only |
| `live.release` | Anyone | Receive release events (SITE-027) |
| `draft.read` | Owner session | Read changes that are not published yet (SITE-027) |
| `notes.edit` | Owner session | Write the source notes of the publications (SITE-027) |
| `library.read` | Owner session | Read your knowledge library (SITE-027) |
| `knowledge.search` | Owner session | Search your knowledge (SITE-027) |

The last four grant nothing until SITE-027 adds the owner session.

### The effective set

The effective set of a site is what both the approved manifest and the active manifest allow:

- its capabilities are those of the approved manifest that the active manifest also declares;
- its publications are those that both manifests name;
- its hostname is the approved one.

So:

- an added capability, publication or hostname waits for a new approval;
- a removed capability or publication stops at once, with no approval;
- with no approval, or after a withdrawal, the set is empty and the site serves nothing.

A release holds only the publications of the effective set. When you remove a publication from the
manifest, or delete it in the web client, its pages, its assets and its search hits stop at once;
the release document no longer lists them. The service also requests a build, in each release
mode, so the pages that stay lose their links to the removed pages. That build reads the notes as
the current release holds them, not as they are now, so it publishes no change that you did not
release. A change of a publication that already has a release requests the same build.

## What a visitor reads

A site reads its release through the Content Delivery API, with no credential:

| Route | Capability |
|---|---|
| `/v1/sites/{site}/content/release` | `pages.read` |
| `/v1/sites/{site}/content/pages/{slug}` | `pages.read` |
| `/v1/sites/{site}/content/assets/{sha256}` | `assets.read` |
| `/v1/sites/{site}/content/search?q=...` | `search.publication` |

Only the current release is served. A site id that does not exist, a site without an approval, a
withdrawn site and a capability outside the effective set all get the same answer: 404
`sites.site_not_found`. A visitor cannot tell them apart.

Search finds only the pages of the current release. A query has 1 to 256 characters.

An asset whose media type is HTML, SVG or XML is served as a download, so that a browser never runs
its script on the origin of the site.

A build that fails for a passing reason, for example because the notes service does not answer, is
tried again after 5 seconds, at most three times, before it is reported as failed.

## Cache bounds after a withdrawal

Public answers can be kept by a shared cache in front of the host. The service limits how long:

| Answer | Longest cache lifetime |
|---|---|
| The release, a page, a search | 60 seconds |
| An asset | 3600 seconds |

Thus, after you withdraw the approval of a site, or after a page leaves the current release:

- the service stops serving the content at once;
- a shared cache can still give a page, the release or a search for at most 60 seconds;
- a shared cache can still give an asset for at most 3600 seconds (one hour).

All public content of a withdrawn site stops within 3600 seconds, also in a shared cache. If an
asset must go away sooner, remove it from the cache of your CDN by hand.
