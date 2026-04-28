# Global Configuration Workflow

Use this workflow when the task is to configure where `cpkg` reads package indexes from or how it resolves repository remotes.

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

Use this before any `cpkg config index ...` or `cpkg config org ...` edits.

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

## Index Lookup Order

Project commands prefer package indexes in this order:

1. `[index]` overrides in `wtrproject.toml`
2. A project-local `cpkg_index.json`
3. Global index sources from `~/.cpkg/config.toml`
4. The built-in default remote index and cache
