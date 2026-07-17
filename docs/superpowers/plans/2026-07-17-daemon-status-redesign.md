# daemon status 重构 + 逐 session 详情 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `espresso daemon status` 重排成三个圆角边框分组，并让 daemon 记录每个 session 的 pid·命令·时长。

**Architecture:** 三层推进——(1) 独立的纯渲染模块 `ui.rs`（画框/上色/截断/时长）；(2) 显示侧：重塑 `StatusInfo` 协议为多行 QUERY 响应，并用 `ui.rs` 重写 `print_status`（此时 session 列表为空）；(3) 写入侧：`HOLD` 带上 pid/命令，daemon 用 `HashMap` 追踪每个 session 的启动时刻，QUERY 返回真实列表，Active Sessions 框随即点亮。每次提交都能编译、可测。

**Tech Stack:** Rust (edition 2024)、crossterm（上色，已有依赖）、unicode-width（列宽，新增）、Unix domain socket IPC、launchd。

## Global Constraints

- 平台仅 macOS；`read_sleep_disabled()` 不需要 root，`print_status` 以普通用户身份运行。
- 零副作用纪律：`print_status` 只读 plist 文件 + `launchctl print`；仅当 launchd 已报告有活实例时才用 `Query` 连一次 socket（绝不新起 daemon、不 bump refcount）。
- 上色仅当 `std::io::stdout().is_terminal()` 为真。
- CJK 等宽字符按 2 列计算，一律经 `unicode-width`。
- `refcount.rs` 状态机不改，保持全绿。
- IPC 为行分隔文本；`version` 与 `cmd` 必须是各自行的最后一个字段（可含空格），且写入前把 `\n`/`\r` 换成空格。
- 提交作者身份用 `Hanyang-Li <60208398+Hanyang-Li@users.noreply.github.com>`，commit message 结尾保留 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` trailer。
- 圆角框字符：`╭ ─ ╮ │ ╰ ╯`。

---

## Task 1: 渲染工具模块 `ui.rs`

纯函数模块，不依赖任何其它改动，独立可测。

**Files:**
- Modify: `Cargo.toml`（新增 `unicode-width` 依赖）
- Modify: `src/lib.rs`（新增 `pub mod ui;`）
- Create + Test: `src/ui.rs`（实现 + 行内 `#[cfg(test)]`）

**Interfaces:**
- Produces（后续 Task 2/3 依赖）：
  - `pub fn display_width(s: &str) -> usize`
  - `pub fn truncate(s: &str, max_cols: usize) -> String`
  - `pub fn format_uptime(secs: u64) -> String`
  - `pub struct Cell { pub plain: String, pub display: String }`，`impl Cell { pub fn plain(s: impl Into<String>) -> Cell; pub fn width(&self) -> usize }`
  - `pub fn yesno(v: bool, use_color: bool) -> Cell`
  - `pub fn join(cells: &[Cell]) -> Cell`
  - `pub fn render_box(title: &str, lines: &[Cell], inner: usize) -> String`

- [ ] **Step 1: 新增依赖**

修改 `Cargo.toml`，在 `[dependencies]` 里加一行（放在 `libc` 之后，保持字母序无强制要求）：

```toml
unicode-width = "0.2"
```

- [ ] **Step 2: 注册模块**

修改 `src/lib.rs`，在 `pub mod time;` 之前加入（保持字母序）：

```rust
pub mod ui;
```

- [ ] **Step 3: 写失败测试**

创建 `src/ui.rs`，先只放测试（实现随后补）：

```rust
//! Rounded-box status rendering primitives: visible-width math (CJK-aware),
//! width-bounded truncation, compact uptime formatting, and a rounded box
//! renderer that pads on PLAIN text so ANSI-colored tokens still align.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_counts_cjk_as_two() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("你好"), 4);
        assert_eq!(display_width("a你b"), 4);
    }

    #[test]
    fn truncate_respects_column_budget() {
        assert_eq!(truncate("abcdef", 10), "abcdef");
        assert_eq!(truncate("abcdef", 4), "abc…");
        // CJK: budget 4 -> one wide char (2) + ellipsis (1) fits, second would overflow.
        assert_eq!(truncate("你好世界", 4), "你…");
        assert_eq!(truncate("abc", 0), "");
    }

    #[test]
    fn uptime_formats_compactly() {
        assert_eq!(format_uptime(45), "45s");
        assert_eq!(format_uptime(192), "3m12s");
        assert_eq!(format_uptime(7500), "2h5m");
        assert_eq!(format_uptime(90000), "1d1h");
    }

    #[test]
    fn box_borders_align_for_ascii() {
        let out = render_box("T", &[Cell::plain("ab")], 5);
        assert_eq!(
            out,
            "╭─ T ───╮\n│ ab    │\n╰───────╯\n"
        );
    }

    #[test]
    fn box_borders_align_for_cjk() {
        // Every rendered line must have identical visible width.
        let out = render_box("状态", &[Cell::plain("你好"), Cell::plain("ab")], 6);
        let widths: Vec<usize> = out.lines().map(display_width).collect();
        assert!(widths.windows(2).all(|w| w[0] == w[1]), "widths={widths:?}");
    }

    #[test]
    fn yesno_plain_when_no_color() {
        assert_eq!(yesno(true, false), Cell::plain("yes"));
        assert_eq!(yesno(false, false), Cell::plain("no"));
    }
}
```

Add `#[derive(Debug, PartialEq, Eq)]` on `Cell` so the `yesno_plain_when_no_color` assert compiles.

- [ ] **Step 4: 跑测试确认失败**

Run: `cargo test --lib ui::`
Expected: 编译失败（`display_width` 等未定义）。

- [ ] **Step 5: 实现模块**

在 `src/ui.rs` 顶部（doc 注释之后、`#[cfg(test)]` 之前）写入实现：

```rust
use crossterm::style::Stylize;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Visible column width (CJK wide chars = 2). Pass PLAIN text (no ANSI).
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate to at most `max_cols` visible columns, appending '…' when cut.
pub fn truncate(s: &str, max_cols: usize) -> String {
    if display_width(s) <= max_cols {
        return s.to_string();
    }
    if max_cols == 0 {
        return String::new();
    }
    let budget = max_cols - 1; // reserve one column for '…'
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

/// Compact human uptime: 45s, 3m12s, 2h5m, 1d3h.
pub fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let (m, s) = (secs / 60, secs % 60);
    if m < 60 {
        return format!("{m}m{s}s");
    }
    let (h, m) = (m / 60, m % 60);
    if h < 24 {
        return format!("{h}h{m}m");
    }
    let (d, h) = (h / 24, h % 24);
    format!("{d}d{h}h")
}

/// A rendered cell: `plain` drives width math, `display` is what's printed
/// (may carry ANSI). With color disabled the two are equal.
#[derive(Debug, PartialEq, Eq)]
pub struct Cell {
    pub plain: String,
    pub display: String,
}

impl Cell {
    /// Uncolored cell: display == plain.
    pub fn plain(s: impl Into<String>) -> Cell {
        let s = s.into();
        Cell { plain: s.clone(), display: s }
    }

    /// Visible width of the cell (from its plain text).
    pub fn width(&self) -> usize {
        display_width(&self.plain)
    }
}

/// A yes/no token: yes = green bold, no = red bold (only when `use_color`).
pub fn yesno(v: bool, use_color: bool) -> Cell {
    let text = if v { "yes" } else { "no" };
    if !use_color {
        return Cell::plain(text);
    }
    let styled = if v {
        text.green().bold()
    } else {
        text.red().bold()
    };
    Cell {
        plain: text.to_string(),
        display: styled.to_string(),
    }
}

/// Concatenate cells into one line-cell (plain and display joined in order).
pub fn join(cells: &[Cell]) -> Cell {
    Cell {
        plain: cells.iter().map(|c| c.plain.as_str()).collect(),
        display: cells.iter().map(|c| c.display.as_str()).collect(),
    }
}

/// Render a rounded box. `inner` is the content width between the side
/// paddings; the caller must ensure `inner >= display_width(title) + 1` and
/// `inner >= max line width`. Total visible width of every line is `inner + 4`.
pub fn render_box(title: &str, lines: &[Cell], inner: usize) -> String {
    let mut out = String::new();
    // Top: "╭─ title " + dashes + "╮"
    out.push_str("╭─ ");
    out.push_str(title);
    out.push(' ');
    let dashes = inner.saturating_sub(display_width(title) + 1);
    for _ in 0..dashes {
        out.push('─');
    }
    out.push_str("╮\n");
    // Content lines.
    for line in lines {
        let pad = inner.saturating_sub(line.width());
        out.push_str("│ ");
        out.push_str(&line.display);
        for _ in 0..pad {
            out.push(' ');
        }
        out.push_str(" │\n");
    }
    // Bottom.
    out.push('╰');
    for _ in 0..inner + 2 {
        out.push('─');
    }
    out.push_str("╯\n");
    out
}
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test --lib ui::`
Expected: PASS（6 个测试）。

- [ ] **Step 7: 提交**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/ui.rs
git -c user.name='Hanyang-Li' -c user.email='60208398+Hanyang-Li@users.noreply.github.com' \
  commit -m "feat(ui): rounded-box render primitives with CJK-aware width

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: 显示侧——协议多行化 + `print_status` 重写

只动「服务器→客户端」方向：重塑 `StatusInfo`（去 `refcount`/`sleep_disabled`/`lid_closed`，加 `sessions`），QUERY 改多行响应，并用 `ui.rs` 重写整个状态输出。`ClientMsg::Hold` 保持不变（仍是单元变体），因此 hold 路径与 `session.rs` 本任务不动，daemon 返回**空** session 列表——`Active Sessions` 框此时不出现，Task 3 再填充。

**Files:**
- Modify: `src/ipc.rs`（`StatusInfo`、新增 `SessionInfo`、`encode_server`/`decode_server` 多行、改测试）
- Modify: `src/daemon.rs`（`query_status` 改 `read_to_string`、`Event::Query` 构造新 `StatusInfo`）
- Modify: `src/install.rs`（`print_status` 全量重写 + 依赖 `ui`）

**Interfaces:**
- Consumes（来自 Task 1）：`crate::ui::{Cell, yesno, join, truncate, format_uptime, display_width, render_box}`。
- Produces（Task 3 依赖）：
  - `pub struct SessionInfo { pub pid: u32, pub command: String, pub uptime_secs: u64 }`
  - `pub struct StatusInfo { pub sessions: Vec<SessionInfo>, pub pid: u32, pub version: String }`
  - `encode_server`/`decode_server` 支持多行 `STATUS`/`SESSION`。

- [ ] **Step 1: 写失败测试（ipc 多行 round-trip）**

修改 `src/ipc.rs` 的 `#[cfg(test)] mod tests`：删除引用旧字段（`refcount`/`sleep_disabled`/`lid_closed`）的四个 status 测试体（`status_round_trip`、`status_missing_prefix_rejected`、`status_missing_field_rejected`、`status_non_numeric_refcount_rejected`、`status_unknown_field_rejected`、`status_version_with_space_round_trips`），替换为下列测试：

```rust
    #[test]
    fn status_round_trip_no_sessions() {
        let info = StatusInfo {
            sessions: vec![],
            pid: 4821,
            version: "0.2.2".into(),
        };
        let line = encode_server(&ServerMsg::Status(info.clone()));
        assert_eq!(decode_server(&line), Ok(ServerMsg::Status(info)));
    }

    #[test]
    fn status_round_trip_with_sessions_incl_spaces_and_cjk() {
        let info = StatusInfo {
            sessions: vec![
                SessionInfo { pid: 12346, command: "espresso -- sleep 100".into(), uptime_secs: 192 },
                SessionInfo { pid: 12888, command: "espresso -- echo 你好 世界".into(), uptime_secs: 45 },
            ],
            pid: 700,
            version: "0.2.2 debug build".into(),
        };
        let line = encode_server(&ServerMsg::Status(info.clone()));
        assert_eq!(decode_server(&line), Ok(ServerMsg::Status(info)));
    }

    #[test]
    fn status_session_count_mismatch_rejected() {
        // Header claims 1 session but no SESSION line follows.
        assert!(matches!(
            decode_server("STATUS pid=1 sessions=1 version=0.1"),
            Err(IpcError::Malformed(_))
        ));
    }

    #[test]
    fn status_missing_prefix_rejected() {
        assert!(matches!(
            decode_server("pid=1 sessions=0 version=0.1"),
            Err(IpcError::Malformed(_))
        ));
    }

    #[test]
    fn status_unknown_field_rejected() {
        assert!(matches!(
            decode_server("STATUS pid=1 sessions=0 bogus=1 version=0.1"),
            Err(IpcError::Malformed(_))
        ));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib ipc::`
Expected: 编译失败（`SessionInfo` 未定义、`StatusInfo` 字段不符）。

- [ ] **Step 3: 重塑类型与编解码**

修改 `src/ipc.rs`：把 `StatusInfo` 定义替换，并在其上方新增 `SessionInfo`：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub pid: u32,
    pub command: String,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInfo {
    pub sessions: Vec<SessionInfo>,
    pub pid: u32,
    pub version: String,
}
```

在文件里新增一个私有清洗函数（供 `encode_server` 用；`ClientMsg` 本任务不动）：

```rust
/// Replace line-breaking bytes so a value stays on its own IPC line.
fn sanitize_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}
```

替换 `encode_server`：

```rust
pub fn encode_server(m: &ServerMsg) -> String {
    match m {
        ServerMsg::Ok => "OK\n".to_string(),
        ServerMsg::Status(s) => {
            let mut out = format!(
                "STATUS pid={} sessions={} version={}\n",
                s.pid,
                s.sessions.len(),
                s.version,
            );
            for sess in &s.sessions {
                out.push_str(&format!(
                    "SESSION pid={} uptime={} cmd={}\n",
                    sess.pid,
                    sess.uptime_secs,
                    sanitize_line(&sess.command),
                ));
            }
            out
        }
    }
}
```

替换 `decode_server`（现在吃多行输入）：

```rust
pub fn decode_server(input: &str) -> Result<ServerMsg, IpcError> {
    let mut lines = input.lines();
    let first = lines.next().unwrap_or("").trim_end();
    if first == "OK" {
        return Ok(ServerMsg::Ok);
    }
    let rest = first
        .strip_prefix("STATUS ")
        .ok_or_else(|| IpcError::Malformed(first.to_string()))?;

    // `version` is the last field on the header line and may contain spaces.
    let (fields, version) = rest
        .split_once(" version=")
        .ok_or_else(|| IpcError::Malformed(first.to_string()))?;

    let mut pid = None;
    let mut count = None;
    for field in fields.split_whitespace() {
        let (k, v) = field
            .split_once('=')
            .ok_or_else(|| IpcError::Malformed(field.to_string()))?;
        match k {
            "pid" => pid = v.parse().ok(),
            "sessions" => count = v.parse::<usize>().ok(),
            _ => return Err(IpcError::Malformed(field.to_string())),
        }
    }
    let pid = pid.ok_or_else(|| IpcError::Malformed(first.to_string()))?;
    let count = count.ok_or_else(|| IpcError::Malformed(first.to_string()))?;

    let mut sessions = Vec::new();
    for line in lines {
        let sl = line
            .strip_prefix("SESSION ")
            .ok_or_else(|| IpcError::Malformed(line.to_string()))?;
        // `cmd` is the last field and may contain spaces.
        let (sf, cmd) = sl
            .split_once(" cmd=")
            .ok_or_else(|| IpcError::Malformed(line.to_string()))?;
        let mut spid = None;
        let mut uptime = None;
        for field in sf.split_whitespace() {
            let (k, v) = field
                .split_once('=')
                .ok_or_else(|| IpcError::Malformed(field.to_string()))?;
            match k {
                "pid" => spid = v.parse().ok(),
                "uptime" => uptime = v.parse().ok(),
                _ => return Err(IpcError::Malformed(field.to_string())),
            }
        }
        sessions.push(SessionInfo {
            pid: spid.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
            uptime_secs: uptime.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
            command: cmd.to_string(),
        });
    }
    if sessions.len() != count {
        return Err(IpcError::Malformed(format!(
            "session count {count} != {} SESSION lines",
            sessions.len()
        )));
    }

    Ok(ServerMsg::Status(StatusInfo {
        sessions,
        pid,
        version: version.to_string(),
    }))
}
```

Note: `SessionInfo` 已在 Step 3 的 `pub struct` 处对外可见；测试模块通过 `use super::*;` 拿到。

- [ ] **Step 4: 跑测试确认通过（ipc）**

Run: `cargo test --lib ipc::`
Expected: PASS。此时整个 crate 还不能编译（daemon/install 仍用旧字段），下一步修。

- [ ] **Step 5: 更新 daemon 的 Query 构造与 query_status**

修改 `src/daemon.rs`。先把顶部 `use std::io::{BufRead, BufReader, Read, Write};` 保持（已含 `Read`）。

替换 `query_status`（改为读到 EOF——Query 连接由 daemon 回完即关）：

```rust
/// Connects to the daemon, sends `QUERY`, and parses the reply. The daemon
/// closes the Query connection after replying, so we read to EOF and decode
/// the whole (possibly multi-line) block. Returns `Ok(None)` if the socket is
/// absent or the reply is unparseable.
pub fn query_status() -> std::io::Result<Option<StatusInfo>> {
    let mut stream = match UnixStream::connect(SOCKET_PATH) {
        Ok(s) => s,
        Err(_) => return Ok(None),
    };
    stream.write_all(encode_client(&ClientMsg::Query).as_bytes())?;
    stream.flush()?;
    let mut buf = String::new();
    BufReader::new(stream).read_to_string(&mut buf)?;
    match decode_server(&buf) {
        Ok(ServerMsg::Status(info)) => Ok(Some(info)),
        _ => Ok(None),
    }
}
```

在 `coordinator` 的 `Event::Query(reply)` 分支里，替换 `StatusInfo` 的构造（去掉 `lid_closed()` 调用与旧字段，session 列表本任务留空）：

```rust
            Event::Query(reply) => {
                let info = StatusInfo {
                    sessions: Vec::new(),
                    pid: std::process::id(),
                    version: VERSION.to_string(),
                };
                let _ = reply.send(info);
                vec![]
            }
```

同时更新 `src/daemon.rs` 顶部的 ipc `use`：把 `StatusInfo` 保留，新增 `SessionInfo` 暂不需要（Task 3 再加）。确认 `use crate::lid::lid_closed;` 仍被 `spawn_lid_watch`/`display_sleep_now` 用到（是的，保留）。

- [ ] **Step 6: 重写 `print_status`**

修改 `src/install.rs`。更新 imports 头部：

```rust
use crate::daemon::query_status;
use crate::ipc::{SOCKET_PATH, StatusInfo};
use crate::power::read_sleep_disabled;
use crate::ui::{self, Cell};
use anyhow::{Context, Result, bail};
use std::io::{IsTerminal, Write};
use std::process::Command;
```

（`SOCKET_PATH` 仍被 `uninstall`/`plist_contents` 用到，保留。）

用下列实现整体替换现有 `print_status` 函数体（保留其上方的 doc 注释）。同时**删除** `report_flag_without_daemon` 函数（不再使用）：

```rust
pub fn print_status() -> Result<()> {
    let use_color = std::io::stdout().is_terminal();
    let cli_version = env!("CARGO_PKG_VERSION");

    let installed = is_installed();
    let state = if installed {
        launchd_state()
    } else {
        LaunchdState { registered: false, running: false, pid: None }
    };
    // Only touch the socket when launchd already reports a live instance.
    let info: Option<StatusInfo> = if state.running {
        query_status().ok().flatten()
    } else {
        None
    };
    let sleep_disabled = read_sleep_disabled().unwrap_or(false);

    // Terminal width budget for the shared inner box width.
    let term = crossterm::terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
    let max_inner = term.saturating_sub(4).max(20);

    // ---- Status box lines ----
    let status_label = 13; // width("SleepDisabled")
    let mut status_lines = vec![
        ui::join(&[
            Cell::plain(format!("{:<status_label$}", "SleepDisabled")),
            Cell::plain("   "),
            ui::yesno(sleep_disabled, use_color),
        ]),
        {
            let mut parts = vec![
                Cell::plain(format!("{:<status_label$}", "Running")),
                Cell::plain("   "),
                ui::yesno(state.running, use_color),
            ];
            if let Some(pid) = state.pid.filter(|_| state.running) {
                parts.push(Cell::plain(format!("   pid {pid}")));
            }
            ui::join(&parts)
        },
    ];

    // ---- Active Sessions box lines (empty until daemon tracks them) ----
    let sessions = info.as_ref().map(|i| i.sessions.as_slice()).unwrap_or(&[]);
    let session_count = sessions.len();
    // Natural (untruncated) widths help size the shared inner width.
    let pid_col = sessions
        .iter()
        .map(|s| ui::display_width(&s.pid.to_string()))
        .max()
        .unwrap_or(0);

    // ---- Infos box lines ----
    let infos_label = 10; // width("Registered")
    let version_value = match &info {
        Some(i) => format!("daemon {} / cli {}", i.version, cli_version),
        None => format!("cli {cli_version}"),
    };
    let mut infos_specs: Vec<Cell> = vec![ui::join(&[
        Cell::plain(format!("{:<infos_label$}", "Version")),
        Cell::plain("   "),
        Cell::plain(version_value),
    ])];
    // Installed row (yes/no + plist path).
    infos_specs.push(info_row(
        infos_label,
        "Installed",
        installed,
        if installed { Some(PLIST_PATH.to_string()) } else { None },
        use_color,
        max_inner,
    ));
    // Registered row (yes/no + launchd label), only meaningful when installed.
    if installed {
        infos_specs.push(info_row(
            infos_label,
            "Registered",
            state.registered,
            Some(format!("system/{LABEL}")),
            use_color,
            max_inner,
        ));
    }

    // ---- Compute shared inner width across all boxes ----
    let session_natural = sessions
        .iter()
        .map(|s| pid_col + 2 + ui::display_width(&s.command) + 2 + ui::display_width(&ui::format_uptime(s.uptime_secs)))
        .max()
        .unwrap_or(0);
    let title_active = format!("Active Sessions ({session_count})");
    let mut inner = 0usize;
    for c in status_lines.iter().chain(infos_specs.iter()) {
        inner = inner.max(c.width());
    }
    inner = inner.max(session_natural);
    // Titles must fit: inner >= width(title) + 1.
    for t in ["Status", "Infos", title_active.as_str()] {
        inner = inner.max(ui::display_width(t) + 1);
    }
    inner = inner.min(max_inner);

    // ---- Lay out session rows to the shared inner width ----
    let session_lines: Vec<Cell> = sessions
        .iter()
        .map(|s| {
            let prefix = format!("{:<pid_col$}  ", s.pid);
            let uptime = ui::format_uptime(s.uptime_secs);
            let reserved = ui::display_width(&prefix) + ui::display_width(&uptime) + 1;
            let cmd_budget = inner.saturating_sub(reserved);
            let cmd = ui::truncate(&s.command, cmd_budget);
            let used = ui::display_width(&prefix) + ui::display_width(&cmd) + ui::display_width(&uptime);
            let gap = inner.saturating_sub(used);
            Cell::plain(format!("{prefix}{cmd}{}{uptime}", " ".repeat(gap)))
        })
        .collect();

    // ---- Emit ----
    let mut out = String::new();
    out.push_str(&ui::render_box("Status", &status_lines, inner));
    if session_count > 0 {
        out.push_str(&ui::render_box(&title_active, &session_lines, inner));
    }
    out.push_str(&ui::render_box("Infos", &infos_specs, inner));
    print!("{out}");
    let _ = &mut status_lines; // silence unused-mut if list is never appended to

    if !installed {
        println!("→ run `espresso daemon install` (needs sudo) to enable lid-closed keep-awake");
    }
    Ok(())
}

/// One Infos row: `label` padded to `label_col`, a yes/no token, then an
/// optional path/value truncated to fit the terminal budget.
fn info_row(
    label_col: usize,
    label: &str,
    flag: bool,
    value: Option<String>,
    use_color: bool,
    max_inner: usize,
) -> Cell {
    let mut parts = vec![
        Cell::plain(format!("{label:<label_col$}")),
        Cell::plain("   "),
        ui::yesno(flag, use_color),
    ];
    if let Some(v) = value {
        // yesno is "yes"(3)/"no"(2); pad so the value column starts evenly.
        let gap = if flag { "   " } else { "    " };
        let prefix_w = label_col + 3 + if flag { 3 } else { 2 } + gap.len();
        let budget = max_inner.saturating_sub(prefix_w);
        parts.push(Cell::plain(gap));
        parts.push(Cell::plain(ui::truncate(&v, budget)));
    }
    ui::join(&parts)
}
```

Note: `let _ = &mut status_lines;` 只是防止 `status_lines` 的 `mut` 触发 warning——若编译器不报 warning，删除该行与 `mut`。按实际编译结果二选一即可（保持 `cargo build` 零 warning）。

- [ ] **Step 7: 编译并跑全部测试**

Run: `cargo build && cargo test`
Expected: 编译通过、无 warning；所有测试 PASS。

- [ ] **Step 8: 手动核对输出（未安装 / idle 态即可）**

Run: `cargo run -- daemon status`
Expected: 打印 `╭─ Status ─…╮` 与 `╭─ Infos ─…╮` 两个圆角框，三框（此处两框）等宽对齐；未安装时框下有 `→ run ...` 提示；无 `Active Sessions` 框、无 `Socket`/`Lid` 行、无标题行。管道到文件 `cargo run -- daemon status | cat` 时应无 ANSI 颜色码。

- [ ] **Step 9: 提交**

```bash
git add src/ipc.rs src/daemon.rs src/install.rs
git -c user.name='Hanyang-Li' -c user.email='60208398+Hanyang-Li@users.noreply.github.com' \
  commit -m "feat(status): rounded-box status display; multi-line QUERY protocol

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: 写入侧——HOLD 带元数据 + daemon 追踪 session

只动「客户端→服务器」方向与 daemon 内部：`HOLD` 带上 pid/命令，daemon 用 `HashMap` 记录每个 session 的启动时刻，QUERY 返回真实列表。完成后 `Active Sessions` 框点亮。

**Files:**
- Modify: `src/ipc.rs`（`ClientMsg::Hold` 携带 `pid`/`command`、`encode_client`/`decode_client`、测试）
- Modify: `src/daemon.rs`（`hold_connection` 签名、`handle_connection` 解析+分配 id、`Event` 携带元数据、coordinator `HashMap`、`build_status`/`sort_sessions`、测试）
- Modify: `src/session.rs`（重建命令串并传给 `hold_connection`）

**Interfaces:**
- Consumes（来自 Task 2）：`crate::ipc::{SessionInfo, StatusInfo}`。
- Produces：
  - `pub enum ClientMsg { Hold { pid: u32, command: String }, Query }`
  - `pub fn hold_connection(command: &str) -> std::io::Result<UnixStream>`
  - daemon 内部：`pub(crate) fn sort_sessions(list: &mut Vec<SessionInfo>)`（按 uptime 降序）。

- [ ] **Step 1: 写失败测试（ipc HOLD 变体 + daemon 排序）**

在 `src/ipc.rs` 测试模块，替换 `client_round_trip` 与 `malformed_client_rejected`，并新增裸 HOLD 测试：

```rust
    #[test]
    fn client_round_trip() {
        for m in [
            ClientMsg::Hold { pid: 4821, command: "espresso -- sleep 100".into() },
            ClientMsg::Hold { pid: 7, command: "espresso -- echo 你好 世界".into() },
            ClientMsg::Query,
        ] {
            let line = encode_client(&m);
            assert_eq!(decode_client(&line), Ok(m));
        }
    }

    #[test]
    fn bare_hold_still_decodes() {
        assert_eq!(
            decode_client("HOLD"),
            Ok(ClientMsg::Hold { pid: 0, command: String::new() })
        );
    }

    #[test]
    fn malformed_client_rejected() {
        assert!(matches!(decode_client("NONSENSE"), Err(IpcError::Malformed(_))));
        assert!(matches!(decode_client("HOLD cmd=x"), Err(IpcError::Malformed(_))));
    }
```

在 `src/daemon.rs` 底部新增测试模块（若无则新建）：

```rust
#[cfg(test)]
mod tests {
    use super::sort_sessions;
    use crate::ipc::SessionInfo;

    #[test]
    fn sort_sessions_longest_uptime_first() {
        let mut list = vec![
            SessionInfo { pid: 1, command: "a".into(), uptime_secs: 45 },
            SessionInfo { pid: 2, command: "b".into(), uptime_secs: 192 },
            SessionInfo { pid: 3, command: "c".into(), uptime_secs: 100 },
        ];
        sort_sessions(&mut list);
        assert_eq!(
            list.iter().map(|s| s.pid).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test --lib ipc:: ; cargo test --lib daemon::tests`
Expected: 编译失败（`ClientMsg::Hold` 无字段、`sort_sessions` 未定义）。

- [ ] **Step 3: 改 `ClientMsg` 与客户端编解码**

修改 `src/ipc.rs`。替换 `ClientMsg`：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Hold { pid: u32, command: String },
    Query,
}
```

替换 `encode_client`：

```rust
pub fn encode_client(m: &ClientMsg) -> String {
    match m {
        ClientMsg::Hold { pid, command } => {
            format!("HOLD pid={pid} cmd={}\n", sanitize_line(command))
        }
        ClientMsg::Query => "QUERY\n".to_string(),
    }
}
```

替换 `decode_client`（接受裸 `HOLD` 与带字段 `HOLD pid=.. cmd=..`）：

```rust
pub fn decode_client(line: &str) -> Result<ClientMsg, IpcError> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line == "QUERY" {
        return Ok(ClientMsg::Query);
    }
    if line == "HOLD" {
        // Bare HOLD from an older client: no metadata available.
        return Ok(ClientMsg::Hold { pid: 0, command: String::new() });
    }
    if let Some(rest) = line.strip_prefix("HOLD ") {
        let (pid_part, cmd) = rest
            .split_once(" cmd=")
            .ok_or_else(|| IpcError::Malformed(line.to_string()))?;
        let pid = pid_part
            .strip_prefix("pid=")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| IpcError::Malformed(line.to_string()))?;
        return Ok(ClientMsg::Hold { pid, command: cmd.to_string() });
    }
    Err(IpcError::Malformed(line.to_string()))
}
```

- [ ] **Step 4: 跑测试确认 ipc 通过**

Run: `cargo test --lib ipc::`
Expected: PASS。（crate 整体此时仍不编译——daemon 的 `hold_connection`/`handle_connection` 待改。）

- [ ] **Step 5: daemon 追踪 session**

修改 `src/daemon.rs`。

顶部 imports 增补：

```rust
use crate::ipc::{
    ClientMsg, SOCKET_PATH, ServerMsg, SessionInfo, StatusInfo, decode_client, decode_server,
    encode_client, encode_server,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
```

（保留既有 `use std::sync::atomic::{AtomicBool, Ordering};` 时，合并成上面一行含 `AtomicU64`；避免重复导入。`Instant` 与既有 `Duration` 同来自 `std::time`，合并 `use std::time::{Duration, Instant};`。）

在常量区（`VERSION` 附近）新增全局 id 计数器：

```rust
/// Monotonic per-connection session id. Assigned in each connection handler
/// so a later `HoldClosed(id)` can remove exactly the session it opened.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
```

替换 `hold_connection`（带命令）：

```rust
/// Connects to the daemon and sends `HOLD` with this process's pid and the
/// reconstructed command line. Holding the returned stream open keeps the
/// hold alive; dropping it releases it.
pub fn hold_connection(command: &str) -> std::io::Result<UnixStream> {
    let mut stream = UnixStream::connect(SOCKET_PATH)?;
    let msg = ClientMsg::Hold { pid: std::process::id(), command: command.to_string() };
    stream.write_all(encode_client(&msg).as_bytes())?;
    stream.flush()?;
    Ok(stream)
}
```

替换 `Event` 枚举与新增两个结构：

```rust
/// Events fed into the single coordinator thread.
enum Event {
    HoldOpened(SessionMeta),
    HoldClosed(u64),
    GraceElapsed(u64),
    Query(Sender<StatusInfo>),
}

/// Metadata carried from a connection handler into the coordinator when a
/// hold opens.
struct SessionMeta {
    id: u64,
    pid: u32,
    command: String,
}

/// A live hold as tracked by the coordinator (started clock is daemon-side).
struct ActiveSession {
    pid: u32,
    command: String,
    started: Instant,
}
```

在 `handle_connection` 的 `Ok(ClientMsg::Hold)` 分支替换为携带元数据版本：

```rust
        Ok(ClientMsg::Hold { pid, command }) => {
            let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
            if tx.send(Event::HoldOpened(SessionMeta { id, pid, command })).is_err() {
                return;
            }
            let _ = conn.write_all(encode_server(&ServerMsg::Ok).as_bytes());
            let _ = conn.flush();
            // Block until the client goes away (EOF or error).
            let mut buf = [0u8; 64];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = tx.send(Event::HoldClosed(id));
        }
```

在 `coordinator` 顶部（`RefcountState::new()` 之后）新增 session 表：

```rust
    let mut sessions: HashMap<u64, ActiveSession> = HashMap::new();
```

替换 `while let Ok(event)` 里的匹配臂（`HoldOpened`/`HoldClosed`/`Query`）：

```rust
            Event::HoldOpened(meta) => {
                let actions = state.on_hold_open();
                sessions.insert(
                    meta.id,
                    ActiveSession { pid: meta.pid, command: meta.command, started: Instant::now() },
                );
                actions
            }
            Event::HoldClosed(id) => {
                let actions = state.on_hold_close();
                sessions.remove(&id);
                actions
            }
```

（`GraceElapsed` 臂不变。）替换 `Query` 臂为真实构造：

```rust
            Event::Query(reply) => {
                let _ = reply.send(build_status(&sessions));
                vec![]
            }
```

Note: 上面用 `let actions = match event { ... };` 结构；`HoldOpened`/`HoldClosed` 臂返回 `actions`，需保证匹配表达式整体求值为 `Vec<Action>`。若现有代码是 `let actions = match event { Event::HoldOpened => state.on_hold_open(), ... }`，把每个臂改成块表达式并以 `actions` 收尾即可（如上）。

在文件末尾（`#[cfg(test)]` 之前）新增两个函数：

```rust
/// Sort a session list for display: longest-running first.
pub(crate) fn sort_sessions(list: &mut Vec<SessionInfo>) {
    list.sort_by(|a, b| b.uptime_secs.cmp(&a.uptime_secs));
}

/// Snapshot the live session table into a `StatusInfo` for a QUERY reply.
fn build_status(sessions: &HashMap<u64, ActiveSession>) -> StatusInfo {
    let mut list: Vec<SessionInfo> = sessions
        .values()
        .map(|s| SessionInfo {
            pid: s.pid,
            command: s.command.clone(),
            uptime_secs: s.started.elapsed().as_secs(),
        })
        .collect();
    sort_sessions(&mut list);
    StatusInfo { sessions: list, pid: std::process::id(), version: VERSION.to_string() }
}
```

- [ ] **Step 6: session.rs 重建命令串**

修改 `src/session.rs`。新增私有函数并让 `start_keepawake` 用它调用 `hold_connection`：

```rust
/// The invocation as the user typed it, argv[0] normalized to `espresso`
/// (e.g. "espresso -- sleep 100", "espresso -t 1800"). Newlines stripped so
/// it stays on one IPC line.
fn current_command() -> String {
    let rest: Vec<String> = std::env::args().skip(1).collect();
    let joined = if rest.is_empty() {
        "espresso".to_string()
    } else {
        format!("espresso {}", rest.join(" "))
    };
    joined.replace(['\n', '\r'], " ")
}
```

在 `start_keepawake` 里，把 `hold_connection()` 调用改为 `hold_connection(&current_command())`：

```rust
    let hold = if installed {
        match hold_connection(&current_command()) {
            Ok(stream) => Some(stream),
            Err(e) => {
                eprintln!("espresso: could not reach daemon ({e}); lid-close will still sleep");
                None
            }
        }
    } else {
        eprintln!(
            "espresso: daemon not installed; idle-sleep prevented, but lid-close will still sleep"
        );
        None
    };
```

- [ ] **Step 7: 编译并跑全部测试**

Run: `cargo build && cargo test`
Expected: 编译零 warning；所有测试 PASS（含 `sort_sessions_longest_uptime_first`、`client_round_trip`、`bare_hold_still_decodes`）。

- [ ] **Step 8: 端到端核对 Active Sessions（需已安装 daemon）**

前置：`sudo cargo run -- daemon install`（或已装的 `espresso daemon install`）。然后：

```bash
# 起一个后台 hold（用 -- sleep，避免 TTY 依赖）
cargo run -- -- sleep 60 &
sleep 2
cargo run -- daemon status
```

Expected: 出现 `╭─ Active Sessions (1) ──…╮`，行内为该进程 pid、命令 `espresso -- sleep 60`（过长则尾部 `…`）、以及递增的时长（如 `2s`）；`Status` 框 `Running yes  pid <daemon_pid>`、`SleepDisabled yes`。等 `sleep` 结束或 `kill %1` 后，约 60s grace 内 daemon 自退，再查为 idle 态（无 Active Sessions 框）。

清理：`kill %1 2>/dev/null; wait 2>/dev/null || true`。

- [ ] **Step 9: 提交**

```bash
git add src/ipc.rs src/daemon.rs src/session.rs
git -c user.name='Hanyang-Li' -c user.email='60208398+Hanyang-Li@users.noreply.github.com' \
  commit -m "feat(daemon): track per-session pid/command/uptime; HOLD carries metadata

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- 三圆角框分组 / 无标题行 / 无缩进 → Task 2 `print_status` + `ui::render_box`。✓
- Status(SleepDisabled yes/no 绿红加粗、Running yes/no + daemon pid) → Task 2 status_lines + `ui::yesno`。✓
- Active Sessions (n) 仅 n>0，逐行 pid·命令(截断)·时长 → Task 2 布局代码 + Task 3 填充。✓
- Infos(Version daemon/cli、Installed、Registered，路径截断) → Task 2 infos_specs + `info_row`。✓
- 删 Socket 行、删 Lid 行 → Task 2 未渲染二者。✓
- 协议 SessionInfo/StatusInfo/多行 QUERY/HOLD 带字段 → Task 2 + Task 3 ipc。✓
- daemon HashMap + Instant + id 追踪 → Task 3 daemon。✓
- 客户端重建命令 → Task 3 session.rs。✓
- unicode-width 依赖、is_terminal 上色门控、refcount.rs 不动 → Task 1 + Task 2。✓
- 兼容：daemon 接受裸 HOLD → Task 3 `decode_client`。✓

**Placeholder scan:** 无 TBD/TODO；每个代码步骤给出完整代码。Task 2 Step 6 的 `let _ = &mut status_lines;` 已注明按实际 warning 二选一，非占位。✓

**Type consistency:**
- `Cell`/`yesno`/`join`/`truncate`/`format_uptime`/`display_width`/`render_box` 在 Task 1 定义，Task 2 使用，签名一致。✓
- `SessionInfo{pid,command,uptime_secs}`、`StatusInfo{sessions,pid,version}` 在 Task 2 定义，Task 3 `build_status`/测试使用，字段名一致。✓
- `ClientMsg::Hold{pid,command}` 在 Task 3 定义，`hold_connection`/`encode_client`/`decode_client` 一致。✓
- `hold_connection(command: &str)` 在 Task 3 定义，session.rs 调用一致。✓
- `sort_sessions` 在 Task 3 daemon.rs 定义并被其测试与 `build_status` 调用，名字一致。✓

## Execution Handoff

见下条消息。
