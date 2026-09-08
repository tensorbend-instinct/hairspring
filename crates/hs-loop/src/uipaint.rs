//! REPL UI painting (UI gap closure batch 1, Eric 2026-09-08: SOTA-REPL
//! UI investigation - pi/omp paint tool-call cards with timing, semantic
//! color, and ambient status; hs-repl painted raw unstyled text).
//!
//! The loop emits TYPED UI events (no print-scraping): a UiSink receives
//! one UiEvent per visible beat. The Painter renders events to any Writer
//! with semantic ANSI color when the terminal supports it and byte-clean
//! plain text when piped (result JSON on stdout stays machine-readable).

use std::io::Write;

/// UI gap #9: a theme - named roles mapped to SGR codes - drives every
/// painted surface. dark and light ship built in; a TOML file overrides
/// any subset of roles and falls back to dark for the rest. HS_THEME
/// selects: "dark", "light", or a path to a theme file.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Bright accent: model label, card arrow, composer label.
    pub accent: String,
    /// Plugin names on tool cards.
    pub tool: String,
    /// Success marks.
    pub ok: String,
    /// Failure marks.
    pub fail: String,
    /// Chrome: separators, timing, trimmed output, fences.
    pub dim: String,
    /// Metered cost.
    pub cost: String,
    /// Inline code spans.
    pub code: String,
    /// Markdown headers.
    pub header: String,
    /// List bullets.
    pub bullet: String,
}

impl Theme {
    /// The HAIRSPRING dark theme (the original hand-tuned codes).
    pub fn dark() -> Self {
        Theme {
            accent: "36;1".into(),
            tool: "36".into(),
            ok: "32".into(),
            fail: "31;1".into(),
            dim: "2".into(),
            cost: "33".into(),
            code: "36".into(),
            header: "1;4".into(),
            bullet: "36".into(),
        }
    }

    /// Light-background variant: blues over cyans, magenta over yellow
    /// (yellow on white is unreadable), faint kept for chrome.
    pub fn light() -> Self {
        Theme {
            accent: "34;1".into(),
            tool: "34".into(),
            ok: "32".into(),
            fail: "31;1".into(),
            dim: "2".into(),
            cost: "35".into(),
            code: "34".into(),
            header: "1;4".into(),
            bullet: "34".into(),
        }
    }

    const ROLES: [&'static str; 9] = [
        "accent", "tool", "ok", "fail", "dim", "cost", "code", "header", "bullet",
    ];

    fn set_role(&mut self, role: &str, code: &str) -> Result<(), String> {
        match role {
            "accent" => self.accent = code.to_string(),
            "tool" => self.tool = code.to_string(),
            "ok" => self.ok = code.to_string(),
            "fail" => self.fail = code.to_string(),
            "dim" => self.dim = code.to_string(),
            "cost" => self.cost = code.to_string(),
            "code" => self.code = code.to_string(),
            "header" => self.header = code.to_string(),
            "bullet" => self.bullet = code.to_string(),
            other => return Err(format!("unknown theme role {other:?}")),
        }
        Ok(())
    }

    /// Parse a theme file: `role = "sgr"` lines over the dark base.
    /// Unknown roles and malformed TOML are errors - a typo must not
    /// silently no-op.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let v: toml::Value = toml::from_str(text).map_err(|e| format!("theme TOML: {e}"))?;
        let table = v
            .as_table()
            .ok_or_else(|| "theme file must be a TOML table".to_string())?;
        let mut t = Theme::dark();
        for (k, val) in table {
            if !Self::ROLES.contains(&k.as_str()) {
                return Err(format!("unknown theme role {k:?}"));
            }
            let code = val
                .as_str()
                .ok_or_else(|| format!("theme role {k:?} must be a string"))?;
            t.set_role(k, code)?;
        }
        Ok(t)
    }

    /// Select by name: "dark", "light", or a path to a theme file.
    pub fn by_name(name: &str) -> Result<Self, String> {
        match name {
            "dark" => Ok(Theme::dark()),
            "light" => Ok(Theme::light()),
            path => {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("theme file {path:?}: {e}"))?;
                Theme::from_toml(&text)
            }
        }
    }

    /// HS_THEME selection for the REPL surfaces. Unset or empty is dark;
    /// a bad value is dark plus a stderr note, never a crash.
    pub fn from_env() -> Self {
        match std::env::var("HS_THEME") {
            Ok(name) if !name.trim().is_empty() => match Theme::by_name(name.trim()) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("HS_THEME: {e} - using dark");
                    Theme::dark()
                }
            },
            _ => Theme::dark(),
        }
    }
}

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
    theme: Theme,
}

impl<'a, W: Write> Painter<'a, W> {
    pub fn new(out: &'a mut W, color: bool) -> Self {
        Painter::with_theme(out, color, &Theme::dark())
    }

    /// UI gap #9: every painted surface takes its codes from a Theme.
    pub fn with_theme(out: &'a mut W, color: bool, theme: &Theme) -> Self {
        Painter {
            out,
            color,
            theme: theme.clone(),
        }
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
                let arrow = self.theme.accent.clone();
                let tool = self.theme.tool.clone();
                let dim = self.theme.dim.clone();
                self.paint(&arrow, "\u{25b6} ");
                self.paint(&tool, plugin);
                if !args_summary.is_empty() {
                    self.paint(&dim, &format!("  {args_summary}"));
                }
                let _ = writeln!(self.out);
            }
            UiEvent::ToolCallEnd {
                plugin: _,
                ok,
                output_summary,
                elapsed_ms,
            } => {
                let dim = self.theme.dim.clone();
                let (code, mark) = if *ok {
                    (self.theme.ok.clone(), "\u{2713} ok")
                } else {
                    (self.theme.fail.clone(), "\u{2717} fail")
                };
                let _ = write!(self.out, "  ");
                self.paint(&code, mark);
                self.paint(&dim, &format!("  {elapsed_ms}ms"));
                if !output_summary.is_empty() {
                    let _ = writeln!(self.out);
                    self.paint(&dim, &format!("  {output_summary}"));
                }
                let _ = writeln!(self.out);
            }
            UiEvent::ModelCallStart { model } => {
                let dim = self.theme.dim.clone();
                self.paint(&dim, &format!("  … {model}"));
                let _ = writeln!(self.out);
            }
            UiEvent::ModelCallEnd {
                model: _,
                input_tokens,
                output_tokens,
            } => {
                let dim = self.theme.dim.clone();
                self.paint(
                    &dim,
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
        let accent = self.theme.accent.clone();
        let dim = self.theme.dim.clone();
        let cost = self.theme.cost.clone();
        self.paint(&accent, &v.model_label);
        self.paint(&dim, " \u{00b7} ");
        let missions = format!(
            "{} mission{}",
            v.missions_run,
            if v.missions_run == 1 { "" } else { "s" }
        );
        self.paint("0", &missions);
        self.paint(&dim, " \u{00b7} ");
        self.paint("0", &format!("{} steps", v.total_steps));
        self.paint(&dim, " \u{00b7} ");
        self.paint("0", &format!("{} calls", v.total_model_calls));
        self.paint(&dim, " \u{00b7} ");
        self.paint(&cost, &format_usd_micros(v.total_cost_micros));
        self.paint(&dim, " \u{00b7} ");
        self.paint(&dim, &format_elapsed(v.elapsed));
        self.paint(&dim, " \u{00b7} ");
        let short: String = v.stream_id.to_string().chars().take(8).collect();
        self.paint(&dim, &short);
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
    theme: Theme,
}

impl MarkdownStreamer {
    pub fn new(color: bool) -> Self {
        MarkdownStreamer::with_theme(color, &Theme::dark())
    }

    /// UI gap #9: markdown chrome (code, headers, bullets, fences) comes
    /// from the theme.
    pub fn with_theme(color: bool, theme: &Theme) -> Self {
        MarkdownStreamer {
            color,
            buf: String::new(),
            in_fence: false,
            theme: theme.clone(),
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
            let fence = format!("2;{}", self.theme.code);
            self.styled(out, &fence.clone(), line); // dim code color, verbatim
            return;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
            let text = trimmed[hashes + 1..].trim_end();
            let header = self.theme.header.clone();
            self.render_inline(out, text, &header);
            return;
        }
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            let indent = &line[..line.len() - trimmed.len()];
            let _ = write!(out, "{indent}");
            let bullet = self.theme.bullet.clone();
            self.styled(out, &bullet, "\u{2022}");
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
                        let code = self.theme.code.clone();
                        self.styled(out, &code, &after[..close]);
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

/// Top border with the dark theme (back-compat wrapper).
pub fn composer_top(label: &str, cols: usize, color: bool) -> String {
    composer_top_themed(label, cols, color, &Theme::dark())
}

/// Top border: "╭─ label ────────╮" at exactly `cols` visible columns.
pub fn composer_top_themed(label: &str, cols: usize, color: bool, theme: &Theme) -> String {
    let fixed = 2 + 1 + label.chars().count() + 1 + 1; // ╭─ sp label sp ╮
    let fill = cols.saturating_sub(fixed);
    format!(
        "{} {} {}{}",
        sgr(color, &theme.dim, "\u{256d}\u{2500}"),
        sgr(color, &theme.accent, label),
        sgr(color, &theme.dim, &"\u{2500}".repeat(fill)),
        sgr(color, &theme.dim, "\u{256e}")
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
