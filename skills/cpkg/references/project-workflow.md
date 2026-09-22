# Firmware Project Workflow

Use this workflow when the task is to manage dependencies in an STM32CubeMX firmware repository.

## Determine Whether The Project Is Managed

- Not every STM32CubeMX repository is a `cpkg` project.
- If `wtrproject.toml` exists at the repository root, treat the repository as already managed.
- If `wtrproject.toml` does not exist, only use this workflow when the user explicitly asks to initialize the repository as a `wtrproject`.

## Preconditions

- Run commands from the firmware repository root.
- The root should contain exactly one applicable `.ioc` file unless the task passes `--ioc`.
- After initialization, `./Modules` is managed as Git submodules by `cpkg`.
- `cmake/wtr_modules.cmake` is generated output.

## Main Commands

### Initialize a project

Use one of these forms:

```bash
cpkg init --ioc MyBoard.ioc
cpkg init --name hero_chassis --ioc Hero.ioc
cpkg init -I
cpkg init -Iu
```

Effects:
- Create or overwrite `wtrproject.toml`.
- Bind the project to an `.ioc` file.
- Optionally choose initial direct dependencies interactively; `cpkg init -I` reuses the project-local or cached index, and `-u`/`--update-index` refreshes it before the picker.
- Start `cpkg` ownership of `./Modules` for this repository.

Integration rule after initialization:
- Do not ask the user to add module repositories manually.
- Do not ask the user to hand-write `add_subdirectory(...)` calls for `./Modules`.
- Tell the user to include the generated `cmake/wtr_modules.cmake` and link the target through the helper functions.

### List available packages

```bash
cpkg list
cpkg list -u
cpkg list --offline
```

- Default behavior reuses the project-local or cached index; it only downloads when no copy exists.
- `-u`/`--update-index` refreshes the remote index first.
- `--offline` never touches the network and fails when no local or cached index exists.

### Add direct dependencies

```bash
cpkg add MotorDrivers::DJI bsp::CANDriver
cpkg add -I
cpkg add -Iu
cpkg add --offline MotorDrivers::DJI
cpkg add MotorDrivers::DJI --submodule-protocol https
```

Effects:
- Update `wtrproject.toml`.
- Resolve direct and transitive dependencies against the project-local or cached index, unless `-u`/`--update-index` refreshes it first.
- Synchronize the required repositories under `Modules/` unless the run is offline or blocked by network limits.
- Regenerate `cmake/wtr_modules.cmake`.
- Regenerate `CMakeLists.txt` for every resolved package from its `cpkg.toml`. That file is derived output; driver repositories exclude it through `.gitignore` and must not commit it.

Offline rule:
- `cpkg add --offline` may still record the dependency in `wtrproject.toml`.
- If a new repository cannot be applied without fetching data, tell the user to run `cpkg sync` online later.

### Remove direct dependencies

```bash
cpkg remove MotorDrivers::DJI
cpkg remove MotorDrivers::DJI bsp::CANDriver
```

Effects:
- Update `wtrproject.toml`.
- Regenerate `cmake/wtr_modules.cmake` locally.
- Regenerate `CMakeLists.txt` for every remaining resolved package from its `cpkg.toml`.
- Remove managed submodule repositories that are no longer required.
- Do not fetch or resync retained repositories.

### Synchronize dependencies

```bash
cpkg sync
cpkg sync -u
cpkg sync --offline
cpkg sync --submodule-protocol ssh
```

Effects:
- Reuse the project-local or cached package index; `-u`/`--update-index` refreshes the remote index before resolving, and a failed refresh aborts the command instead of using the stale cache.
- Re-resolve the active dependency graph.
- Synchronize required repositories under `Modules/`.
- Regenerate `cmake/wtr_modules.cmake`.
- Regenerate `CMakeLists.txt` for every resolved package from its `cpkg.toml`; packages without a `cpkg.toml` are warned about and skipped.

## CMake Integration

After a successful sync, include the generated file from the root `CMakeLists.txt` and link packages through the helper functions:

```cmake
include(cmake/wtr_modules.cmake)
wtr_link_packages(app_target)
```

Use `wtr_link_packages_public(...)` when the target should re-export the dependency linkage.

Do not tell the user to populate `./Modules` manually or to wire module repositories by hand. `cpkg` owns both the repository checkout and the generated CMake integration once the project is initialized.

## Working Rules

- Prefer changing dependencies through `cpkg init`, `cpkg add`, `cpkg remove`, and `cpkg sync` instead of hand-editing generated integration files.
- Prefer editing the `.ioc` file and regenerating CubeMX output instead of patching generated HAL files.
- Treat `UserCode/` as the application-owned code area. Avoid inspecting `Core/`, `Drivers/`, or `Middlewares/` unless the user explicitly asks.
