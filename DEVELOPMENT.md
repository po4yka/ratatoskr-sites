# Developing Ratatoskr Sites

This file tells you how to build, test and run `ratatoskr-sites` on a developer machine. Read
AGENTS.md first: it holds the rules that this file does not repeat.

## Toolchain

`rust-toolchain.toml` pins Rust 1.97.0 with `rustfmt` and `clippy`. `rustup` installs it on the
first `cargo` command in this directory.

## Local dependencies

The tests need PostgreSQL 17. `compose.yaml` starts it:

```bash
docker compose up -d --wait
```

The tests of `services/sites` that build a release (`release_build.rs`, `release_switch.rs`)
create one database per test and drop it at the end. They read
`SITES_TEST_DATABASE_URL`, and the default is `postgres://sites:sites@127.0.0.1:5432/sites`, the
database that `compose.yaml` starts. If the postgres of another Ratatoskr repository uses port
5432, stop it first or set `SITES_TEST_DATABASE_URL`.

The test of the outbox relay (`services/sites/tests/outbox_relay.rs`) needs a NATS server with
JetStream. It reads `SITES_TEST_NATS_URL`, and the default is `nats://127.0.0.1:4222`. CI starts one;
on a laptop:

```bash
docker run -d --rm --name sites-nats -p 127.0.0.1:4222:4222 nats:2.15.0-alpine -js
```

`outbox_relay.rs::outbox_relay_publishes_draft_changed` also needs Docker: it starts its own NATS
server, from the image digest of `ci.yml`, on a free loopback port, with an administrator and only
the `SITES` stanza of `deploy/nats/identity.conf`, and removes the container at the end. Without
that image, set `SITES_TEST_NATS_SERVER` to a native `nats-server` (2.10 or later), and the test
runs it as a child process with the same configuration.

The consumer tests (`build_triggers.rs` and the tests after it) make their own command and event
streams, with subjects under a prefix of their own, and the four durables on them as Edge makes
them, so they share no subject with another test. The relay test makes a stream with a new name
for the subjects `evt.sites.release.published.v1` and
`evt.platform.operation.reported.v1`, and deletes it at the end. Do not point it at a server that
holds the stream `ratatoskr_events`, which takes the same subjects.

### Rust — also the CI gate

The gate is `.github/workflows/ci.yml`; this list and that job's `run:` steps are one list, and a step in ci.yml fails when they differ. Run them in this order before you declare a change done:

```bash
cargo fetch --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
cargo test --workspace --locked
cargo build --workspace --locked --release
```

`cargo deny check` is not in this list. It runs in its own `deny` job in ci.yml, so that a new
RustSec advisory cannot hide a clippy or test failure. It reads RustSec advisories, licences,
duplicate versions and the source policy (`deny.toml`). Install it once with
`cargo install cargo-deny`, and run it before you declare a change done.

Notes on the steps:

- `cargo clippy` applies the size limits from `clippy.toml`: a function has at most 100 code
  lines, a signature at most 7 arguments, a block at most 5 levels. An exception is
  `#[expect(clippy::too_many_lines, reason = "...")]` at the site, never a higher number.
- The file-length limit (850 lines in a tracked `.rs` file) runs as the awk step between clippy
  and build in ci.yml. Run it locally with:
  `git ls-files -z "*.rs" | xargs -0 -r wc -l | awk '$2 != "total" && $1 > 850 { print; bad = 1 } END { exit bad }'`
- The tests need the database above. They fail when it is unreachable; they do not skip.
- `openspec validate --all --strict` checks the OpenSpec changes and specs of this repository.

## Running the service

```bash
export RATATOSKR__DATABASE__URL="postgres://sites:sites@127.0.0.1:5432/sites"
cargo run -p ratatoskr-sites
```

The service applies `schema.sql` on its first start, then serves the operator plane on
`127.0.0.1:9472` and the domain API on `127.0.0.1:8100`. `cargo run -p ratatoskr-sites -- check-config`
validates the environment and binds nothing (exit 0 valid, exit 78 invalid).

| Variable | Default | Meaning |
|---|---|---|
| `RATATOSKR__DATABASE__URL` | none, required | SECRET. The `PostgreSQL` URL. |
| `RATATOSKR__DATABASE__MAX_CONNECTIONS` | `4` | The pool bound, 1 to 16. |
| `RATATOSKR__DATABASE__ACQUIRE_TIMEOUT_SECONDS` | `5` | 1 to 60. |
| `RATATOSKR__ADMIN__BIND` | `127.0.0.1:9472` | The operator listener. |
| `RATATOSKR__API__LISTEN_ADDRESS` | `127.0.0.1:8100` | The domain API listener. It must differ from the operator listener. |
| `RATATOSKR__BUS__URL` | none | The NATS server, for example `nats://127.0.0.1:4222`. Without it no outbox relay runs, the events stay in the outbox, and no durable is read. |
| `RATATOSKR__BUS__NKEY_SEED_PATH` | none | The nkey seed of the `SITES` identity, `/etc/ratatoskr/sites.nkey` on the host. Every connection uses the inbox prefix `_INBOX_sites`. |
| `RATATOSKR__BLOBS__ROOT` | `/mnt/nvme/ratatoskr/blobs/ratatoskr-sites` | The pages and assets of each release. |
| `RATATOSKR__BLOBS__NOTES_ROOT` | `/mnt/nvme/ratatoskr/blobs/ratatoskr-notes` | The note bytes, read only. |
| `RATATOSKR__NOTES__BASE_URL` | `http://127.0.0.1:8099` | The notes domain listener; the build reads `GET /internal/connections/{connection}/files` there (30 s, at most 32 MiB). |
| `RATATOSKR__SHUTDOWN__GRACE_SECONDS` | `10` | How long in-flight requests may take after SIGTERM, 1 to 60. |
| `RATATOSKR__TELEMETRY__LOG_FORMAT` | `text` | `text` or `json`. |
| `RATATOSKR__TELEMETRY__LOG_FILTER` | `info` | A tracing `EnvFilter` directive string. |

An unknown `RATATOSKR__` key stops the start with exit 78.

## Running the sites gateway

```bash
export RATATOSKR__SITES__ROOT="$PWD/target/sites-tree"
cargo run -p ratatoskr-sites-gateway
```

It reads the sites tree, serves the public listener on `127.0.0.1:8183` and the operator plane on
`127.0.0.1:9473`, and forwards `/_r/` to Edge at `RATATOSKR__EDGE__URL`. On a developer machine that
has only `127.0.0.1` on its loopback interface, set `RATATOSKR__EDGE__SOURCE_ADDRESS=127.0.0.1`.
`deploy/systemd/sites-gateway.conf.example` lists every key. `cargo run -p ratatoskr-sites-gateway --
check-config` validates the environment and binds nothing.

## Schema

`schema.sql` at the repository root is the whole schema. The binary embeds it and applies it, under
an advisory lock, to a database that has no `sites` schema. There are no migrations: a schema
change edits `schema.sql` in place, and the next test database has it. A database made before a
part of `schema.sql` does not get that part. Drop its `sites` schema (`drop schema sites cascade`),
and the next start of the service makes it again from `schema.sql` (AGENTS.md, Development status:
no migrations).

## Fixtures

`fixtures/selection-vault/` and `fixtures/privacy-vault/` are vault connections: `vault/` holds the
files, and `index.json` holds what the notes internal index route gives for each file, without the
revision. The loader of `ratatoskr-sites-publications` (feature `test-support`) computes each
revision from the bytes. Every string with `zq7` in the privacy vault comes from a file that its
publication does not select, and no byte of a release may contain it. A new filter rule adds its
canary location there.

## Deploy files

| File | Purpose |
|---|---|
| `deploy/postgres/role.sql` | Creates the role `ratatoskr_sites` in the shared database. |
| `deploy/nats/identity.conf` | A copy of the `SITES` stanza of Platform's NATS authorization file. |
| `deploy/systemd/sites.conf.example` | The environment file of the service on the host. |
| `deploy/systemd/ratatoskr-sites.service` | The unit of the service. |
| `Dockerfile` | The `export` stage holds only `ratatoskr-sites` and `ratatoskr-sites-gateway` for `linux/arm64` and glibc 2.36. The `release-artifact` job of `ci.yml` publishes it for each commit on `main` through the reusable workflow of `ratatoskr-workspace`. |
| `deploy/systemd/sites-gateway.conf.example` | The environment file of the sites gateway on the host. |
| `deploy/systemd/ratatoskr-sites-gateway.service` | The unit of the sites gateway. |
| `deploy/sysusers.d/ratatoskr-sites-gateway.conf` | The system user `ratatoskr-sites-gateway`, and its membership in the read group `ratatoskr-sites-bundles` of the sites tree. |
| `deploy/sysusers.d/ratatoskr-sites.conf` | The system user `ratatoskr-sites`, and its membership in the read group `ratatoskr-notes-blobs` of the notes blob root. The group, the setgid root and the umask of notes come from OBS-023. |

## Workflow

1. Start from an OpenSpec change (`/opsx:propose`), as AGENTS.md describes.
2. For each behaviour, write the failing test, run it, and confirm the failure reason.
3. Write the implementation and run the test again.
4. Run the command list above and `cargo deny check`.

## The Rust skills in this repository

`.agents/skills/` holds the vendored Rust skills and `.claude/skills/` links to them. Do not edit
them here. AGENTS.md explains where a correction goes.
