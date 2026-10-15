# Sites testing

## Rules

- Every behaviour starts from a failing test. The task list pairs a RED task with a GREEN task.
- A test that needs PostgreSQL creates a disposable database with
  `ratatoskr_sites_persistence::test_support::TestDatabase` and drops it at the end. It fails when
  the database is unreachable; it does not skip.
- HTTP behaviour is tested through the real router with `tower::ServiceExt::oneshot`, with the
  headers that Edge mints.
- A privacy test builds a release from a fixture vault whose unselected files carry canary strings,
  and searches every byte of the release for every canary.
- A test never reads the wall clock. `fleet.yml` fails on `SystemTime::now`, `Utc::now` and the
  like in a test file.
- Test data is obvious fake content. Never use a real vault or a real credential.

## Running

```bash
docker compose up -d --wait
cargo test --workspace --locked
```

`DEVELOPMENT.md` holds the whole gate.
