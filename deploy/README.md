# Deploy files of ratatoskr-sites

This repository ships two units, one per binary. The playbook of `ratatoskr-workspace` installs
them; the host is frozen, so nothing here runs on it until the owner lifts the freeze
(`ratatoskr-workspace/docs/DEPLOYMENT_TARGET.md`).

| File | Purpose |
|---|---|
| `systemd/ratatoskr-sites.service`, `systemd/sites.conf.example` | The sites service: the domain API on `127.0.0.1:8100`, the operator plane on `9472`. It is the only writer of the sites tree `/mnt/nvme/ratatoskr/sites` (`RATATOSKR__DEPLOYS__ROOT`). |
| `systemd/ratatoskr-sites-gateway.service`, `systemd/sites-gateway.conf.example` | The sites gateway: the public listener on `127.0.0.1:8183`, the operator plane on `9473`. It reads the sites tree and writes nothing. |
| `sysusers.d/ratatoskr-sites.conf`, `sysusers.d/ratatoskr-sites-gateway.conf` | The two system users. |
| `nats/identity.conf` | The copy of the `SITES` stanza of Platform's NATS authorization file. |
| `postgres/role.sql` | The role `ratatoskr_sites`. |

## The sites tree

The host role of `ratatoskr-workspace` creates `/mnt/nvme/ratatoskr/sites` with the owner
`ratatoskr-sites`, the group `ratatoskr-sites-bundles` and the mode `2750`. The sites service writes
with `UMask=0027`, so every deploy directory is `2750` and every file `0640` with that group. The
gateway is a member of the group (`SupplementaryGroups=` of its unit and its sysusers file), and the
tree is in its `ReadOnlyPaths=`.

```text
/mnt/nvme/ratatoskr/sites/<site id>/
├── site.json                  hostnames and approval state, rewritten on every approval event
├── bundles/<commit>/          one verified deploy: ratatoskr-bundle.json and files/
├── current  -> bundles/<c2>   the deploy that the gateway serves
└── previous -> bundles/<c1>   the deploy that was current before
```

## The gateway on the host

- `cloudflared` reaches `127.0.0.1:8183` through the marked ingress block that the playbook renders
  from `group_vars/all/sites.json`; one rule per approved hostname.
- The gateway connects to Edge (`127.0.0.1:8080`) from `127.0.0.83`. Edge trusts
  `X-Ratatoskr-Client-IP` only from that address (`RATATOSKR__SITES__GATEWAY_PEER` of Edge), so the
  two values are one setting in two files.
- The unit starts with `MemoryMax=128M` and `CPUQuota=100%`. These are starting values to measure.
- HSTS is a setting of the Cloudflare zone, because TLS ends at Cloudflare.

## A deploy

The site's CI publishes an attested bundle; `ratatoskr-site deploy` of `ratatoskr-site-sdk` verifies
it on the controller and uploads it through Edge to `POST /v1/sites/{site}/deploys`, then activates
it with `POST /v1/sites/{site}/deploys/{commit}/activate`. The sites service checks every path and
digest again before it stores a byte, keeps the current, the previous and the newest deploys up to
`RATATOSKR__DEPLOYS__RETENTION` (default 5), and refuses a site that is not approved. Activating the
previous commit again is a rollback; it needs no download.
