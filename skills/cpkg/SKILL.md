---
name: cpkg
description: "Use when an agent needs to operate the `cpkg` CLI for WTR-managed STM32CubeMX repositories, reusable driver-package repositories, global configuration, or `cpkg` self-update: detect managed projects by `wtrproject.toml`, initialize only on explicit request, manage dependencies and `Modules/`, maintain package metadata and generated CMake, configure mirrors, or update the installed CLI from official GitHub Releases."
---

# Use cpkg

## Start Here

- For firmware project work, determine whether `cpkg` applies before doing anything else:
  - If `wtrproject.toml` exists, treat the repository as a WTR-managed `cpkg` project.
  - If `wtrproject.toml` does not exist, do not assume the project is managed by `cpkg`.
  - Only run `cpkg init` when the user explicitly asks to initialize the repository as a `wtrproject`.
- Self-update is independent of project detection: `cpkg update` does not require `wtrproject.toml` and may run from any directory.
- Pick the matching workflow first:
  - Firmware project workflow: manage `wtrproject.toml`, `Modules/`, or `cmake/wtr_modules.cmake` in a WTR-managed STM32CubeMX repository.
  - Driver package workflow: manage `cpkg.toml` and generated `CMakeLists.txt` inside a reusable driver package.
  - Global config workflow: manage mirror and org settings in `~/.cpkg/config.toml`.
  - Self-update workflow: update the installed `cpkg` executable from the official GitHub Releases.
- Run `cpkg --help` and the specific subcommand `--help` before using an unfamiliar command.
- Prefer `cpkg ...` in user repositories. Prefer `cargo run --offline -- ...` only when validating the CLI from the `cpkg` source tree itself.

## Operating Rules

- Run project-side commands from the firmware repository root.
- Expect exactly one applicable `.ioc` file unless the task passes `--ioc`.
- Do not present `cpkg` as a generic STM32CubeMX requirement. A plain STM32CubeMX repository is outside `cpkg` until it is initialized as a `wtrproject`.
- Treat `UserCode/` as the project-specific code area. Avoid hand-editing CubeMX-generated `Core/`, `Drivers/`, or `Middlewares/` unless the user explicitly asks.
- Let `cpkg` own generated outputs. Do not hand-edit `cmake/wtr_modules.cmake` or a package `CMakeLists.txt` unless the task is explicitly about generator development.
- Treat a package `CMakeLists.txt` as derived output: project-side `cpkg sync` regenerates it from `cpkg.toml`, driver repositories exclude it through `.gitignore`, and it must not be committed.
- After `cpkg init`, treat `./Modules` as `cpkg`-managed. Do not ask the user to add module repositories manually.
- Prefer `--offline` when the user wants cache-only behavior or the network is unavailable.
- Remember the offline write semantics: `cpkg add --offline` can still update `wtrproject.toml` even if a new repository cannot be fetched until a later online `cpkg sync`.
- Index updates and package updates are separate: project commands reuse the project-local or cached index by default and must not re-download it unless the user passes `-u`/`--update-index` (supported by `cpkg add`, `cpkg add -I`, `cpkg sync`, `cpkg list`, and `cpkg init -I`).
- Without `-u`, the remote index is downloaded only when neither a project-local index nor a cached copy exists, so a first run needs no extra flag; pass `-u` when the user needs the latest indexed packages.
- `-u`/`--update-index` conflicts with `--offline`, and an explicit refresh failure aborts the command instead of falling back to the stale cached copy.
- Treat `cpkg update` as an online, direct update: it accesses GitHub, verifies the Release SHA-256 digest and staged binary version, and replaces the current executable without confirmation.
- Do not expect automatic elevation. Run it only when the current executable's installation location is writable, or use user-chosen appropriate privileges.

## Choose A Workflow

### Firmware Project Workflow

- Use [references/project-workflow.md](references/project-workflow.md) for `init`, `list`, `add`, `remove`, `sync`, and post-sync CMake integration.
- Use this path only when the repository is already a WTR-managed project or the user explicitly asks to initialize it as one.

### Driver Package Workflow

- Use [references/package-authoring.md](references/package-authoring.md) for `cpkg package create`, `cpkg package init`, and `cpkg package generate`.
- Use this path when the task is to author or maintain a reusable driver package rather than a consuming firmware project.

### Global Config Workflow

- Use [references/configuration.md](references/configuration.md) for `cpkg config ...`, index mirror order, named org sources, and protocol selection.
- Use this path when the user needs to change where package indexes or Git remotes are resolved from.

### Self-Update Workflow

- Run `cpkg update`; do not require or create `wtrproject.toml` for this workflow.
- Expect it to select the latest stable official GitHub Release for the current target, verify the required SHA-256 digest, and check the staged binary with `--version` before replacing the current executable in place.
- If the installed version is already current, report that status; after a successful update, report the installed version shown by the command.
- Keep this workflow separate from firmware dependencies, driver package metadata, and global mirror configuration.

## Report Clearly

- When operating on a firmware project, report changes to `wtrproject.toml`, `Modules/`, and `cmake/wtr_modules.cmake`, plus regenerated package `CMakeLists.txt` files under `Modules/`.
- When operating on a driver package, report changes to `cpkg.toml`, generated `CMakeLists.txt`, and any discovered source/header coverage changes. Remind the user that generated package `CMakeLists.txt` files are derived output excluded by `.gitignore`, not committed content.
- When an offline run cannot fully apply a new dependency, state explicitly that the manifest was updated and that `cpkg sync` must be run online later.
- When initializing a project, state explicitly that `cpkg` has taken ownership of `./Modules` and that users should integrate by including the generated `.cmake` and linking targets instead of wiring modules manually.
- When updating `cpkg` itself, report whether it was already current or updated and include the version reported by the command.
- When changing the `cpkg` source repository itself, verify the relevant `--help` output after CLI edits.
