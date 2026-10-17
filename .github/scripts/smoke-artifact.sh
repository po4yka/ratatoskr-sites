#!/usr/bin/env bash
# Real ARM64 artifact: unavailable without policy, ready with the approved root-owned tools.
set -euo pipefail
image=ratatoskr-sites:ci
namespace=sites-artifact-smoke
work=$(mktemp -d "${RUNNER_TEMP:?set the private runner directory}/sites-smoke.XXXXXX")
owned=()
root() { if ((EUID == 0)); then "$@"; else sudo "$@"; fi; }
cleanup() {
  local status=$? name
  trap - EXIT
  for name in "${owned[@]}"; do
    if [[ $(docker inspect --format '{{index .Config.Labels "ratatoskr.test"}}' "$name") == "$namespace" ]]; then
      docker rm -f "$name" >/dev/null
    fi
  done
  root rm -rf -- "$work"
  exit "$status"
}
trap cleanup EXIT
for name in "$namespace" "$namespace-db"; do
  [[ -z $(docker ps -aq --filter "name=^/$name$") ]] || { echo "Fixture already exists: $name" >&2; exit 1; }
done
mkdir "$work/downloads"
curl -fsSL --max-time 120 https://github.com/cli/cli/releases/download/v2.102.0/gh_2.102.0_linux_arm64.tar.gz -o "$work/downloads/gh.tar.gz"
curl -fsSL --max-time 120 https://github.com/oras-project/oras/releases/download/v1.3.0/oras_1.3.0_linux_arm64.tar.gz -o "$work/downloads/oras.tar.gz"
printf '%s  %s\n' 7862c86c72f43df3a2d93ddde6f473285b4e2af61b494849846827e513ef6484 "$work/downloads/gh.tar.gz" 7649738b48fde10542bcc8b0e9b460ba83936c75fb5be01ee6d4443764a14352 "$work/downloads/oras.tar.gz" | sha256sum --check --status
tar -xOf "$work/downloads/gh.tar.gz" gh_2.102.0_linux_arm64/bin/gh > "$work/downloads/gh"
tar -xOf "$work/downloads/oras.tar.gz" oras > "$work/downloads/oras"
printf '%s  %s\n' 93308395c2d296a63a662742c6366e4db413d2a4870d07bd9b84e491c065d65d "$work/downloads/gh" e2b67c4efb35f20a546aacc680e0f81f907d3cf4cf9f1b2040c34bb471c428f1 "$work/downloads/oras" | sha256sum --check --status
# CI fixture only: a public reviewed snapshot, fetched by immutable commit and content digest.
curl -fsSL --max-time 120 https://raw.githubusercontent.com/po4yka/ratatoskr-workspace/37ee5f9254d68188a7d1a94e2bd50dba18aff3a7/deploy/ansible/roles/sites_verifier/files/trusted-root.jsonl -o "$work/downloads/trusted-root.jsonl"
printf '%s  %s\n' 65ca537f6ed8a47fd0e560c421baa1f6c1efb8b25fc200d8c5c02c0e92eb2b9c "$work/downloads/trusted-root.jsonl" | sha256sum --check --status
root install -d -m 0755 "$work/tools"
root install -m 0755 "$work/downloads/gh" "$work/downloads/oras" "$work/tools/"
root install -m 0644 "$work/downloads/trusted-root.jsonl" "$work/tools/"
service_uid=$(docker run --rm --entrypoint id "$image" -u)
service_gid=$(docker run --rm --entrypoint id "$image" -g)
root install -d -m 0750 -o "$service_uid" -g "$service_gid" "$work/data"
port=55432
docker run -d --name "$namespace-db" --label "ratatoskr.test=$namespace" --network host \
  -e POSTGRES_USER=sites -e POSTGRES_PASSWORD=sites -e POSTGRES_DB=sites \
  -e POSTGRES_INITDB_ARGS="--locale-provider=icu --icu-locale=und-x-icu --encoding=UTF8" \
  postgres:17@sha256:e38411452a464af89e5adadb8d223bf53b898d47d6ef918b2d58c08707350449 -p "$port" >/dev/null
owned+=("$namespace-db")
for _ in $(seq 1 60); do
  if docker exec "$namespace-db" pg_isready -h 127.0.0.1 -p "$port" -U sites -d sites >/dev/null 2>&1; then break; fi
  sleep 1
done
cat > "$work/smoke.env" <<ENV
RATATOSKR__ADMIN__BIND=127.0.0.1:9472
RATATOSKR__API__LISTEN_ADDRESS=127.0.0.1:8100
RATATOSKR__DATABASE__URL=postgres://sites:sites@127.0.0.1:$port/sites
RATATOSKR__DEPLOYS__ROOT=/data/sites-tree
ENV
stop_service() {
  docker stop -t 40 "$namespace" >/dev/null
  [[ $(docker inspect -f '{{.State.ExitCode}}' "$namespace") == 0 ]] || { docker logs "$namespace" >&2; exit 1; }
  docker rm "$namespace" >/dev/null
  owned=("$namespace-db")
}
start_service() {
  docker run -d --name "$namespace" --label "ratatoskr.test=$namespace" --network host \
    --memory 512m --memory-swap 512m --cpus 1 --pids-limit 128 \
    --env-file "$work/smoke.env" -v "$work/data:/data/sites-tree" "$@" "$image" >/dev/null
  owned+=("$namespace")
  for _ in $(seq 1 60); do
    if curl -fsS http://127.0.0.1:9472/health/live >/dev/null 2>&1; then return; fi
    sleep 1
  done
  docker logs "$namespace" >&2
  echo 'Sites did not start' >&2
  exit 1
}
start_service
[[ $(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9472/health/ready) == 503 ]]
stop_service
cat >> "$work/smoke.env" <<'ENV'
RATATOSKR__DEPLOYS__VERIFIER__GH_PATH=/usr/lib/ratatoskr-sites/verifier/gh
RATATOSKR__DEPLOYS__VERIFIER__ORAS_PATH=/usr/lib/ratatoskr-sites/verifier/oras
RATATOSKR__DEPLOYS__VERIFIER__TRUSTED_ROOT_PATH=/usr/lib/ratatoskr-sites/verifier/trusted-root.jsonl
RATATOSKR__DEPLOYS__VERIFIER__SIGNER_COMMIT=89331e6aa0b845c3565c1d9785b0dd5038e2de81
ENV
docker run --rm --network host --env-file "$work/smoke.env" "$image" check-config 2>/dev/null
start_service -v "$work/tools:/usr/lib/ratatoskr-sites/verifier:ro"
ready=no
for _ in $(seq 1 60); do
  if curl -fsS http://127.0.0.1:9472/health/ready >/dev/null 2>&1; then ready=yes; break; fi
  sleep 1
done
[[ $ready == yes ]] || { docker logs "$namespace" >&2; echo 'Sites verifier did not become ready' >&2; exit 1; }
present=$(docker exec "$namespace-db" psql -U sites -p "$port" -d sites -Atc "select to_regclass('sites.releases') is not null")
[[ $present == t ]]
stop_service
echo 'ARM64 artifact: unavailable without verifier, ready with real tools, clean stop: PASS'
