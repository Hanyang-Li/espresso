# espresso ☕

> 用命令行让你的 Mac 保持唤醒——合盖只是关闭屏幕,而不是睡眠。

[English](README.md) | **简体中文**

![license](https://img.shields.io/badge/license-MIT-blue.svg)
![platform](https://img.shields.io/badge/platform-macOS%20·%20Apple%20Silicon-lightgrey.svg)

`espresso` 是一个小巧的 macOS 命令行工具,用来阻止 Mac 进入睡眠。它分两层工作:

1. **防空闲睡眠**(始终生效,无需权限)——只要有 `espresso` 会话在运行,Mac
   就不会因为无操作而睡眠。类似 `caffeinate`,但作用域限定在会话内。
2. **合盖保持唤醒**(可选,一次性 `sudo` 配置)——安装一个小的 root 辅助进程后,
   即使**合上盖子**(屏幕关闭、不睡眠,电池供电下也一样)Mac 也保持唤醒。不装这个
   辅助进程时,合盖仍会让 Mac 睡眠。

## 特性

- 按**倒计时**或到某个**时钟时间**保持唤醒(`-t`)。
- 在**某条命令运行期间**保持唤醒,命令结束后自动退出。
- 可选的**合盖**保持唤醒,通过按需启动的 `launchd` 辅助进程实现。
- 首次运行引导安装:未安装辅助进程时,espresso 会询问是否帮你安装;若拒绝——或在
  没有终端的环境里运行(脚本、CI)——会话会仅以防空闲睡眠继续。
- 单个自包含二进制,无运行时依赖。

## 环境要求

- Apple Silicon(arm64)芯片的 macOS。

## 安装

```sh
curl -fsSL https://raw.githubusercontent.com/Hanyang-Li/espresso/main/install.sh | sh
```

该命令会下载最新版本、校验其 checksum,并把二进制安装到 `/usr/local/bin/espresso`。

可选覆盖项:

```sh
# 锁定某个版本
ESPRESSO_VERSION=v0.2.2 sh -c "$(curl -fsSL https://raw.githubusercontent.com/Hanyang-Li/espresso/main/install.sh)"

# 安装到自定义目录
ESPRESSO_INSTALL_DIR="$HOME/.local/bin" sh -c "$(curl -fsSL https://raw.githubusercontent.com/Hanyang-Li/espresso/main/install.sh)"
```

> 二进制通过 `curl` 下载,macOS 不会对它做隔离(quarantine)——没有 Gatekeeper
> 弹窗,也不需要代码签名。

## 使用

### 保持唤醒一段时间

```sh
espresso -t 1800      # 30 分钟(按秒倒计时)
espresso -t 17:00     # 到今天 17:00
espresso -t 09:30:00  # 到某个具体时刻
```

倒计时可以是纯秒数,也可以是一个**未来**的时钟/日期时间,支持以下任一格式:
`HH:MM`、`HH:MM:SS`、`MM-DD HH:MM`、`YYYY-MM-DD HH:MM`、`YYYY-MM-DD HH:MM:SS`。

计时期间会显示实时进度条。按 **`q`** 可提前停止。

### 在命令运行期间保持唤醒

```sh
espresso npm run build
espresso -- rsync -a ./src remote:/backup   # 命令自带参数时用 -- 分隔
```

Mac 会一直保持唤醒直到命令结束,`espresso` 以该命令自身的退出码退出。

### 合盖保持唤醒(可选)

**第一次**在未安装辅助进程的情况下启动保持唤醒会话时,espresso 会询问是否安装,
你同意的话它会替你执行 `sudo espresso daemon install`。如果你拒绝——或在没有终端
的脚本里运行——会话会仅以防空闲睡眠继续。

你也可以显式管理辅助进程:

```sh
sudo espresso daemon install     # 一次性配置
espresso daemon status           # 查看当前状态
sudo espresso daemon uninstall   # 移除
```

辅助进程由 `launchd` 按需启动,没有会话时自动退出。安装之后,每一次
`espresso -t …` / `espresso <命令>` 会话都会自动获得合盖覆盖,无需额外参数。

## 常用命令

| 命令 | 说明 |
| --- | --- |
| `espresso -t <秒\|时间>` | 按倒计时或到某时钟时间保持唤醒。 |
| `espresso <命令> …` | 在命令运行期间保持唤醒。 |
| `espresso daemon install` | 安装合盖辅助进程(需要 `sudo`)。 |
| `espresso daemon uninstall` | 移除辅助进程(需要 `sudo`)。 |
| `espresso daemon status` | 查看辅助进程与保持唤醒状态。 |
| `espresso --version` | 打印版本号。 |
| `espresso --help` | 显示帮助。 |

## 注意事项

### `SleepDisabled` 是全局的

合盖唤醒依赖内核的 `SleepDisabled` 标志,它是一个**没有归属**的全局开关。
`espresso` 在有会话活跃时把它置上,最后一个会话结束时清除。这是**最后写入者
获胜**:它可能覆盖你通过 `pmset` 手动设置(或其它 app 设置)的 `SleepDisabled`,
反之亦然。

### 已安装辅助进程时的升级

重新运行安装命令会通过"同目录原子 rename"就地覆盖二进制,即使 daemon 正在运行
也是安全的。运行中的 daemon 会继续服务到下次空闲退出;新版本会在 `launchd`
下次启动它时接管。只有当你把二进制移到了别的路径,或执行过
`sudo espresso daemon uninstall` 之后,才需要重新运行 `sudo espresso daemon install`。

## 从源码构建

需要较新的 Rust 工具链(Rust 2024 edition)。

```sh
git clone https://github.com/Hanyang-Li/espresso
cd espresso
cargo build --release
# 二进制位于 target/release/espresso

# 或直接安装到 PATH:
cargo install --path .
```

用 `cargo test` 运行测试。

## 卸载

```sh
sudo espresso daemon uninstall   # 如果装过辅助进程
sudo rm /usr/local/bin/espresso
```

## 许可证

[MIT](LICENSE) © Hanyang Li
