---
name: cpkg
description: "Use when an agent needs to operate the `cpkg` CLI for WTR-managed STM32CubeMX repositories or reusable driver-package repositories: detect managed projects by the presence of `wtrproject.toml`, initialize a repository as a `wtrproject` only when the user explicitly asks for `cpkg init`, manage dependencies and `Modules/`, configure `~/.cpkg/config.toml`, or create/migrate/generate `cpkg.toml` and package `CMakeLists.txt`."
---

# Use cpkg

## Start Here

- Determine whether `cpkg` applies before doing anything else:
  - If `wtrproject.toml` exists, treat the repository as a WTR-managed `cpkg` project.
  - If `wtrproject.toml` does not exist, do not assume the project is managed by `cpkg`.
  - Only run `cpkg init` when the user explicitly asks to initialize the repository as a `wtrproject`.
- Pick the matching workflow first:
  - Firmware project workflow: manage `wtrproject.toml`, `Modules/`, or `cmake/wtr_modules.cmake` in a WTR-managed STM32CubeMX repository.
  - Driver package workflow: manage `cpkg.toml` and generated `CMakeLists.txt` inside a reusable driver package.
  - Global config workflow: manage mirror and org settings in `~/.cpkg/config.toml`.
- Run `cpkg --help` and the specific subcommand `--help` before using an unfamiliar command.
- Prefer `cpkg ...` in user repositories. Prefer `cargo run --offline -- ...` only when validating the CLI from the `cpkg` source tree itself.

## Operating Rules

- Run project-side commands from the firmware repository root.
- Expect exactly one applicable `.ioc` file unless the task passes `--ioc`.
- Do not present `cpkg` as a generic STM32CubeMX requirement. A plain STM32CubeMX repository is outside `cpkg` until it is initialized as a `wtrproject`.
- Treat `UserCode/` as the project-specific code area. Avoid hand-editing CubeMX-generated `Core/`, `Drivers/`, or `Middlewares/` unless the user explicitly asks.
- Let `cpkg` own generated outputs. Do not hand-edit `cmake/wtr_modules.cmake` or a package `CMakeLists.txt` unless the task is explicitly about generator development.
- After `cpkg init`, treat `./Modules` as `cpkg`-managed. Do not ask the user to add module repositories manually.
- Prefer `--offline` when the user wants cache-only behavior or the network is unavailable.
- Remember the offline write semantics: `cpkg add --offline` can still update `wtrproject.toml` even if a new repository cannot be fetched until a later online `cpkg sync`.

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

## Report Clearly

- When operating on a firmware project, report changes to `wtrproject.toml`, `Modules/`, and `cmake/wtr_modules.cmake`.
- When operating on a driver package, report changes to `cpkg.toml`, generated `CMakeLists.txt`, and any discovered source/header coverage changes.
- When an offline run cannot fully apply a new dependency, state explicitly that the manifest was updated and that `cpkg sync` must be run online later.
- When initializing a project, state explicitly that `cpkg` has taken ownership of `./Modules` and that users should integrate by including the generated `.cmake` and linking targets instead of wiring modules manually.
- When changing the `cpkg` source repository itself, verify the relevant `--help` output after CLI edits.
