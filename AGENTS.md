# Repository Guidelines

## Project Structure & Module Organization
`cpkg` is now both a WTR project-side package manager and a driver-package authoring tool. Keep CLI wiring in `src/main.rs` thin and place reusable logic under `src/lib.rs`, `src/project/`, and `src/package/`.

- `src/project/` contains STM32CubeMX project management logic:
  - `manifest.rs` owns `wtrproject.toml` loading, validation, and dependency edits.
  - `index.rs` resolves the package index from project-local files, configured paths/URLs, or the default cache under `~/.cpkg/`.
  - `resolver.rs` computes direct and transitive package/repository requirements.
  - `submodule.rs` synchronizes `Modules/` Git submodules, always tracking `main`.
  - `integration.rs` regenerates project integration output for the current dependency set.
  - `interactive.rs` contains interactive dependency-selection flows shared by `init` and `add`.
- `src/package/` contains driver-package authoring logic:
  - `manifest/mod.rs` owns canonical `cpkg.toml` parsing and saving.
  - `manifest/migrations.rs` contains versioned migration steps; keep migration logic out of the main manifest module.
  - `scanner.rs` discovers package sources and headers.
  - `generator.rs` writes package `CMakeLists.txt`.

Build artifacts belong under `target/` and should stay untracked.

## Supported Workflows
There are now two distinct command families:

- Project-side package manager commands:
  - `cpkg init`
  - `cpkg add`
  - `cpkg remove`
  - `cpkg sync`
- Driver-package authoring commands:
  - `cpkg package init`
  - `cpkg package generate`
  - `cpkg package create`

When changing CLI behavior, update the help text in `src/main.rs` and verify the relevant `--help` output.
- Keep interactive dependency selection in a single tree-style picker; avoid multi-step repository-then-package selection flows unless the user explicitly asks for them.
- Interactive selection must not write or modify project files until the user confirms the final selection.
- Once the user confirms a direct-dependency change, write `wtrproject.toml` before any follow-up network-backed work so an interrupted run can be resumed with `cpkg sync`.
- Network-backed operations should surface live execution status to the user; prefer a temporary, bounded terminal log panel over silent background work.
- `cpkg add -I` should refresh the package index as soon as the command starts, before rendering the picker, and should reuse that refreshed index for the confirmed selection.
- When `cpkg add -I` only removes packages and adds no new direct dependency, it should update `wtrproject.toml`, regenerate project links locally, and remove any managed submodule repositories that are no longer required, without fetching or syncing retained submodules.
- `cpkg remove` should not fetch or synchronize retained submodules; it should update `wtrproject.toml`, locally regenerate the project integration file, and remove any managed submodule repositories that are no longer required.
- The tree-style picker should support inline fuzzy search/filtering without leaving the interactive view.
- `cpkg add -I` should behave like editing the current direct dependency set: existing direct dependencies start preselected, and the confirmed selection becomes the new direct dependency list.
- Preselected or currently selected packages in the interactive picker should have clear visual highlighting beyond the checkbox marker alone.
- The confirmation output for `cpkg add -I` should summarize dependency changes (added/removed/unchanged) instead of only reporting the final selected package count.
- Search-hit character-level highlighting is intentionally deferred for now; keep it tracked as a TODO and do not implement it unless the user asks again.

## STM32CubeMX Project Assumptions
Project-side commands operate on STM32CubeMX firmware repositories only.

- Run project commands from the Git repository root.
- The project root must contain exactly one applicable `*.ioc` file unless the user passes `--ioc`.
- Managed driver repositories live under `Modules/` and are tracked as Git submodules.
- Synchronization always pulls the latest upstream state and forces managed submodules to track the `main` branch.
- Direct project dependencies are stored in `wtrproject.toml`; there is currently no lockfile.
- Package versions are currently non-semantic for resolution purposes; treat the latest indexed package as the only selectable version.

## Package Index Rules
Prefer the existing resolution order implemented by the code:

1. `wtrproject.toml` `[index]` overrides
2. project-local `cpkg_index.json`
3. remote default index with cache under `~/.cpkg/cpkg_index.json`

The current default remote index is the WTR package index on GitHub. Keep index-loading behavior configurable and avoid hard-coding new lookup paths in unrelated modules.

## Build, Test, and Development Commands
- `cargo run -- --help` — show the full CLI surface.
- `cargo run -- init --help` — inspect project initialization options.
- `cargo run -- add --help` — inspect dependency-add and sync options.
- `cargo run -- sync --help` — inspect submodule synchronization options.
- `cargo run -- package --help` — inspect package-authoring commands.
- `cargo test --offline` — run unit and doc tests without network access.
- `cargo fmt --check` — verify formatting before review.

Use targeted `cargo run --offline -- <command> --help` checks when editing CLI docs or argument parsing.
When a change needs to fetch a new crate, refresh `Cargo.lock`, or otherwise update dependencies, it is allowed to run the relevant Cargo command without `--offline`.

## Coding Style & Naming Conventions
Follow default Rust style: 4-space indentation, `snake_case` for functions/modules, `PascalCase` for types, and concise error messages with `anyhow::Context`.

- Prefer small modules with clear responsibilities.
- Keep project-side and package-side logic separated instead of reintroducing large monolithic modules.
- Reuse shared interactive or sync-option code instead of duplicating command-specific implementations.
- Keep manifest migration logic versioned and composable so future format upgrades only add new steps.

## Cross-Platform Expectations
The codebase must remain compatible with Linux and Windows.

- Avoid Unix-only shell or path assumptions in Rust code.
- Normalize generated/discovered paths where needed so tests pass on both platforms.
- Prefer standard-library filesystem/process APIs over shell-specific behavior.
- Do not spend effort on MSVC compatibility; if Windows toolchain validation is needed, target GNU.

## Testing Guidelines
Place unit tests next to the code they validate using `#[cfg(test)] mod tests`.

- Cover `wtrproject.toml` behavior, package-index loading, dependency resolution, submodule/integration generation logic, and `cpkg.toml` migrations.
- Add focused regression tests for CLI parsing changes when behavior is non-trivial.
- Run the most targeted checks first, then broader validation such as `cargo test --offline`.
- If dependency changes require network access or a lockfile refresh, use the smallest non-offline Cargo command that unblocks verification, then resume targeted checks.

## Commit & Pull Request Guidelines
Keep commits scoped to one change and use commit messages in `type(scope): content` form, for example `docs(cli): expand command help text`.

- Disable GPG signing for local commits with `git -c commit.gpgsign=false commit ...`.
- PRs should explain user-visible impact, list verification commands run, and include sample CLI output when command behavior changes.

## Tooling Notes
`cpkg` manages both project dependency state and driver-package metadata.

- Prefer updating package metadata through `cpkg package init`, `cpkg package generate`, and `cpkg package create` instead of editing generated outputs by hand.
- Prefer updating project dependency state through `cpkg init`, `cpkg add`, `cpkg remove`, and `cpkg sync` instead of editing generated integration files by hand.
- If the user's requirements change and the new requirement should be captured in `AGENTS.md`, update `AGENTS.md` immediately before continuing the implementation.
