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
- 支持全局配置多个索引镜像源与命名 org 镜像源
- 交互式编辑项目直接依赖
- 为驱动包生成或迁移 `cpkg.toml`
- 为驱动包自动生成 `CMakeLists.txt`

## 安装

`cpkg` 是终端工具，不是图形界面程序；不过对 Windows 用户，安装步骤尽量可以通过图形界面完成。

README 不提供 Linux 安装帮助。Linux 用户如需使用，请按自己的环境自行准备 Rust / Cargo 并构建。

### Windows：从 Release 安装

适合只想直接使用 `cpkg.exe` 的用户。

1. 打开仓库 Release 页面，下载 `cpkg-<version>-x86_64-pc-windows-msvc.zip`
2. 在资源管理器中右键压缩包，选择“全部解压”，解压到一个你自己容易找到的位置，例如 `C:\Tools\cpkg`
3. 打开解压后的文件夹，确认其中有 `cpkg.exe`
4. 如果 Windows 显示文件来自互联网并阻止运行：
   - 右键 `cpkg.exe`
   - 打开“属性”
   - 如果底部有“解除锁定”或类似选项，勾选后点击“确定”
5. 用图形界面把该目录加入当前用户的 `Path`：
   - 在开始菜单搜索“编辑账户的环境变量”或“环境变量”
   - 打开后，在“用户变量”里选中 `Path`
   - 点击“编辑” → “新建”
   - 填入 `cpkg.exe` 所在目录，例如 `C:\Tools\cpkg`
   - 依次点击“确定”保存
6. 关闭并重新打开 **Windows Terminal**、PowerShell 或 `cmd`，执行：

   ```powershell
   cpkg --version
   ```

### 从源码构建

适合已经具备 Rust 开发环境、需要参与开发或想使用最新代码的用户。

```bash
git clone https://github.com/HITSZ-WTRobot-Packages/cpkg.git
cd cpkg
cargo build --release
```

构建完成后，可执行文件位于：

- Windows：`target\release\cpkg.exe`
- 其他平台：对应 `target/release/` 下的 `cpkg`

如果你只是想在 Windows 上安装使用，优先选择上面的 Release 压缩包方式。

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
- `--submodule-protocol` 支持 `ssh` 和 `https`
- 如果项目未显式设置 `[org].name`，则会优先使用全局 `config.toml` 中的 `default_org`
- 如果不显式传入 `--submodule-protocol`，则优先使用项目 `[org]` 的 `protocol`，再回退到命名全局 org 源的 `default_protocol`，最后回退到内置默认 `ssh`

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

如果你希望项目只为当前仓库指定一个 org 源，也可以添加：

```toml
[org]
name = "wtr-github"
protocol = "https"
```

或者直接在项目内定义一套独立 org 前缀：

```toml
[org]
ssh_base = "git@example.com:robot-packages"
https_base = "https://example.com/robot-packages"
protocol = "ssh"
```

包索引加载顺序为：

1. `wtrproject.toml` 中的 `[index]` 配置
2. 项目根目录的 `cpkg_index.json`
3. `~/.cpkg/config.toml` 中按顺序声明的全局 `[[index]]`
4. 默认远程索引与本地缓存

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
- `WTR_MANAGED_PACKAGE_DIRS`
- `WTR_DIRECT_PACKAGE_TARGETS`
- `WTR_RESOLVED_PACKAGE_TARGETS`
- `wtr_project_dependencies`

生成的 `cmake/wtr_modules.cmake` 只会对当前依赖链上的包目录调用 `add_subdirectory(...)`，不会把同一仓库里未被依赖的其他包一起编译。

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
version = "0.1.0"
dependencies = ["bsp::CANDriver"]
```

字段说明：

- `name`：包短名
- `pkgname`：完整目标名，推荐使用 `Namespace::Name`
- `version`：包版本，默认生成 `0.1.0`
- `dependencies`：此包依赖的其他包目标

## 常见命令速查

```bash
cpkg --help
cpkg init --help
cpkg add --help
cpkg remove --help
cpkg sync --help
cpkg config --help
cpkg config init --help
cpkg config index --help
cpkg config org --help
cpkg package --help
cpkg package init --help
cpkg package generate --help
cpkg package create --help
```

## 全局镜像配置

`cpkg` 支持用户级全局配置文件，默认位于 `~/.cpkg/config.toml`。

- 如果这个文件**不存在**，`cpkg` 会继续使用**内部默认配置**
- 如果你要修改全局配置，需要先显式创建文件：

```bash
cpkg config init
```

如需覆盖已有全局配置文件：

```bash
cpkg config init --force
```

`cpkg config init` 生成的默认模板会同时带上：

- 一个默认的 `[[index]]`，指向内置 WTR 包索引
- 一个默认的命名 `[[org]]`
- `default_org` 指向这个默认 org

可以先查看当前配置：

```bash
cpkg config show
```

### 全局索引源

全局允许配置多个索引源，按顺序尝试使用；只有在项目没有显式 `[index]` 配置、项目根目录也没有 `cpkg_index.json` 时才会进入这条全局回退链。

如果 `config.toml` 还没有创建，`cpkg config index list` 会显示当前使用的是内置默认索引；而任何 `add/set/remove/move` 写操作都会提示先运行 `cpkg config init`。

也可以通过 `cpkg config index` 管理顺序：

```bash
cpkg config index list
cpkg config index add --url https://mirror-a.example.com/cpkg_index.json
cpkg config index add --url https://mirror-b.example.com/cpkg_index.json --position 1
cpkg config index set 2 --path /tmp/cpkg_index.json
cpkg config index move 2 1
cpkg config index remove 1
```

其中位置参数都是 **1-based**，也就是第一个源的位置是 `1`。

示例：

```toml
format_version = 1

[[index]]
url = "https://raw.githubusercontent.com/HITSZ-WTRobot-Packages/index/refs/heads/main/cpkg_index.json"
cache_path = "cpkg_index.json"

[[index]]
url = "https://mirror-a.example.com/cpkg_index.json"
cache_path = "indexes/mirror-a.json"

[[index]]
url = "https://mirror-b.example.com/cpkg_index.json"
cache_path = "indexes/mirror-b.json"
```

每个全局索引源都可以使用：

- `path`：本地索引文件路径
- `url`：远程索引地址
- `cache_path`：远程索引缓存路径，仅在设置了 `url` 时有效

### 全局 org 源

全局允许配置多个命名 org 源。每个 org 源可以设置 SSH / HTTPS 两套仓库前缀，以及默认拉取协议；同时还可以用 `default_org` 指定**项目未显式选择时**默认使用哪个命名 org。

示例：

```toml
format_version = 1

default_org = "wtr-github"

[[index]]
url = "https://raw.githubusercontent.com/HITSZ-WTRobot-Packages/index/refs/heads/main/cpkg_index.json"
cache_path = "cpkg_index.json"

[[org]]
name = "wtr-github"
ssh_base = "git@github.com:HITSZ-WTRobot-Packages"
https_base = "https://github.com/HITSZ-WTRobot-Packages"
default_protocol = "ssh"

[[org]]
name = "wtr-mirror"
https_base = "https://gitee.com/example/wtr-packages"
default_protocol = "https"
```

也可以通过命令更新指定名称的 org 源：

```bash
cpkg config org set wtr-github \
  --ssh-base git@github.com:HITSZ-WTRobot-Packages \
  --https-base https://github.com/HITSZ-WTRobot-Packages \
  --default-protocol ssh

cpkg config org set wtr-github --default-protocol https
cpkg config org remove wtr-github
cpkg config org default set wtr-github
cpkg config org default clear
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

因为 `cpkg` 是纯命令行工具。即使安装过程可以主要通过图形界面完成，实际使用时仍需要在终端里执行 `cpkg init`、`cpkg add`、`cpkg sync` 等命令，而不是双击运行。

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
