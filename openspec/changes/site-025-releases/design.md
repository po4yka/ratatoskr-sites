## Context

The workspace design `add-site-publications` D2, D3, D4, D5 and D6a decides the cross-repository
shape. This file records only the decisions that are local to this repository.

## Decisions

### 1. The index of a connection is a trait with a stub

The notes route `GET /internal/connections/{connection}/files` does not exist yet. The build reads
the files of a connection through the trait `NotesIndex`. The shape of a file is
`ratatoskr_sites_publications::IndexedFile`: the path, the revision `BlobRef`, the frontmatter, the
tags, the headings, the block ids, and the links and embeds with the target path that the notes
resolver found, or none. A fixture vault is `vault/` and `index.json`; the test loader computes each
revision from the bytes.

### 2. Links are matched by the text that the note writes

The parser finds each wikilink and Markdown link in the filtered source. The filter takes the target
from the index entry of the same file whose `link` is the same text. So the release uses the notes
resolver and never resolves a name itself (design D3), and a link that the comment pass removed has
no edge, also when the index lists it.

### 3. The slug of a frontmatter key outside the grammar

A frontmatter `slug` that is not a string of the `Slug` grammar fails the build with the class
`sites.slug_invalid`, and the detail names the file. A path whose derived slug is outside the
grammar fails the same way. The workspace spec has no class for this; the class is proposed here.

### 4. The release id is the SHA-256 of the RFC 8785 form

The release document is serialised with `serde_jcs`, and the release id is the SHA-256 of those
bytes. A page document is stored as the `serde_json` bytes of the typed `Page`; its SHA-256 is the
`page_sha256` of the release. Both are deterministic for the same content.

### 5. A note is read with a bound

The build reads each selected note and each used attachment through `NotesBlobReader` with a bound
of 64 MiB, the default `RATATOSKR__FILES__MAX_FILE_BYTES` of `ratatoskr-notes`: notes indexes no
larger file. A larger reference fails the build with `blob_length` before the file is read.

### 6. A page read names its release

A reader fetches the current release document, then each page with the release id of that
document. A page is given only while that release is current; otherwise the answer is not found.
So a reader never gets a page of another release than the one it fetched.

### 7. A link to an attachment is an embed of the asset

`PageNode::Link` takes only an `https`, `http` or `mailto` URL. A link to an attachment that the
release publishes is therefore written as an `embed` of that asset, and the link text is dropped.
