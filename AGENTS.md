# AGENTS Guidelines for `cpkg`

## Scope
- This file applies to the entire repository rooted at `/home/syhanjin/workspace/tools/cpkg`.
- `cpkg` serves two related roles:
  - a project-side package manager for STM32CubeMX firmware repositories
  - a driver-package authoring tool for reusable driver libraries
- This repository targets projects under `/home/syhanjin/workspace/robocon2026`, so domain assumptions about those firmware repositories are documented here because they affect CLI behavior, integration logic, and validation strategy.

## Managed Firmware Project Assumptions

### STM32 + CubeMX Projects
These rules describe the projects that `cpkg` manages under `/home/syhanjin/workspace/robocon2026`.

- All STM32 projects under `/home/syhanjin/workspace/robocon2026` share the same common driver-library pool; project-to-project differences should be limited to linked driver targets, business logic under `UserCode/`, and potentially the `.ioc` configuration.
- Treat `UserCode/` as the only location for project-specific application code.
- Treat `Modules/*` as referenced driver-library repositories; each `Modules/${Name}` directory normally corresponds to an upstream GitHub repository brought in as a Git submodule.
- Large driver repositories may contain multiple smaller driver packages.
- A directory containing `cpkg.toml` marks a driver package.
- Main projects consume driver libraries by first adding the driver repository with `add_subdirectory(...)`, which makes all packages from that repository available.
- After adding a driver repository with `add_subdirectory(...)`, link only the required driver targets into the firmware target.
- Driver target names should match the names declared in the corresponding `cpkg.toml` files.
- Use `/home/syhanjin/workspace/robocon2026/references/Packages/cpkg_index.json` as the generated driver-package index.
- Prefer editing the project `.ioc` file and regenerating CubeMX output instead of hand-editing generated HAL code.
- Do not inspect `Core/`, `Drivers/`, or `Middlewares/` unless the user explicitly asks; they are treated as STM32CubeMX-generated code.

### Driver Package Metadata Management
- Run `cpkg --help` for top-level usage and `cpkg <COMMAND> --help` before using an unfamiliar subcommand.
- Use `cpkg package init`, `cpkg package generate`, and `cpkg package create` for package-authoring workflows in this repository.
- When adding or renaming driver source files, update the nearest `CMakeLists.txt` and `cpkg.toml` if the package surface changes, then regenerate the package index.
- Do not manually edit generated package index files when `cpkg` can regenerate them.

## Product Responsibilities

### Command Families
`cpkg` exposes two distinct command families:

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

### Repository Layout
- Keep CLI wiring in `src/main.rs` thin.
- Put reusable logic under `src/lib.rs`, `src/project/`, and `src/package/`.
- Build artifacts belong under `target/` and should remain untracked.

`src/project/` owns STM32CubeMX project management:
- `manifest.rs` loads, validates, and edits `wtrproject.toml`.
- `index.rs` resolves the package index from project-local files, configured paths or URLs, or the default cache under `~/.cpkg/`.
- `resolver.rs` computes direct and transitive package and repository requirements.
- `submodule.rs` synchronizes `Modules/` Git submodules and forces them to track `main`.
- `integration.rs` regenerates project integration output for the current dependency set.
- `interactive.rs` contains interactive dependency-selection flows shared by `init` and `add`.

`src/package/` owns driver-package authoring:
- `manifest/mod.rs` parses and saves canonical `cpkg.toml` manifests.
- `manifest/migrations.rs` contains versioned migration steps; keep migration logic out of the main manifest module.
- `scanner.rs` discovers package sources and headers.
- `generator.rs` writes package `CMakeLists.txt`.

## Behavior Requirements

### Interactive Dependency Selection
- Keep interactive dependency selection in a single tree-style picker; avoid multi-step repository-then-package flows unless the user explicitly asks for them.
- Interactive selection must not write or modify project files until the user confirms the final selection.
- The tree-style picker should support inline fuzzy search and filtering without leaving the interactive view.
- Existing direct dependencies should start preselected in `cpkg add -I`, and the confirmed selection becomes the new direct dependency set.
- Preselected or currently selected packages in the interactive picker should have clear visual highlighting beyond the checkbox marker alone.
- The confirmation output for `cpkg add -I` should summarize dependency changes as added, removed, and unchanged items instead of only reporting the final selected package count.
- Search-hit character-level highlighting is intentionally deferred; keep it as a TODO and do not implement it unless the user asks again.

### Dependency Write and Sync Ordering
- Once the user confirms a direct-dependency change, write `wtrproject.toml` before any follow-up network-backed work so interrupted runs can resume with `cpkg sync`.
- When `cpkg add` cannot apply a newly added dependency without fetching repository data, keep the updated `wtrproject.toml` and tell the user to run `cpkg sync` online to apply the change.
- `cpkg add -I` should refresh the package index as soon as the command starts, before rendering the picker, and should reuse that refreshed index for the confirmed selection.
- Network-backed operations should surface live execution status to the user; prefer a temporary, bounded terminal log panel over silent background work.
- `cpkg add --offline` and `cpkg sync --offline` should resolve dependencies from the project-local or cached package index without refreshing the remote index.
- In offline mode, existing submodules should use locally cached repository state only; skip fetch and pull operations.
- In offline mode, when a newly required repository is not yet registered as a submodule, attempt to register it without fetching repository data; if the installed Git does not support that workflow, report that `--offline` cannot be used for that repository yet.

### Removal Semantics
- When `cpkg add -I` only removes packages and adds no new direct dependency, it should:
  - update `wtrproject.toml`
  - regenerate project links locally
  - remove any managed submodule repositories that are no longer required
  - avoid fetching or syncing retained submodules
- `cpkg remove` should:
  - update `wtrproject.toml`
  - locally regenerate the project integration file
  - remove any managed submodule repositories that are no longer required
  - avoid fetching or synchronizing retained submodules

## Project-Side Resolution Rules

### STM32CubeMX Repository Expectations
Project-side commands operate on STM32CubeMX firmware repositories only.

- Run project commands from the Git repository root.
- The project root must contain exactly one applicable `*.ioc` file unless the user passes `--ioc`.
- Managed driver repositories live under `Modules/` and are tracked as Git submodules.
- Generated `cmake/wtr_modules.cmake` should call `add_subdirectory(...)` only for resolved package directories in the active dependency chain; do not pull unrelated package directories from the same repository into the build.
- Synchronization always pulls the latest upstream state and forces managed submodules to track the `main` branch.
- Direct project dependencies are stored in `wtrproject.toml`; there is currently no lockfile.
- Package versions are currently non-semantic for resolution purposes; treat the latest indexed package as the only selectable version.

### Package Index Lookup Order
Prefer the existing resolution order implemented by the code:

1. `wtrproject.toml` `[index]` overrides
2. project-local `cpkg_index.json`
3. remote default index with cache under `~/.cpkg/cpkg_index.json`

The current default remote index is the WTR package index on GitHub. Keep index-loading behavior configurable and avoid hard-coding new lookup paths in unrelated modules.

### Mirror Source Configuration
- Global mirror configuration should live under the user-level `cpkg` config and distinguish index sources from org sources.
- Global index sources may contain multiple candidates and should be tried in order when project-level index overrides are absent.
- Global index sources should also be manageable through `cpkg config index ...`, with order-aware add, set, remove, or move operations instead of requiring manual file edits.
- Global org sources are named entries that can be updated through `cpkg config` without forcing users to hand-edit the whole config file.
- The global config file may optionally choose one named global org source as the default when a project does not explicitly select an org source.
- Org sources must support a default pull protocol (`ssh` or `https`) in addition to their remote base configuration.
- If the global config file does not exist, runtime behavior should use built-in defaults without creating any file.
- Commands that modify the global config must require the file to exist already and should instruct the user to run a dedicated create/init command first.
- The dedicated global-config create/init command should write an explicit template that includes both the default org source and the default index source.
- Project manifests may define at most one project-local index source and at most one project-local org source override.
- Project-local org configuration may override the protocol used for that project without mutating global defaults.

## Development Workflow

### Build, Test, and Help Commands
- `cargo run -- --help` shows the full CLI surface.
- `cargo run -- init --help` inspects project initialization options.
- `cargo run -- add --help` inspects dependency-add and sync options.
- `cargo run -- sync --help` inspects submodule synchronization options.
- `cargo run -- package --help` inspects package-authoring commands.
- `cargo test --offline` runs unit and doc tests without network access.
- `cargo fmt --check` verifies formatting before review.

Use targeted `cargo run --offline -- <command> --help` checks when editing CLI docs or argument parsing.
When a change needs to fetch a new crate, refresh `Cargo.lock`, or otherwise update dependencies, it is acceptable to run the smallest non-offline Cargo command that unblocks the task.

### Coding Style
- Follow default Rust style:
  - 4-space indentation
  - `snake_case` for functions and modules
  - `PascalCase` for types
  - concise error messages with `anyhow::Context`
- Prefer small modules with clear responsibilities.
- Keep project-side and package-side logic separated instead of reintroducing monolithic modules.
- Reuse shared interactive or sync-option code instead of duplicating command-specific implementations.
- Keep manifest migration logic versioned and composable so future format upgrades only add new steps.

### Cross-Platform Expectations
- Maintain compatibility with Linux and Windows.
- Avoid Unix-only shell or path assumptions in Rust code.
- Normalize generated and discovered paths where needed so tests pass on both platforms.
- Prefer standard-library filesystem and process APIs over shell-specific behavior.
- Local development should not depend on MSVC-specific tooling.
- CI may use the default Windows toolchain provided by GitHub-hosted runners.

### Testing
- Place unit tests next to the code they validate using `#[cfg(test)] mod tests`.
- Cover `wtrproject.toml` behavior, package-index loading, dependency resolution, submodule and integration generation logic, and `cpkg.toml` migrations.
- Add focused regression tests for CLI parsing changes when behavior is non-trivial.
- Run the most targeted checks first, then broader validation such as `cargo test --offline`.
- If dependency changes require network access or a lockfile refresh, use the smallest non-offline Cargo command that unblocks verification, then resume targeted checks.

## Contribution Conventions

### Commits and Pull Requests
- Keep commits scoped to one change.
- Use commit messages in `type(scope): content` form, for example `docs(cli): expand command help text`.
- Disable GPG signing for local commits with `git -c commit.gpgsign=false commit ...`.
- PRs should explain user-visible impact, list verification commands run, and include sample CLI output when command behavior changes.

### Tooling Notes
- Prefer updating package metadata through `cpkg package init`, `cpkg package generate`, and `cpkg package create` instead of editing generated outputs by hand.
- Prefer updating project dependency state through `cpkg init`, `cpkg add`, `cpkg remove`, and `cpkg sync` instead of editing generated integration files by hand.
- If the user's requirements change and the new requirement should be captured in `AGENTS.md`, update `AGENTS.md` immediately before continuing the implementation.
