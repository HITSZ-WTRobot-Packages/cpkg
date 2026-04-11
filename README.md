# cpkg

`cpkg` 是一个面向 STM32CubeMX 固件仓库的命令行包管理器，同时也提供 WTR 驱动包作者工具链。

它解决两类问题：

- **项目侧依赖管理**：在 STM32CubeMX 工程根目录维护 `wtrproject.toml`，解析包索引，拉取 `Modules/` 下的 Git submodule，并生成 `cmake/wtr_modules.cmake`
- **包侧元数据生成**：在驱动包目录维护 `cpkg.toml`，扫描源码/头文件并生成对应的 `CMakeLists.txt`

## 功能概览

- 管理项目直接依赖与传递依赖
- 从包索引解析包所属仓库与依赖关系
- 将驱动仓库同步到 `Modules/` Git submodule
- 生成 `cmake/wtr_modules.cmake` 供主工程引入
- 交互式编辑项目直接依赖
- 为驱动包生成或迁移 `cpkg.toml`
- 为驱动包自动生成 `CMakeLists.txt`

## 安装

### 方式一：从 Release 安装

适合不想本地安装 Rust 工具链的用户。

#### Linux

1. 从仓库的 Release 页面下载 `cpkg-<version>-x86_64-unknown-linux-gnu.tar.gz`
2. 解压：

   ```bash
   tar -xzf cpkg-<version>-x86_64-unknown-linux-gnu.tar.gz
   ```

3. 复制到你的 PATH 目录，例如：

   ```bash
   install -Dm755 cpkg-<version>-x86_64-unknown-linux-gnu/cpkg ~/.local/bin/cpkg
   ```

4. 验证：

   ```bash
   cpkg --version
   ```

#### Windows

`cpkg` 是终端工具，不是图形界面程序。Windows 用户请在 **Windows Terminal**、PowerShell 或 `cmd` 中使用它。

1. 从仓库的 Release 页面下载 `cpkg-<version>-x86_64-pc-windows-msvc.zip`
2. 打开 **PowerShell**，解压到自己的工具目录，例如：

   ```powershell
   $version = "v0.2.0"
   $installRoot = "$env:USERPROFILE\Tools"
   Expand-Archive ".\cpkg-$version-x86_64-pc-windows-msvc.zip" -DestinationPath $installRoot -Force
   $cpkgDir = Join-Path $installRoot "cpkg-$version-x86_64-pc-windows-msvc"
   ```

3. 将目录加入当前用户的 `Path`：

   ```powershell
   $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
   [Environment]::SetEnvironmentVariable("Path", "$userPath;$cpkgDir", "User")
   ```

4. 关闭并重新打开终端，然后验证：

   ```powershell
   cpkg --version
   ```

如果 Windows 拦截了可执行文件，可以执行：

```powershell
Unblock-File "$cpkgDir\cpkg.exe"
```

### 方式二：从源码构建

适合开发者或需要最新改动的用户。

```bash
git clone <repo-url> cpkg
cd cpkg
cargo build --release
```

请将 `<repo-url>` 替换为实际仓库地址。

构建完成后，可执行文件位于：

- Linux：`target/release/cpkg`
- Windows：`target\release\cpkg.exe`

也可以手动复制到 PATH 目录中使用。

## 依赖与前提

### 项目侧命令前提

- 在 **STM32CubeMX 工程 Git 仓库根目录** 运行
- 根目录需要存在且只存在一个适用的 `*.ioc` 文件；否则请显式传入 `--ioc`
- `Modules/` 由 `cpkg` 作为 Git submodule 管理
- 若使用默认远程索引，需要可用的网络环境
- 同步 submodule 需要本机安装 `git`

### 包作者命令前提

- 在驱动包目录运行
- 包目录通常包含 `include/`、`src/`、`cpkg.toml`
- `cpkg package generate` 会扫描当前目录下的 `.c/.cpp/.h/.hpp` 文件来生成 `CMakeLists.txt`

## 两种工作流

### 1. 项目侧：管理 STM32CubeMX 固件项目依赖

项目侧命令如下：

- `cpkg init`
- `cpkg add`
- `cpkg remove`
- `cpkg sync`

这套流程围绕 `wtrproject.toml` 工作。

### 2. 包侧：编写/维护驱动包

包侧命令如下：

- `cpkg package init`
- `cpkg package generate`
- `cpkg package create`

这套流程围绕 `cpkg.toml` 工作。

## 快速开始：项目侧工作流

假设你当前位于一个 STM32CubeMX 工程根目录，并且目录中已经有 `.ioc` 文件。

### 1. 初始化项目

```bash
cpkg init --ioc MyBoard.ioc
```

也可以交互式初始化依赖：

```bash
cpkg init -I
```

常用参数：

- `--name <NAME>`：手动指定项目名
- `--ioc <IOC>`：手动指定 `.ioc` 文件
- `-f, --force`：覆盖已有 `wtrproject.toml`
- `-I, --interactive`：进入交互式依赖选择

初始化后会生成 `wtrproject.toml`。

### 2. 添加直接依赖

```bash
cpkg add MotorDrivers::DJI bsp::CANDriver
```

如果你没有配置 Git SSH key，可以使用 HTTPS：

```bash
cpkg add MotorDrivers::DJI --submodule-protocol https
```

如果你更喜欢交互式选择：

```bash
cpkg add -I
```

说明：

- `cpkg add` 会更新 `wtrproject.toml`
- 如果有新增依赖，会同步 `Modules/` 下所需仓库
- 会生成或更新 `cmake/wtr_modules.cmake`
- `--submodule-protocol` 支持 `ssh` 和 `https`，默认是 `ssh`

### 3. 从项目中移除直接依赖

```bash
cpkg remove MotorDrivers::DJI
```

说明：

- `cpkg remove` 会更新 `wtrproject.toml`
- 会本地刷新 `cmake/wtr_modules.cmake`
- 会删除不再需要的受管仓库
- 不会重新同步保留中的 submodule

### 4. 重新同步依赖

```bash
cpkg sync
```

如果需要使用 HTTPS：

```bash
cpkg sync --submodule-protocol https
```

说明：

- 会刷新包索引
- 会解析直接依赖与传递依赖
- 会同步 `Modules/` 中需要的仓库
- 会生成 `cmake/wtr_modules.cmake`

## `wtrproject.toml` 示例

一个典型的项目清单如下：

```toml
format_version = 1

[project]
name = "hero_chassis"
ioc_file = "Hero.ioc"

[dependencies]
packages = [
    "MotorDrivers::DJI",
    "bsp::CANDriver",
]
```

如果你希望覆盖包索引来源，还可以添加：

```toml
[index]
path = "cpkg_index.json"
url = "https://example.com/cpkg_index.json"
cache_path = ".cpkg/cpkg_index.json"
```

包索引加载顺序为：

1. `wtrproject.toml` 中的 `[index]` 配置
2. 项目根目录的 `cpkg_index.json`
3. 默认远程索引与本地缓存

## 集成到 CMake

执行过 `cpkg sync` 后，会生成 `cmake/wtr_modules.cmake`。

在根 `CMakeLists.txt` 中引入：

```cmake
include(cmake/wtr_modules.cmake)
```

然后在你的固件目标创建之后链接包：

```cmake
wtr_link_packages(${PROJECT_NAME})
```

如果你需要 `PUBLIC` 作用域：

```cmake
wtr_link_packages_public(${PROJECT_NAME})
```

`cpkg` 还会在生成文件中创建：

- `WTR_MANAGED_REPOSITORIES`
- `WTR_DIRECT_PACKAGE_TARGETS`
- `WTR_RESOLVED_PACKAGE_TARGETS`
- `wtr_project_dependencies`

## 快速开始：包作者工作流

### 1. 创建新包目录

```bash
cpkg package create TrajectoryControl
```

这会创建：

- `TrajectoryControl/include/`
- `TrajectoryControl/src/`
- `TrajectoryControl/cpkg.toml`

### 2. 初始化或迁移 `cpkg.toml`

在驱动包目录中执行：

```bash
cpkg package init MotorDrivers::DJI --deps bsp::CANDriver
```

常用参数：

- `<PKGNAME>`：包目标名，例如 `MotorDrivers::DJI`
- `-d, --deps <DEPS>`：记录直接依赖
- `-f, --force`：覆盖已有 `CMakeLists.txt`

如果目录里已有旧格式 `cpkg.toml`，该命令会自动迁移到当前格式。

### 3. 重新生成 `CMakeLists.txt`

```bash
cpkg package generate
```

这个命令会读取本地 `cpkg.toml`，扫描当前目录并生成 `CMakeLists.txt`。

## `cpkg.toml` 示例

```toml
format_version = 1
name = "DJI"
pkgname = "MotorDrivers::DJI"
dependencies = ["bsp::CANDriver"]
```

字段说明：

- `name`：包短名
- `pkgname`：完整目标名，推荐使用 `Namespace::Name`
- `dependencies`：此包依赖的其他包目标

## 常见命令速查

```bash
cpkg --help
cpkg init --help
cpkg add --help
cpkg remove --help
cpkg sync --help
cpkg package --help
cpkg package init --help
cpkg package generate --help
cpkg package create --help
```

## 常见问题

### 为什么 `cpkg init` 失败并提示有多个 `.ioc` 文件？

因为项目根目录必须能唯一确定要绑定的 STM32CubeMX 工程。请使用：

```bash
cpkg init --ioc YourProject.ioc
```

### 为什么 `cpkg add` / `cpkg sync` 会涉及 Git submodule？

因为 `cpkg` 会把驱动仓库放到 `Modules/` 下，并用 Git submodule 管理这些受管仓库。

### 为什么推荐在 Windows Terminal / PowerShell 中使用？

因为 `cpkg` 是纯命令行工具。你需要在终端中执行 `cpkg init`、`cpkg add`、`cpkg sync` 等命令，而不是双击运行。

## 开发时常用检查

```bash
cargo run -- --help
cargo run -- init --help
cargo run -- add --help
cargo run -- sync --help
cargo run -- package --help
cargo test --offline
cargo fmt --check
```
