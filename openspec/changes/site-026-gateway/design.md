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

The gateway reads the tree as a member of the group `ratatoskr-sites-bundles`, which the host role
gives the setgid root (`2750`). A directory made in a setgid directory gets its group and the setgid
bit, so the service needs no `chmod`: its unit has `UMask=0027`, which makes a deploy directory
`2750` and a file `0640`. A `chmod` that sets the setgid bit would not work, because the service
user is not a member of the group and the kernel clears the bit.

### 5. Activation needs no download

Activation reads nothing but the site directory, and the store has no HTTP client. The rollback test
asserts the links and that the stored deploy is the same directory as before.

### 6. Retention

After an activation, the store keeps the current and the previous deploy, every deploy stored after
the last activation (the time `current` was last replaced), and the newest others up to the
retention count, by the time that each deploy directory was stored, and removes the rest. A deploy
uploaded ahead therefore survives a rollback, also with a retention of two (review M4). Uploads that
are never activated are bounded only by the bundle size limit and the owner's deploy device.

### 7. The deploy routes check the device claim again

Edge forwards the deploy routes for the owner's primary session and for a `site_deployer` device of
the site. The routes refuse, with `sites.request.forbidden`, a request whose claims name a device
that is not a `site_deployer` of the site of the route. The archive streams into the receipt with a
limit of the bundle limit and 16 MiB of tar headers; a declared length above it is refused before
the body is read.

### 8. No repository binds a bundle to a site

The bundle manifest names the repository that built it, but the site record does not: the site
manifest `sites/site-manifest.v1`, the approval event, Platform's `identity.site_approvals` and
`sites.sites` hold no repository. So the receipt cannot refuse a bundle of another repository whose
manifest names the site, and this change does not invent a repository field. The gap is reported to
the workspace.

### 9. `site.json` holds the site, the approval state and the hostnames

`site.json` is `{"site_id", "approved", "hostnames"}`, written from the approval projection after
the event is applied, so an older event that the projection ignores rewrites the newer state. A site
that is not approved lists no hostname. The workspace design also names the retention; the gateway
does not read it, so the file leaves it out, and the retention stays a setting of the sites service.
The approval projection holds one hostname, lower-cased in the file.

### 10. The gateway rescans every two seconds and uses no inotify

Workspace task 6.2 names inotify plus a two-second rescan. The spec requires that a rewritten
projection takes effect within five seconds, and the rescan alone does that: a change waits at most
two seconds and a lost event loses nothing. inotify would add the `notify` crate and its tree, and a
second code path that the tests cannot hold to the bound. So the gateway rescans only. A rescan
reads one small `site.json` per site.

### 11. Codes of the gateway that the shared names do not list

`docs/NOTES_AND_SITES.md` names `sites.gateway.unknown_host`, `sites.gateway.no_deploy` and
`sites.gateway.route_not_forwarded`. The gateway also answers a missing file with
`sites.page_not_found`, a write method on a static path with `sites.request.method_not_allowed`,
and proposes `sites.gateway.rate_limited` (429) and `sites.gateway.body_too_large` (413).

### 12. Every folder path may serve its `index.html`

The spec names the folder index for `/.ratatoskr/` only. A static site generator such as Astro
writes `about/index.html` for `/about/`, so the gateway resolves a folder path to its `index.html`
for every path, before the previous deploy and the entry fallback. Only a listed file is served, so
this widens nothing outside the manifest. The gateway keeps the manifests of the deploys that
requests resolved, at most 64, because a stored deploy never changes.

### 13. The forward list and its headers

The gateway forwards these routes below `/v1/sites/<site>/` of the site of the host: `content/*`,
`events`, `sessions`, `releases`, `drafts/*`, `edits`, `conflicts/*`, `library` and
`knowledge/search` (SITE-025 and SITE-027 site-origin, sign-in and release routes). Management and
deploy routes are never forwarded from a site origin. The gateway removes the inbound `Host` and
`Accept-Encoding` too, so Edge never compresses a forwarded answer and an event stream reaches the
client unchanged. A forwarded answer that sends nothing for one minute, four times Edge's keep-alive
interval, ends with an error.
