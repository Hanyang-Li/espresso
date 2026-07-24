# curl 一键安装 + GitHub Releases 自动发布

日期: 2026-07-13
状态: 已确认

## 背景

`espresso` 是一个 macOS-only 的 Rust CLI(依赖 CoreFoundation / `launchctl`)。
目前发布靠 `dist/` 里手动打的 tar.gz,没有标准安装方式。目标是提供业界通行的
`curl -fsSL <url> | sh` 一键安装体验。

关键认知:**不需要任何下载服务器**。GitHub 自带两项免费能力即可:
- **GitHub Releases** 托管编译好的二进制 tar.gz。
- **raw.githubusercontent.com** 直接把仓库里的 `install.sh` 当静态文件返回。

## 决策

- **目标架构**: 仅 Apple Silicon(`aarch64-apple-darwin`)。Intel Mac 明确报错。
- **发布方式**: GitHub Actions,push `v*` tag 时自动构建并发布。

## 产物

### 1. `install.sh`(仓库根目录)

用户命令:
```sh
curl -fsSL https://raw.githubusercontent.com/Hanyang-Li/espresso/main/install.sh | sh
```

脚本逻辑(`set -euo pipefail`):
1. 校验 `uname -s` = `Darwin` 且 `uname -m` = `arm64`,否则报错退出(提示暂只支持 Apple Silicon)。
2. 解析版本:默认用 GitHub `releases/latest/download/` 跳转拿最新版;支持
   `ESPRESSO_VERSION=v0.2.0` 锁定某版本(走 `releases/download/<tag>/`)。**不调用 GitHub API**,避免限流。
3. 下载到 `mktemp -d` 临时目录(`trap` 退出清理):`espresso-aarch64-apple-darwin.tar.gz` 及同名 `.sha256`。
4. `shasum -a 256 -c` 校验完整性,失败退出。
5. 解压,安装到 `/usr/local/bin/espresso`(默认,可用 `ESPRESSO_INSTALL_DIR` 覆盖);目标目录不可写时自动 `sudo`。
6. `chmod +x`,打印安装版本 + 下一步提示(`sudo espresso daemon install`)。

选 `/usr/local/bin` 的原因:它在 PATH 上且路径稳定 —— `daemon install` 写的
LaunchDaemon plist 会记录二进制绝对路径,不能装在会变动的位置。

### 2. `.github/workflows/release.yml`

- 触发: push tag 匹配 `v*`。
- Runner: `macos-14`(原生 Apple Silicon,直接编译,无需交叉编译)。
- `permissions: contents: write`(创建 Release 需要)。
- 步骤: checkout → 装 Rust(`dtolnay/rust-toolchain@stable`)→ `cargo build --release --locked`
  → 打包 `espresso-aarch64-apple-darwin.tar.gz` + `shasum -a 256` 生成 `.sha256`
  → `softprops/action-gh-release@v2` 创建 Release 并上传两个文件。
- 资产名不带版本号,配合 `/latest/download/` 跳转,使脚本免解析版本。

### 3. README 安装章节

顶部新增 `curl | sh` 一行命令,以及手动下载/校验/安装说明。

## 关键点

- **无需签名/公证**: 通过 `curl` 下载的二进制不会被打上 `com.apple.quarantine`,
  不触发 Gatekeeper,可直接运行。
- **发版流程**: 改代码 → 更新 `Cargo.toml` 版本 → `git tag vX.Y.Z && git push --tags`
  → CI 自动产出 Release。
- 现有 `dist/` 目录手动打的包清理,改由 CI 产出。

## 非目标(YAGNI)

- Intel / universal 二进制。
- Homebrew tap、代码签名/公证、自动更新。
- Windows / Linux 支持。

## 变更记录 (2026-07-24, v1.0.0)

默认安装位置由 `/usr/local/bin` 改为 **`~/.local/bin`**,并让安装期**默认不再获取
sudo**。理由:espresso 是面向个人用户的工具,装进用户自己的 `~/.local/bin` 更契合其
定位,也免去每次安装都要输密码。借鉴同作者 `agent-limit` 项目的 `install.sh`。

本次变更**取代**上文 §产物.1 第 5 步与"选 `/usr/local/bin` 的原因"一段,具体为:

- **默认目录**:`INSTALL_DIR` 默认 `$HOME/.local/bin`(仍可用 `ESPRESSO_INSTALL_DIR` 覆盖)。
- **sudo**:先无权限 `mkdir -p` 并检测可写性;`~/.local/bin` 在 `$HOME` 下恒可写,
  故默认零 sudo。**仅当**被覆盖到不可写目录(如 `/usr/local/bin`)时才回退 `sudo`。
- **PATH 处理**(借鉴 `agent-limit`):安装后若目标目录不在 `PATH` 上,按当前 shell 幂等
  写入 rc(zsh→`.zshrc`、bash→`.bash_profile`、fish→`config.fish`,以
  `# added by espresso installer` marker 防重复);未知 shell 或写入失败则打印手动说明。
  新增 `ESPRESSO_NO_MODIFY_PATH=1` 可跳过改 rc、只打印手动 PATH 说明。
- **不变**:LaunchDaemon plist 路径(`/Library/LaunchDaemons/local.espresso.daemon.plist`)、
  socket(`/var/run/espresso.sock`)、资产命名、强制 checksum 校验、以及 `daemon install`
  仍需 sudo(写系统级 plist)。同目录原子 `rename` 安装逻辑保留(避免替换运行中二进制的 `ETXTBSY`)。

> 注:原选 `/usr/local/bin` 的理由是"路径稳定,plist 记录二进制绝对路径"。`~/.local/bin`
> 同样是稳定绝对路径,该顾虑不受影响。
