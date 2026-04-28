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

Use `-f` only when intentionally overwriting an existing generated `CMakeLists.txt`.

### Regenerate package build files

```bash
cpkg package generate
```

Effects:
- Regenerate `CMakeLists.txt` from the local `cpkg.toml`.

## Maintenance Rules

- After adding, removing, or renaming package source files, rerun `cpkg package generate`.
- If the public package surface or dependency list changes, update `cpkg.toml` and regenerate.
- Do not hand-edit generated `CMakeLists.txt` unless the task is explicitly about changing generator behavior.
- If the repository maintains an aggregate package index elsewhere, regenerate that index with the appropriate `cpkg` workflow instead of editing the generated index by hand.
