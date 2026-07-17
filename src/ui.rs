//! Rounded-box status rendering primitives: visible-width math (CJK-aware),
//! width-bounded truncation, compact uptime formatting, and a rounded box
//! renderer that pads on PLAIN text so ANSI-colored tokens still align.

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
