# CLI cookbook

These examples use the default database and namespace. Add global `--db`, or set `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE`, when a deployment needs explicit isolation.

## Initialize and inspect health

```sh
supermem init
supermem status
supermem --json status
```

Run observational diagnostics against a repository:

```sh
supermem --json doctor --cwd /absolute/path/to/repository
```

`doctor` does not create, migrate, recover, checkpoint, or change the journal mode of a store.

## Record durable memory

Record a decision with file evidence:

```sh
supermem remember \
  --kind decision \
  --title "Release profile location" \
  --body "Use the workspace-level Cargo release profile" \
  --canonical-key release-profile-location \
  --file Cargo.toml \
  --tags release,cargo \
  --cwd .
```

A repeated `--canonical-key` revises the durable memory instead of creating an unrelated duplicate.

Read the body from stdin:

```sh
cat decision.txt | supermem remember \
  --kind decision \
  --body-stdin \
  --trust user_confirmed \
  --cwd .
```

Available memory kinds are `fact`, `preference`, `constraint`, `decision`, `procedure`, `episode`, `outcome`, `task`, and `observation`.

## Append an observation

Observations retain source evidence without automatically promoting it to durable memory:

```sh
printf '%s\n' 'cargo test --workspace passed' | supermem observe \
  --kind verification \
  --content-stdin \
  --tool-name cargo \
  --succeeded true \
  --verification true \
  --cwd .
```

Stable `--event-id` or `--idempotency-key` values make host retries idempotent.

## Create a task checkpoint

```sh
supermem checkpoint \
  --goal "Make release packaging deterministic" \
  --summary "Normalized archive timestamps and sorted entries" \
  --outcome success \
  --verification "python scripts/release_smoke.py --archive dist/package.tar.gz --version 0.1.0" \
  --open-task "Verify the Windows archive in CI" \
  --file scripts/package_release.py \
  --cwd .
```

`--verification` and `--open-task` are repeatable. Checkpoints attempt complete changed-file capture unless `--no-auto-artifacts` is set.

## Recall bounded context

```sh
supermem recall \
  --query "why is the package-level release profile ignored?" \
  --file Cargo.toml \
  --cwd . \
  --token-budget 1200
```

Useful switches:

- `--limit N` caps selected memories.
- `--include-stale` admits stale records.
- `--include-divergent` admits descendant or branch-divergent records.
- `--include-superseded` includes superseded revisions.
- `--format json` returns structured recall output.
- `--observe-prompt --event-id ID` records the query before recall in the same process.

Use the inclusion switches deliberately. The default excludes evidence that should not silently be treated as current.

## Inspect history and give feedback

```sh
supermem --json inspect MEMORY_ID --history
```

Attach a retrieval judgment:

```sh
supermem feedback MEMORY_ID \
  --signal helpful \
  --query-id QUERY_ID \
  --note "Explained the current release configuration"
```

Signals are `used`, `helpful`, `harmful`, `incorrect`, `outdated`, and `dismissed`.

Retract a memory from ordinary retrieval without erasing its history:

```sh
supermem retract MEMORY_ID --reason "Superseded by the workspace migration"
```

## Export, import, and purge

Create a canonical JSONL snapshot:

```sh
supermem export --output memory.jsonl
```

Import into an otherwise empty store:

```sh
supermem --db restored.sqlite3 import memory.jsonl
```

Permanently delete the selected database and sidecars only after stopping every process that uses it:

```sh
supermem purge --yes
```

Prefer the safer export and uninstall procedures in [Backup, upgrade, and uninstall](data-lifecycle.md).

## Manage optional search projections

List and manage immutable expansion or dense-vector profiles:

```sh
supermem index list-profiles
supermem index add-profile --profile-id PROFILE --model-digest DIGEST --dimensions 768
supermem index activate --profile-id PROFILE
supermem index pending --profile-id PROFILE --cwd . --limit 100
supermem index register --profile-id PROFILE --cwd . projections.jsonl
supermem index status --profile-id PROFILE --cwd .
supermem index artifact-status --cwd .
supermem index deactivate --profile-id PROFILE
supermem index remove-profile --profile-id PROFILE --yes
supermem index rebuild
```

The caller generates expansions and vectors outside the write and recall paths. Read [Search indexing](search-indexing.md) before registering a profile.

## Start MCP

```sh
supermem mcp --root /absolute/path/to/repository
```

The root, namespace, and optional workspace are launch-pinned. Model tool arguments cannot replace those boundaries. See [Harness integrations](integrations.md).
