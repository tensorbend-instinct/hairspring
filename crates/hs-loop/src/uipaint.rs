//! REPL UI painting (UI gap closure batch 1, Eric 2026-09-08: SOTA-REPL
//! UI investigation - pi/omp paint tool-call cards with timing, semantic
//! color, and ambient status; hs-repl painted raw unstyled text).
//!
//! The loop emits TYPED UI events (no print-scraping): a UiSink receives
//! one UiEvent per visible beat. The Painter renders events to any Writer
//! with semantic ANSI color when the terminal supports it and byte-clean
//! plain text when piped (result JSON on stdout stays machine-readable).

use std::io::Write;

/// One visible beat of a running mission.
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// A model call started (per mission step).
    ModelCallStart { model: String },
    /// A model call finished; token counts as reported by the provider.
    ModelCallEnd {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
    },
    /// A tool call is about to execute.
    ToolCallStart {
        plugin: String,
        args_summary: String,
    },
    /// A tool call finished.
    ToolCallEnd {
        plugin: String,
        ok: bool,
        output_summary: String,
        elapsed_ms: u64,
    },
}

/// Sink for mission UI events (mirrors hs_kernel::DeltaSink).
pub type UiSink = Box<dyn FnMut(UiEvent) + Send>;

/// One-line, length-capped summary of a tool call's arguments: the
/// command for shell-shaped tools, else compact JSON.
pub fn summarize_args(args: &serde_json::Value) -> String {
    let s = args
        .get("command")
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(args).unwrap_or_else(|_| "?".into()));
    truncate(&s.replace('\n', " "), 120)
}

/// One-line, length-capped summary of a tool result.
pub fn summarize_output(out: &serde_json::Value) -> String {
    let s = out
        .get("stdout")
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(out).unwrap_or_else(|_| "?".into()));
    truncate(&s.trim().replace('\n', " "), 200)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
    t.push('…');
    t
}

/// Renders UiEvents to a writer. `color` on: semantic ANSI color.
pub struct Painter<'a, W: Write> {
    out: &'a mut W,
    color: bool,
}

impl<'a, W: Write> Painter<'a, W> {
    pub fn new(out: &'a mut W, color: bool) -> Self {
        Painter { out, color }
    }

    fn paint(&mut self, code: &str, text: &str) {
        let _ = if self.color {
            write!(self.out, "\x1b[{code}m{text}\x1b[0m")
        } else {
            write!(self.out, "{text}")
        };
    }

    /// Render one event. Cards go to the writer immediately.
    pub fn handle(&mut self, ev: &UiEvent) {
        match ev {
            UiEvent::ToolCallStart {
                plugin,
                args_summary,
            } => {
                self.paint("36;1", "\u{25b6} "); // bright cyan ▶
                self.paint("36", plugin);
                if !args_summary.is_empty() {
                    self.paint("2", &format!("  {args_summary}"));
                }
                let _ = writeln!(self.out);
            }
            UiEvent::ToolCallEnd {
                plugin: _,
                ok,
                output_summary,
                elapsed_ms,
            } => {
                let (code, mark) = if *ok { ("32", "\u{2713} ok") } else { ("31;1", "\u{2717} fail") };
                let _ = write!(self.out, "  ");
                self.paint(code, mark);
                self.paint("2", &format!("  {elapsed_ms}ms"));
                if !output_summary.is_empty() {
                    let _ = writeln!(self.out);
                    self.paint("2", &format!("  {output_summary}"));
                }
                let _ = writeln!(self.out);
            }
            UiEvent::ModelCallStart { model } => {
                self.paint("2", &format!("  … {model}"));
                let _ = writeln!(self.out);
            }
            UiEvent::ModelCallEnd {
                model: _,
                input_tokens,
                output_tokens,
            } => {
                self.paint(
                    "2",
                    &format!("  ↑{input_tokens} ↓{output_tokens}"),
                );
                let _ = writeln!(self.out);
            }
        }
        let _ = self.out.flush();
    }

    /// UI gap #1: the ambient status bar - ONE line carrying the session
    /// vitals: model, missions, steps, calls, metered cost, elapsed wall
    /// time, and the stream's short id. Semantic color when the terminal
    /// supports it: bright cyan model, dim separators, yellow cost.
    pub fn status_line(&mut self, v: &crate::repl::SessionVitals) {
        self.paint("36;1", &v.model_label); // bright cyan
        self.paint("2", " \u{00b7} ");
        let missions = format!(
            "{} mission{}",
            v.missions_run,
            if v.missions_run == 1 { "" } else { "s" }
        );
        self.paint("0", &missions);
        self.paint("2", " \u{00b7} ");
        self.paint("0", &format!("{} steps", v.total_steps));
        self.paint("2", " \u{00b7} ");
        self.paint("0", &format!("{} calls", v.total_model_calls));
        self.paint("2", " \u{00b7} ");
        self.paint("33", &format_usd_micros(v.total_cost_micros)); // yellow
        self.paint("2", " \u{00b7} ");
        self.paint("2", &format_elapsed(v.elapsed));
        self.paint("2", " \u{00b7} ");
        let short: String = v.stream_id.to_string().chars().take(8).collect();
        self.paint("2", &short);
        let _ = writeln!(self.out);
        let _ = self.out.flush();
    }
}

/// Metered cost as dollars: 430320 micros -> "$0.4303".
pub fn format_usd_micros(micros: u64) -> String {
    format!("${:.4}", micros as f64 / 1_000_000.0)
}

/// Compact wall time: 65s -> "1m5s", 3700s -> "1h1m", 9s -> "9s".
pub fn format_elapsed(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}h{}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m{}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// REPL UI gap #4: streaming markdown for model prose. Deltas arrive in
/// arbitrary chunks (a token can split mid-construct), so the streamer
/// buffers to line boundaries: a construct split across chunks renders
/// byte-identical to one fed whole. Supported: ATX headers, fenced code
/// blocks, inline `code`, **bold**, "- "/"* " bullets. Anything that
/// never closes prints literally. Plain mode strips markers, no ANSI.
pub struct MarkdownStreamer {
    color: bool,
    buf: String,
    in_fence: bool,
}

impl MarkdownStreamer {
    pub fn new(color: bool) -> Self {
        MarkdownStreamer {
            color,
            buf: String::new(),
            in_fence: false,
        }
    }

    /// Feed one delta. Complete lines render immediately; a partial
    /// tail waits for its newline (or finish()).
    pub fn push<W: Write>(&mut self, delta: &str, out: &mut W) {
        self.buf.push_str(delta);
        while let Some(pos) = self.buf.find('\n') {
            let mut line: String = self.buf.drain(..=pos).collect();
            line.pop(); // the newline itself
            self.render_line(&line, out);
            let _ = writeln!(out);
        }
        let _ = out.flush();
    }

    /// Flush a partial final line - the model's last tokens must not
    /// vanish. No-op when everything buffered is already rendered.
    pub fn finish<W: Write>(&mut self, out: &mut W) {
        if self.buf.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.buf);
        self.render_line(&line, out);
        let _ = writeln!(out);
        let _ = out.flush();
    }

    fn styled<W: Write>(&self, out: &mut W, code: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.color && !code.is_empty() {
            let _ = write!(out, "\x1b[{code}m{text}\x1b[0m");
        } else {
            let _ = write!(out, "{text}");
        }
    }

    fn render_line<W: Write>(&mut self, line: &str, out: &mut W) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            self.in_fence = !self.in_fence;
            return; // fence markers never print
        }
        if self.in_fence {
            self.styled(out, "2;36", line); // dim cyan, verbatim
            return;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            let text = trimmed[hashes + 1..].trim_end();
            self.render_inline(out, text, "1;4"); // bold underline
            return;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let indent = &line[..line.len() - trimmed.len()];
            let _ = write!(out, "{indent}");
            self.styled(out, "36", "\u{2022}");
            let _ = write!(out, " ");
            self.render_inline(out, rest, "");
            return;
        }
        self.render_inline(out, line, "");
    }

    /// Inline constructs on one complete line: `code` spans and **bold**
    /// spans, segment-styled so ANSI never nests. Unmatched markers
    /// print literally. `plain_style` styles the unmarked runs (headers
    /// render their whole line bold-underline).
    fn render_inline<W: Write>(&self, out: &mut W, line: &str, plain_style: &str) {
        let mut rest = line;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix("**") {
                if let Some(close) = after.find("**") {
                    if close > 0 {
                        self.styled(out, "1", &after[..close]);
                        rest = &after[close + 2..];
                        continue;
                    }
                }
                let _ = write!(out, "**");
                rest = after;
                continue;
            }
            if let Some(after) = rest.strip_prefix('`') {
                if let Some(close) = after.find('`') {
                    if close > 0 {
                        self.styled(out, "36", &after[..close]);
                        rest = &after[close + 1..];
                        continue;
                    }
                }
                let _ = write!(out, "`");
                rest = after;
                continue;
            }
            let next = rest.find(['*', '`']).unwrap_or(rest.len());
            if next == 0 {
                // a marker char that opened no construct: literal
                let end = rest
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| i)
                    .unwrap_or(rest.len());
                let _ = write!(out, "{}", &rest[..end]);
                rest = &rest[end..];
                continue;
            }
            self.styled(out, plain_style, &rest[..next]);
            rest = &rest[next..];
        }
    }
}

/// UI gap #8: the composer frame. A line-based REPL cannot hold a
/// persistent box under a scrolling transcript the way pi/omp's TUI
/// does, so the box is painted PER ENTRY: top border + left-bar prompt,
/// bottom border once the line lands. Visible width counts glyphs only
/// (escape codes excluded), so the frame is exact on any terminal.
///
/// The composer prompt: the box's left bar + the REPL sigil.
pub const EDITOR_PROMPT: &str = "\u{2502} hs> ";

/// Visible terminal width of a string: ANSI CSI sequences count zero,
/// every other char counts one (box glyphs are single-width).
pub fn visible_width(s: &str) -> usize {
    let mut w = 0;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' && it.peek() == Some(&'[') {
            it.next();
            for c2 in it.by_ref() {
                if c2.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            w += 1;
        }
    }
    w
}

fn sgr(color: bool, code: &str, text: &str) -> String {
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Top border: "╭─ label ────────╮" at exactly `cols` visible columns.
pub fn composer_top(label: &str, cols: usize, color: bool) -> String {
    let fixed = 2 + 1 + label.chars().count() + 1 + 1; // ╭─ sp label sp ╮
    let fill = cols.saturating_sub(fixed);
    format!(
        "{} {} {}{}",
        sgr(color, "2", "\u{256d}\u{2500}"),
        sgr(color, "36;1", label),
        sgr(color, "2", &"\u{2500}".repeat(fill)),
        sgr(color, "2", "\u{256e}")
    )
}

/// Bottom border: "╰────────────╯" at exactly `cols` visible columns.
pub fn composer_bottom(cols: usize, color: bool) -> String {
    let fill = cols.saturating_sub(2);
    sgr(
        color,
        "2",
        &format!("\u{2570}{}\u{256f}", "\u{2500}".repeat(fill)),
    )
}

/// A section rule: "── label ────────────" at exactly `cols` columns.
pub fn separator(label: &str, cols: usize, color: bool) -> String {
    let fixed = 2 + 1 + label.chars().count() + 1; // ── sp label sp
    let fill = cols.saturating_sub(fixed);
    sgr(
        color,
        "2",
        &format!("\u{2500}\u{2500} {label} {}", "\u{2500}".repeat(fill)),
    )
}
