# Global Configuration Workflow

Use this workflow when the task is to configure package indexes, repository remotes, or automatic update-check preferences.

## File Location

- Global config lives at `~/.cpkg/config.toml`.
- If the file does not exist, built-in defaults are still active.
- Commands that modify the global config require the file to exist first.

## Main Commands

### Create the config file

```bash
cpkg config init
cpkg config init --force
```

Use this before any `cpkg config index ...`, `cpkg config org ...`, or `cpkg config update-check ...` edits.

### Inspect the current config

```bash
cpkg config show
cpkg config index list
```

### Manage global index sources

```bash
cpkg config index add --url https://mirror.example.com/cpkg_index.json
cpkg config index add --path /tmp/cpkg_index.json --position 1
cpkg config index set 1 --url https://mirror.example.com/cpkg_index.json --cache-path indexes/mirror.json
cpkg config index remove 2
cpkg config index move 3 1
```

Rules:
- Index sources are tried in order.
- Each source must set either `path` or `url`.
- `cache_path` only makes sense with `url`.

### Manage named org sources

```bash
cpkg config org set wtr-github --ssh-base git@github.com:HITSZ-WTRobot-Packages --https-base https://github.com/HITSZ-WTRobot-Packages
cpkg config org set wtr-github --default-protocol https
cpkg config org default set wtr-github
cpkg config org default clear
```

Rules:
- A named org source should define at least one remote base.
- The default protocol must match a defined remote base.
- Project manifests may override the org name or protocol without mutating the global defaults.

### Control automatic update checks

```bash
cpkg config update-check disable
cpkg config update-check enable
```

Rules:
- Requires an existing config file; run `cpkg config init` first if it is absent.
- Stores `[update_check] enabled = false` or `true`, preserving index/org settings. Missing preferences default to enabled.
- Automatic checks start before online `init -I`, `list`, `add`, or `sync` when stderr is a terminal. Offline, local, config, help/version, and explicit-update commands never trigger them. A command that later fails may already have started a check, but must not show or consume a notice.
- The foreground never waits for GitHub. Detached checks only fetch official Release metadata, with a 3-second timeout, no retries, and approximately 24 hours between attempts, including failures.
- After success, show a ready cached newer Release in the same invocation, once per tag. If the check is unfinished, do not wait; a later eligible successful command can show the result. Automatic checks never install or create/change `config.toml`; `cpkg update` remains available when checks are disabled.

## Index Lookup Order

Project commands prefer package indexes in this order:

1. `[index]` overrides in `wtrproject.toml`
2. A project-local `cpkg_index.json`
3. Global index sources from `~/.cpkg/config.toml`
4. The built-in default remote index and cache

Index refresh is opt-in and separate from package synchronization: cached copies are reused unless the user passes `-u`/`--update-index`, and a download only happens automatically when no local or cached index exists.
