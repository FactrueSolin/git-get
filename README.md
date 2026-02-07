# git-get

`git-get` 是一个用 Rust 编写的命令行工具，用于**从 GitHub 仓库抓取指定分支下的某个子目录**到本地目标路径。

它的核心目标是：**不污染当前工作目录的 Git 仓库结构**——所有与 Git 仓库相关的操作都在系统临时目录中完成，最终只把你指定的子目录内容复制出来。

## 项目介绍

在日常开发中，经常会遇到“只想拿某个仓库里的一个子目录（例如 examples / 模板 / 配置）”的情况。直接 `git clone` 会带来完整仓库与 `.git` 元数据；在已有仓库里操作还可能造成目录结构被污染。

`git-get` 的做法是：

1. 在**临时目录**中初始化仓库并配置 **sparse-checkout**；
2. 只拉取你需要的子目录（浅克隆 `--depth=1`）；
3. 将该子目录递归复制到你的目标路径（复制时跳过 `.git`）；
4. 临时目录在程序退出时自动清理。

## 本地安装（Cargo）

本项目是一个标准的 Rust Cargo 项目，你可以使用 `cargo install` 将可执行文件安装到系统路径。

### 1) 前置条件

- 已安装 Rust 工具链（含 `cargo`）
- 系统可用的 `git` 命令（`git-get` 通过调用系统 `git` 完成克隆与 sparse-checkout）

可先确认：

```bash
git --version
cargo --version
```

### 2) 使用 `cargo install --path .` 安装到本机

在项目根目录执行：

```bash
cargo install --path .
```

该命令会将本项目构建出的可执行文件（`git-get`）安装到 Cargo 的 bin 目录，通常为：

- macOS / Linux：`~/.cargo/bin`

请确保该目录已加入 `PATH`，例如（以 zsh 为例）：

```bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

安装完成后，你应该可以直接运行：

```bash
git-get --help
```

### 3) 最小可执行示例（复制即可运行）

下面示例会把指定仓库中的子目录下载到本地 `./example-servers`：

```bash
git-get https://github.com/modelcontextprotocol/rust-sdk/tree/main/examples/servers -d ./example-servers
```

如果你更偏好“分散参数”形式，也可以：

```bash
git-get --repo modelcontextprotocol/rust-sdk --branch main --path examples/servers --dest ./example-servers
```

> 注意：为避免误覆盖本地文件，`--dest` 目标路径必须**不存在**或是一个**空目录**；否则程序会直接报错退出。

## 项目功能概览

### 1) 支持两种输入方式

- **URL 模式（推荐）**：直接传入 GitHub 目录 URL（通常是 `/tree/<branch>/...`）
  
  ```bash
  git-get https://github.com/owner/repo/tree/main/path/to/dir -d ./dest
  ```

- **分散参数模式**：使用 `--repo/--branch/--path/--dest`
  
  ```bash
  git-get --repo owner/repo --branch main --path path/to/dir --dest ./dest
  ```

### 2) 仅抓取子目录（sparse-checkout + 浅克隆）

- 在临时目录中执行 `git init / git remote add / git fetch --depth=1` 并启用 sparse-checkout
- 只拉取你指定的子目录路径，减少下载体积与耗时

### 3) 复制结果不包含 `.git` 元数据

复制时会跳过 `.git` 目录，确保输出结果是“普通文件夹”，不会在目标目录里携带上游仓库元数据。

### 4) 目标路径安全检查

为了降低误操作导致的数据丢失风险：

- 若目标路径不存在：允许写入
- 若目标路径存在但为空目录：允许写入
- 若目标路径存在且非空：拒绝执行并报错

### 5) 可选：自动更新 `.gitignore`

如果当前工作目录存在 `.gitignore`，程序会尝试把 `--dest` 目标路径追加到其中，并附带注释 `# Added by git-get`。

### 6) 说明与限制

- 当前版本预留了 `--token` 参数，但暂未用于鉴权（后续可扩展用于私有仓库场景）。
- 工具以“子目录抓取”为目标，建议使用 `.../tree/<branch>/...` 形式的目录 URL；若传入指向单个文件的 URL，可能无法按预期工作。

