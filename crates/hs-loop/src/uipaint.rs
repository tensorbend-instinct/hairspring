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
}
