## Context

The workspace design `add-site-owner-editing-and-live` D3, D5 and D6 decide the cross-repository
shape. This file records only the decisions that are local to this repository.

## Decisions

### 1. A `source_ref` is a random name in a table, and the scope is computed on each request

`sites.source_refs` maps a random `source_ref` (`src_` and 32 hex digits of a version 4 UUID) to a
site, a connection and a path. The service mints the name the first time it names the note, and
keeps it: one note of one site has one `source_ref`. The name holds no path, no time and no hash of
a path, so it tells nothing about the vault.

The table says nothing about the scope. Every request decides the scope again: the note is in scope
when a publication of the effective set of the site, in the state `active`, over that connection,
selects the path in the notes index now. So a note that a publication stops selecting is out of
scope at once, and a note that is not yet in a release is in scope.

### 2. The draft routes answer a site owner session of the owner of the site only

The workspace spec says that a `source_ref` appears only in answers to a `site_owner` session. Edge
forwards the draft routes only for such a session. The service checks it again: the draft routes
need the session kind `site_owner`, and the user of the claim must own the site. Another kind is the
unauthenticated error. Another user gets the not-found answer of an unknown site.

### 3. The not-found answer of a note is `sites.page_not_found`

The shared names of `docs/NOTES_AND_SITES.md` have no code for a note. A note outside the scope, an
unknown `source_ref` and an unknown slug all answer 404 `sites.page_not_found`, so the answer does
not show whether the note exists.

### 4. The draft change event names the revision of the event

`evt.sites.draft.changed.v1` carries the revision of the `notes.file.changed.v1` that caused it,
the `source_ref` of the note, and its slug when the current release of the site has a page for it
that a publication of the effective set serves. The event goes out for each site whose effective
publications select the note now, in either release mode. A deleted note is selected by no
publication, so it makes no event.
