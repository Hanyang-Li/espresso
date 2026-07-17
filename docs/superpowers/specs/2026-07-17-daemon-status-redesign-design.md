# `espresso daemon status` 输出重构 + 逐 session 详情

日期: 2026-07-17
状态: 已确认

## 背景

`espresso daemon status` 目前的输出用缩进纯文本铺开，观感差：一行「espresso daemon
status」标题、每项二级缩进、`SleepDisabled` 后面挂着一串括号注解、还有一个基本无用的
`Socket ... (present)` 行。

更关键的是数据层的限制：daemon 目前**只记一个 refcount 计数**（`refcount.rs`），
每个 hold 连接发的是裸 `HOLD\n`，`StatusInfo` 只带 `refcount/sleep_disabled/
lid_closed/pid/version`。所以想按 session 展示 pid·命令·时长，daemon 根本没有这些数据。

本次目标：把状态输出重排成三个圆角边框分组，并让 daemon 真正记录每个 session 的元数据。

## 决策

- **逐 session 详情做完整版**：客户端在 HOLD 时上报自己的 pid 和命令行；daemon 记录每个
  session 的启动时刻；QUERY 返回 session 列表（pid·命令·时长）。
- **Socket 行整行删掉**：它现在的 `(present/absent)` 只表示「socket 文件是否存在」，而
  该文件由 launchd 在注册时创建，语义和 `Registered` 重复；真正测「是否可达」需要连接
  socket，但那会触发 socket-activation 把 daemon 拉起来，破坏「idle」观测。因此不值得保留。
- **圆角边框手写渲染器 + `unicode-width` crate**：自己画 `╭─╮│╰─╯`、算颜色、做截断；只把
  「可见列宽」（CJK 占两格）这一块交给 `unicode-width`（clap / ripgrep 同款，是唯一手写有
  风险的部分）。不引入 comfy-table 之类的重表格库。
- **`SleepDisabled` 统一读系统真值**：`read_sleep_disabled()` 不需要 root，无论 daemon 是否
  运行都能读到系统 `SleepDisabled` 的真实状态，因此去掉原来「(set by another process)」/
  「(espresso: N sessions)」这类分支注解，只显示 yes/no。
- **不显示 Lid 行**。

## 视觉结果

`yes` = 绿色加粗，`no` = 红色加粗（仅当 stdout 是 TTY 时上色）。三个框共用一个宽度
（取各框自然内容宽度的最大值，clamp 到终端宽度），以便对齐。

### daemon 运行中、2 个 session

```
╭─ Status ──────────────────────────────────────╮
│ SleepDisabled   yes                            │
│ Running         yes   pid 12345                │
╰────────────────────────────────────────────────╯
╭─ Active Sessions (2) ──────────────────────────╮
│ 12346   espresso -- sleep 100            3m12s │
│ 12888   espresso 30m                        45s │
╰────────────────────────────────────────────────╯
╭─ Infos ────────────────────────────────────────╮
│ Version      daemon 0.2.2 / cli 0.2.2          │
│ Installed    yes   /Library/LaunchDaemons/loc… │
│ Registered   yes   system/local.espresso.daem… │
╰────────────────────────────────────────────────╯
```

### daemon 未运行

`Active Sessions` 框不显示（n=0）；未查询 daemon，所以 `Version` 只显示 cli：

```
╭─ Status ───────────────────────────────╮
│ SleepDisabled   no                      │
│ Running         no                      │
╰─────────────────────────────────────────╯
╭─ Infos ────────────────────────────────╮
│ Version      cli 0.2.2                  │
│ Installed    yes   /Library/LaunchDae…  │
│ Registered   no    system/local.espre…  │
╰─────────────────────────────────────────╯
```

### 未安装

同样两个框（`Installed no` 红色，`Registered no`），框下方保留原有 sudo 提示行：

```
→ run `espresso daemon install` (needs sudo) to enable lid-closed keep-awake
```

### 分组与规则

- **Status**：`SleepDisabled`（yes/no，读系统真值）、`Running`（yes/no + 运行时挂 daemon pid）。
- **Active Sessions (n)**：仅当 n>0 才显示；每行 `pid  命令(超长截断…)  时长(右对齐)`。
- **Infos**：`Version`（运行时 `daemon X / cli Y`，否则 `cli Y`）、`Installed`（yes/no + plist
  路径截断）、`Registered`（yes/no + `system/<LABEL>` 截断）。
- 无标题行、无缩进、无 Lid 行、无 Socket 行。

## 协议与数据模型（`ipc.rs`）

`StatusInfo` 去掉 `refcount`（改由列表长度派生）、`sleep_disabled`（改为本地读系统真值）、
`lid_closed`（Lid 行已删，无用），新增 session 列表：

```rust
pub struct SessionInfo {
    pub pid: u32,
    pub command: String,   // 已 sanitize，例如 "espresso -- sleep 100"
    pub uptime_secs: u64,  // 查询时 daemon 侧算出
}
pub struct StatusInfo {
    pub sessions: Vec<SessionInfo>,
    pub pid: u32,          // daemon 自己的 pid
    pub version: String,
}
```

**QUERY 返回改为多行**（reader 从 `sessions=N` 得知后续要读 N 行；`version` 与 `cmd` 都放在各自
行的最后一个字段，因此可含空格 / CJK）：

```
STATUS pid=<daemon_pid> sessions=<N> version=<v>
SESSION pid=<p> uptime=<secs> cmd=<command…>     (共 N 行)
```

**HOLD 带字段**：`HOLD pid=<client_pid> cmd=<command>`。daemon 仍接受裸 `HOLD`（向前兼容旧客户端）；
`cmd` 去掉换行后是行内最后一个字段，可含空格。

兼容性说明：升级期间可能出现「新客户端 → 旧 daemon」（旧 daemon 只认裸 `HOLD`，会拒绝带字段的
HOLD）。该窗口最长约 60s（daemon idle 后自退），且失败会优雅降级（IOKit idle-sleep assertion 仍生效，
只是这一会话不覆盖合盖，并打印一行 warning），可接受，不做额外处理。

## daemon 侧 session 追踪（`daemon.rs`）

`refcount.rs` **不动** —— 它继续作为已测过的状态机，在 0↔1 跳变上驱动
sleep-disable / lid-watch / grace。在它旁边，coordinator 新增一个
`HashMap<u64, ActiveSession>`，其中 `ActiveSession { pid, command, started: Instant }`。

- 一个共享的 `AtomicU64` 计数器给每个连接分配 id（不依赖随机数 / 时间戳）。
- `Event::HoldOpened(SessionMeta { id, pid, command })` → `state.on_hold_open()`（驱动 actions）
  且 `sessions.insert(id, ActiveSession { pid, command, started: Instant::now() })`。
- `Event::HoldClosed(id)` → `state.on_hold_close()` 且 `sessions.remove(&id)`。
- `Event::Query` → 用 `sessions` 构造 `Vec<SessionInfo>`（`uptime_secs = started.elapsed().as_secs()`，
  按 `started` 升序 / 最久优先），组装 `StatusInfo { sessions, pid: process::id(), version }`。

`state.count()` 与 `sessions.len()` 由同一组事件驱动，始终一致；列表用 `sessions`，状态机仍用
`state.count()`。

连接处理线程（`handle_connection`）：解析扩展后的 `HOLD` 行拿到 pid/command，本地
`NEXT_ID.fetch_add(1, SeqCst)` 生成 id，发 `HoldOpened`；连接 EOF 后发 `HoldClosed(id)`。

## 客户端上报（`session.rs`）

`hold_connection` 改签名为 `hold_connection(command: &str)`，内部发
`HOLD pid=<own_pid> cmd=<command>`（pid 用 `std::process::id()`）。

`command` 由 `std::env::args()` 重建：argv[0] 归一化成 `espresso`，其余原样 join，例如
`espresso -- sleep 100`、`espresso 30m`；去掉换行。timer 与 command 两种模式走同一套机制。
`start_keepawake` / `run_timer` / `run_command` 把重建好的 command 传进去。

## 渲染模块（新增 `src/ui.rs`）

把画框逻辑独立出来，让 `install.rs` 只负责组装数据：

- `unicode-width` 算可见列宽（CJK 两格）—— 唯一不手写的部分。
- 圆角框渲染器：入参为标题 + 若干内容行；每行同时携带 `plain`（算宽用）与 `display`（含 ANSI，
  用于对齐上色 token 后正确 pad）。
- 宽度感知的截断（末尾 `…`，按列预算而非字节 / 字符数裁剪）。
- `paint(text, color, bold, use_color) -> (plain, display)`：`use_color = false` 时 display == plain。
- `format_uptime(secs)`：`45s` / `3m12s` / `2h5m` / `1d3h`。
- `use_color = std::io::stdout().is_terminal()`。

`install.rs::print_status` 保持零副作用纪律：装/注册/运行事实来自 plist 文件 + `launchctl print`
（都不碰 socket），仅当 launchd 已报告有活实例时才用 `Query` 连一次 socket（触达既有 daemon，
不新起、不 bump refcount），然后把数据喂给渲染器。

Active Sessions 行的列布局：内容宽度 W；左段 = `pid`（右 pad 到该框内最大 pid 宽）+ 两空格 +
命令；右段 = uptime；命令按 `左段 + 至少两空格 + uptime ≤ W` 截断，中间用空格撑开。所有宽度经
`unicode-width` 计算。

## 依赖

`Cargo.toml` 增加 `unicode-width = "0.2"`。

## 测试

- `ipc.rs`：`StatusInfo` 0/1/多 session 的 round-trip，`cmd` 含空格与 CJK；扩展 `HOLD` 可解码，
  裸 `HOLD` 仍可解码。
- `refcount.rs`：不改，保持全绿。
- `ui.rs` 纯函数测试：ASCII+CJK 字符串宽度、截断遵守列预算、框边对齐、`format_uptime`。
- 手动 e2e：`espresso daemon status` 在 运行中 / idle / 无 session / 未安装 四态下的输出。
