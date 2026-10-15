## Why

Workspace groups 4 to 7 of `add-site-gateway-hosting` (SITE-026) let an approved site be served on
its own hostname from a static bundle that CI built: the sites service receives and stores the
bundle, activates a deploy, and writes the gateway projection; a second binary, the sites gateway,
serves the current deploy and forwards the site's own API calls to Edge. The cross-repository rules
are the workspace specs `site-bundles` and `site-gateway`. Platform already forwards the deploy
routes (SITE-026 group 8). This change implements groups 4 to 7 as the workspace tasks word them,
and records only the decisions that are local to this repository.

## What Changes

- Add the crate `crates/deploys`: the streaming receipt of a bundle archive, the deploy store, the
  activation of `current` and `previous` by `rename(2)`, and the retention of old deploys.
- Add the deploy routes `POST /v1/sites/{site}/deploys` and
  `POST /v1/sites/{site}/deploys/{commit}/activate` to the domain API of the sites service.
- Write the gateway projection `site.json` of a site from the approval projection on every approval
  event.
- Add the binary `services/sites-gateway` (`ratatoskr-sites-gateway`): host routing from the
  projection, static serving with the cache and security headers, the reserved path `/.ratatoskr/`,
  forwarding under `/_r/` to Edge, the token bucket and the body cap, and its operator listener.
- Add `deploy/systemd/ratatoskr-sites-gateway.service` and `deploy/systemd/sites-gateway.conf.example`.

## Capabilities

### New Capabilities

- `sites-gateway`: the local decisions of the deploy store, the projection and the gateway.

### Modified Capabilities

None.

## Impact

`crates/deploys` (new), `services/sites`, `services/sites-gateway` (new), `deploy/`, `Dockerfile`,
`.github/workflows/ci.yml` (the export stage holds both binaries). New dependency: `tar` (no
default features) and its dependency `filetime`.
