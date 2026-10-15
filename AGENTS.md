# Ratatoskr Sites Agent Instructions

## Scope

These instructions apply to the `ratatoskr-sites` repository.

This repository owns the sites bounded context: the site registry, the submitted site manifests,
the read projection of the approvals that Platform holds, the publications, the release builder,
the releases with their pages, assets and search index, and the Content Delivery API. The sites
gateway also lives here; the workspace change `add-site-gateway-hosting` (SITE-026) adds it.

## Repository mission

Sites makes a safe public copy of the notes that the owner selects, and serves it to sites that
have no session. It answers two questions:

1. Which files of a vault connection does a publication select, and what is the release that the
   selection makes after the privacy rules?
2. Which release of an approved site does a visitor read now?

A vault is private by default. The selection, the privacy filter and the public read path are in
this one repository so that no site can receive what did not pass the privacy rules.

## Current phase

The repository starts with the workspace change `add-site-publications` (SITE-025). The service,
the `sites` schema, the operator plane and the deploy examples exist. Read README.md under Project
status for the parts that exist today. Do not assume anything beyond what is present in the
checkout.

### Development status

Ratatoskr is in development. No database holds data that has to survive a schema change. While this
status holds, these rules are binding, and they override anything else in this repository that
plans otherwise, including the rest of this file:

- **One version only.** The API, the database, and the contracts keep their first version. Do not
  add a `v2` or a later major version, and do not add version negotiation, deprecation windows, or
  parallel-major routing.
- **No database migrations.** Do not add a migration file, and do not add migration tooling. A
  schema change edits the current schema definition in place, and a test database is created from
  that definition.
- **The product is `Ratatoskr`.** It is not "Ratatoskr Next". Do not write that name in code,
  documentation, identifiers, comments, or commit messages.

Only the repository owner changes this status. Ask before you write anything these rules forbid.

## How a change starts

Every non-trivial change begins as an OpenSpec change rather than as an edit, and each assistant
starts one in its own syntax. Claude Code has the command: `/opsx:propose <what you want to build>`,
or `/opsx:explore` first when the shape is not clear yet. Codex has no project-level command and
triggers the same skill by name, `$openspec-propose`, or lets its description match it. OpenCode has
its own command, `/opsx-propose`. Whichever starts it, the result is `openspec/changes/<id>/` holding
a proposal, the spec deltas, a design and a task list, and you read that plan before any code is
written. `/opsx:apply`, `$openspec-apply-change` or `/opsx-apply` builds it, and `/opsx:archive`,
`$openspec-archive-change` or `/opsx-archive` folds the deltas into `openspec/specs/`.

`openspec/specs/` holds the behaviour that is true today, and it starts empty on purpose. A spec here
grows from a change that needed it. Do NOT convert `docs/REQUIREMENTS.md`, `docs/INTERFACES.md`,
`docs/DOMAIN.md` or `docs/DATA_MODEL.md` into specs in bulk. Those documents stay where they are, as
material an exploration reads. A spec set produced by bulk conversion is large, stale on the day it
lands, and trusted by nobody.

Behaviour that more than one repository can see — the shape of a contract, the meaning of a field, the
order in which repositories must receive a change — belongs in the `ratatoskr-workspace` store, not
here. `openspec/config.yaml` references it, so `openspec instructions` in this repository lists the
store's specs with the exact command that fetches one. Cite that spec from a local proposal instead
of restating it.

### Tests come first

The task list carries one pair per behaviour. The first task adds a test that fails. The second makes
it pass. Never one task that does both.

- Run the new test before you write the implementation, and confirm it fails for the reason the task
  states — not for a compile error or a typo.
- A refactor task comes after the tests are green. It adds no test and changes no behaviour.
- A task that cannot start from a failing test says why in one line. Configuration, documentation and
  generated files are the usual reasons.
- Do not tick a task whose test has not been run.

Nothing can check the order in which the two were written. What CI does check is
`openspec validate --archived`, which fails when a change was archived with a task left unticked, and
the step in `fleet.yml` that fails when a repository holds a manifest and a `ci.yml` that never runs
a test. `ratatoskr-workspace/docs/QUALITY_GATES.md` states that limit rather than implying it is
covered.

## The Rust skill catalogue

`.agents/skills/` holds eighteen Rust skills, and `.claude/skills/` symlinks to them, so all three
assistants read one copy. Codex reads `.agents/skills/`, Claude Code reads `.claude/skills/`, and
OpenCode scans both, so the existing symlink already covers it and nothing belongs under
`.opencode/skills/`. Each is a reference sheet rather than a tutorial: the commands, flags,
thresholds and triage tables for one Rust concern. Your assistant reads the descriptions and opens a
skill only when the task matches one, so the set costs almost nothing until it is needed.

`rust-tdd` is the Rust form of the task pair above. `rust-lints` owns `clippy.toml`, which is where
this repository's size limits live. `rust-security` answers a `RUSTSEC` advisory.
`rust-async-internals` covers `tokio::select!` cancel safety and shutdown. `rust-database` covers
pool budgets and transaction ownership. `rust-compiler-errors` is the entry point when the build
fails and the cause is not obvious.

`rust-database` also carries a section on deploying migrations in compatible phases. The Development
status above overrides it: while that status holds, this product has no migrations at all. Read the
rest of that skill and skip that section.

The eighteen are identical in every Ratatoskr repository whose stack is Rust, and
`ratatoskr-workspace/.github/workflows/drift.yml` fails when one copy stops matching the others. Do
not edit a file under `.agents/skills/`. A correction belongs upstream in `po4yka/rust-skills` and
reaches this repository through `npx skills update`.

The catalogue holds forty-four skills and eighteen are vendored here.
`ratatoskr-workspace/docs/QUALITY_GATES.md` records which were left out and why. They are vendored
under BSD-3-Clause, (c) 2026 Nikita Pochaev, who also owns this repository; each `SKILL.md` keeps its
`license` field, and the full text is in that repository's `LICENSE`.

## Deployment target

**The host is frozen: nothing is deployed to it and nothing on it is changed.** It stays in this document as a set of constraints on the code you write, and for no other reason. Do not offer to deploy, do not measure the running host against `main`, and do not treat what is installed there as the state of this repository. Everything below is a budget, not an environment.

Ratatoskr runs on ONE machine and there is no second one: a Raspberry Pi 5, `aarch64`, 4 cores, 16 GiB RAM, Debian 12 with glibc 2.36 and systemd 252, a 16 KiB page size, root moving to an NVMe SSD and every durable byte under `/mnt/nvme`. One systemd unit, `ratatoskr-sites.service`, supervises `ratatoskr-sites`. There is no orchestrator, no pod, no replica, no rolling deployment and no secret store. `ratatoskr-workspace/docs/DEPLOYMENT_TARGET.md` holds the host description, the storage contract and the port table; this section is complete without it.

You may assume:

- exactly one `ratatoskr-sites` process, started by systemd on that host;
- one PostgreSQL cluster (major 17) and one NATS server, shared with other tenants on the box;
- a path under `/mnt/nvme` is durable storage: the sites blob root `/mnt/nvme/ratatoskr/blobs/ratatoskr-sites`, and the notes blob root `/mnt/nvme/ratatoskr/blobs/ratatoskr-notes`, which this service reads and never writes, as a member of the read group `ratatoskr-notes-blobs`;
- the operator is one person with `ssh`, not a pipeline with an API.

Stop assuming:

- **a second instance.** Never answer a load or availability problem with a second process. The schema advisory lock, the outbox, the inbox and the one build worker per site STAY: they make a restart and a redelivery safe with one process.
- **a rolling deployment.** An upgrade is a full stop and start.
- **spare memory or CPU.** The host runs other services and has no disk swap. A build renders every selected page, so builds are coalesced, and the unit carries `MemoryMax=`. Every queue, cache, batch, connection pool and worker pool carries an explicit bound.
- **the boot device.** New durable state gets an absolute path under `/mnt/nvme`, named in the deployment unit.
- **that a dependency runs here.** Every crate, image and binary needs a `linux/arm64` build against glibc 2.36 that works on 16 KiB pages. `PrivatePIDs=` needs systemd 257 and is not available.
- **a free port.** The domain API is `8100` and the operator listener is `9472`, both planned for the port table in `ratatoskr-workspace/docs/DEPLOYMENT_TARGET.md` (SITE-025, workspace task 9.6). A compiled default is for a developer machine only. Never widen a bind to `0.0.0.0`.

The domain API is reached only through Edge on loopback, and it trusts only the claims that Edge mints. Public traffic reaches it only through Edge's `public` route class, behind `cloudflared`. The operator listener (`/health/*`, `/metrics`, `/version`), PostgreSQL and NATS are never reachable from outside the host.

## Sources of truth

Use this order:

1. the active task or changeset, and the workspace change `add-site-publications` with its specs;
2. accepted ADRs, in this repository and
   `ratatoskr-workspace/docs/adr/0011-independent-sites-on-the-single-host.md`;
3. `README.md`;
4. the contracts in `ratatoskr-contracts` (`ratatoskr-sites-contracts`);
5. repository tests;
6. implementation details.

`ratatoskr-workspace/docs/NOTES_AND_SITES.md` holds the shared names: subjects, routes, ports,
storage paths and error codes. Use those names and no others.

## Hard bounded-context rules

### Sites owns

- the site registry and the submitted manifests, by digest;
- the read projection of the approvals that Platform announces;
- the publications, the build queue, the releases, their pages, assets and search index;
- the current release of each site, and the Content Delivery API;
- the sites blob root, the sites outbox and inbox.

### Sites does not own

- the approval of a manifest (Platform's `identity.site_approvals`, the only authority);
- user identity, sessions or device pairing (Platform);
- the vault connections, their files, revisions, bytes and index (`ratatoskr-notes`);
- analysis, embeddings or the owner's knowledge search (Knowledge).

Sites reads note bytes read-only from the notes blob root and verifies every `BlobRef` before it
uses the bytes. It never writes, renames or deletes anything there.

## Privacy rules

These rules come from the workspace spec `site-release-privacy`. Do not weaken them.

- A release holds no data that the release builder read from a file that the publication does not
  select.
- The privacy filter is one pass on the parsed tree, before the AST serialiser, the HTML renderer,
  the search indexer and the asset collector.
- A test fixture that adds a filter rule adds a canary location to the privacy fixture vault.

## Testing expectations

- Every behaviour starts from a failing test, as the task pairs above require.
- Tests that touch the database use a disposable database from
  `ratatoskr_sites_persistence::test_support`.
- A test never reads the wall clock (`fleet.yml` checks it).

## Completion criteria

A task is complete only when:

- the responsibility belongs to Sites;
- its test failed first for the stated reason and passes now;
- the command list in DEVELOPMENT.md passes, and so does `cargo deny check`;
- no release can hold data from an unselected file;
- the schema change, if any, edits `schema.sql` in place.
