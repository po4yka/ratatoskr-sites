## Context

The workspace design `add-site-gateway-hosting` D2 to D8 decides the cross-repository shape. This
file records only the decisions that are local to this repository.

## Decisions

### 1. The layout of a deploy

A deploy is `bundles/<commit>/` with the manifest `ratatoskr-bundle.json` and the listed files under
`files/`. The gateway reads the manifest beside the files and serves only a path that the manifest
lists, so the manifest itself is never served. A manifest that lists `ratatoskr-bundle.json` is
refused with `sites.bundle.reserved_path`.

### 2. The receipt streams in any order and leaves nothing on a refusal

The workspace contract does not fix the order of the archive, so the manifest can come after the
files. The receipt checks the path rules of each entry before it writes a byte of that entry, hashes
each file while it writes it into a staging directory of the site, and checks the manifest when the
archive ends. It renames the staging directory to `bundles/<commit>` only after every file passed.
On any refusal it removes the staging directory, so no file remains under the site directory. A
leading `./` of an entry name is removed; a directory entry is only checked. A symbolic link, a hard
link and a device entry are `sites.bundle.unsafe_path`; so is a path that is not in NFC or that the
archive holds twice.

### 3. Codes that the shared names do not list

`docs/NOTES_AND_SITES.md` names five bundle codes. The receipt also needs a code for a bundle over the
size limit and for an activation of a commit that the site does not hold. This repository proposes
`sites.bundle.too_large` (413) and `sites.bundle.not_found` (404). An archive that is not a tar, or
that has no valid `ratatoskr-bundle.json`, is `sites.request.invalid` (400). A listed file that the
archive lacks is `sites.bundle.digest_mismatch` with its path.

### 4. Modes in the tree

The sites unit runs with `UMask=0077`. The gateway reads the tree as a member of the group
`ratatoskr-sites-bundles`, which the host role gives the setgid root. So the sites service sets the
mode of what it writes: `2750` for a directory and `0640` for a file, after it creates it.

### 5. Activation needs no download

Activation reads nothing but the site directory, and the store has no HTTP client. The rollback test
asserts the links and that the stored deploy is the same directory as before.

### 6. Retention

After an activation, the store keeps the current and the previous deploy and the newest others up to
the retention count, by the time that each deploy directory was stored, and removes the rest.
