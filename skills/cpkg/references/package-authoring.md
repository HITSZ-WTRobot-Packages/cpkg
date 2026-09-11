# Driver Package Workflow

Use this workflow when the task is to author or maintain a reusable driver package rather than a consuming firmware project.

## Package Model

- A directory containing `cpkg.toml` is one package.
- Large driver repositories may contain multiple packages.
- Package target names should match the names declared in `cpkg.toml`.
- Downstream firmware projects add the repository with `add_subdirectory(...)` and then link only the required package targets.

## Main Commands

### Create a new package scaffold

```bash
cpkg package create MyDriver
```

Effects:
- Create a new package directory.
- Scaffold `include/` and `src/`.

### Create or migrate package metadata

```bash
cpkg package init MotorDrivers::DJI
cpkg package init MotorDrivers::DJI --deps bsp::CANDriver
cpkg package init MotorDrivers::DJI -f
```

Effects:
- Create or migrate `cpkg.toml`.
- Generate `CMakeLists.txt`.
- Record direct package dependencies when `--deps` is provided.

Run it from the package directory. Use `-f` only when intentionally overwriting an existing generated `CMakeLists.txt`.

### Regenerate package build files

```bash
cd MotorDrivers/motors/DJI
cpkg package generate
```

Effects:
- Regenerate `CMakeLists.txt` in the current directory from its `cpkg.toml`.

## Derived Build Files

- A package `CMakeLists.txt` is derived output of `cpkg.toml`: driver repositories must exclude it through `.gitignore` and must not commit it.
- Consuming firmware projects do not run `cpkg package generate`. Project-side `cpkg sync` (also reached through `cpkg add` and `cpkg remove`) regenerates `CMakeLists.txt` for every resolved package, including transitive dependencies.
- If a package directory has no `cpkg.toml`, project-side sync warns and skips it instead of failing.
- When cpkg generates a package `CMakeLists.txt` that is still tracked by git, it warns that the file should be added to `.gitignore`; it never edits `.gitignore` or runs `git rm --cached`.

## Maintenance Rules

- After adding, removing, or renaming package source files, rerun `cpkg package generate` locally, or rerun project-side `cpkg sync` when the package is consumed by a firmware project.
- If the public package surface or dependency list changes, update `cpkg.toml` and regenerate.
- Do not hand-edit the generated `CMakeLists.txt` unless the task is explicitly about changing generator behavior.
- If the repository maintains an aggregate package index elsewhere, regenerate that index with the appropriate `cpkg` workflow instead of editing the generated index by hand.
