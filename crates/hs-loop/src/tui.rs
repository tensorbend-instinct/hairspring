//! UI gap #10: the ratatui-class full-screen surface (Eric 2026-09-08
//! via Main: full-screen is THE BUILD; loop substrate - phase indicator,
//! event-stream ticker, delegation graph - is first-class).
//!
//! Four regions: transcript viewport (grows), one-row loop rail, pinned
//! composer box, one-row HUD. M1 ships the layout + skeleton rendering
//! against ratatui's `TestBackend`; line mode stays the piped fallback.

use std::collections::VecDeque;

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use hs_core::EventKind;

/// Where in the agent loop cycle the mission is right now, derived from
/// the latest stream events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopPhase {
    /// M17: no mission in flight - the rail lights nothing. Boot and
    /// post-mission both sit here; pre-M17 the rail glowed a stale
    /// REFLECT over an idle composer.
    #[default]
    Idle,
    /// Model call in flight, deciding the next action.
    Plan,
    /// A tool call is executing.
    Act,
    /// A tool result / observation just landed.
    Observe,
    /// Decision / feedback bookkeeping.
    Reflect,
}

impl LoopPhase {
    /// Display order on the loop rail.
    pub const ALL: [LoopPhase; 4] = [
        LoopPhase::Plan,
        LoopPhase::Act,
        LoopPhase::Observe,
        LoopPhase::Reflect,
    ];

    fn label(self) -> &'static str {
        match self {
            LoopPhase::Idle => "",
            LoopPhase::Plan => "PLAN",
            LoopPhase::Act => "ACT",
            LoopPhase::Observe => "OBSERVE",
            LoopPhase::Reflect => "REFLECT",
        }
    }
}

/// One glyph per stream event kind for the loop-rail ticker.
#[must_use]
pub fn kind_glyph(k: EventKind) -> char {
    match k {
        EventKind::ModelCall => '\u{25c6}',       // ◆
        EventKind::ToolCall => '\u{2699}',        // ⚙
        EventKind::Observation => '\u{25c8}',     // ◈
        EventKind::Decision => '\u{2726}',        // ✦
        EventKind::ContextInject => '\u{21b3}',   // ↳
        EventKind::Feedback => '\u{2713}',        // ✓
        EventKind::SnapshotRef => '\u{2398}',     // ⎘
        EventKind::Proposal => '\u{2731}',        // ✱
        EventKind::Consequence => '\u{21af}',     // ↯
        EventKind::GoalUpdate => '\u{25ce}',      // ◎
        EventKind::BudgetUpdate => '$',
        EventKind::Spawn => '\u{2b21}',           // ⬡
        EventKind::Message => '\u{2709}',         // ✉
        EventKind::Mutation => '\u{270e}',        // ✎
        EventKind::Score => '\u{2605}',           // ★
        EventKind::ScorerPin => '\u{1f4cc}',      // 📌
        EventKind::CanaryResult => '\u{1f41e}',   // 🐞
        EventKind::AnchorResult => '\u{2693}',     // ⚓
        EventKind::Prefetch => '\u{21bb}',        // ↻
        EventKind::CapabilityDelta => '\u{0394}', // Δ
        EventKind::FitnessDelta => '\u{2206}',    // ∆
        EventKind::Regression => '\u{26a0}',      // ⚠
        EventKind::CapabilityChange => '\u{21c4}',// ⇄
    }
}

/// M26: the full-screen surface's OWN help - every command the TUI
/// actually implements (bin/hs-repl.rs Submit arms + `handle_key`), no
/// line-mode leftovers. Pre-M26 the TUI printed the line-mode
/// `REPL_HELP`, which advertised :status/:history/:last as dead ends
/// and never mentioned :resume/:agents.
pub const TUI_HELP: &str = "hairspring - full-screen surface
  <text>    run <text> as a goal (queues behind a running mission)
  /status   model, missions, steps, calls, cost, stream of this session
  /history  goals you have submitted this session
  /last     the latest mission's answer artifact
  /resume   pick a prior session to continue
  /models   pick the operator model (next mission onward)
  /theme    pick the surface theme
  /agents   toggle the delegation graph panel
  /lineage  toggle the selfmod lineage panel
  /scorer   toggle the scorer stream panel
  /evidence toggle the evidence claims panel
  /time     toggle the T_mission decomposition panel
  /help     this text
  /quit     exit (Ctrl+C works too)
  type / to open the live palette; :command is a backward-compatible alias
  keys: Enter run - Alt+Enter newline - ^C interrupt/clear/quit - PgUp/PgDn scroll";

/// The command registry behind the palette (Eric 2026-09-10: "commands
/// in common TUIs are /<command> with immediate feedback on options").
/// Canonical spelling is "/name"; ":" is a backward-compatible alias
/// resolving the same set. Both the handle_key dispatch and the bin's
/// Submit handler read this table - one source, no drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: &'static str,
    pub summary: &'static str,
    /// Argument hint for the palette row (None = takes no args; the
    /// current set is arg-less, the slot renders for future commands).
    pub args: Option<&'static str>,
}

pub const TUI_COMMANDS: &[CommandSpec] = &[
    CommandSpec { name: "help", summary: "list commands", args: None },
    CommandSpec { name: "status", summary: "model, missions, steps, calls, cost, stream", args: None },
    CommandSpec { name: "history", summary: "goals submitted this session", args: None },
    CommandSpec { name: "last", summary: "the latest mission's answer artifact", args: None },
    CommandSpec { name: "resume", summary: "pick a prior session to continue", args: None },
    CommandSpec { name: "models", summary: "pick the operator model", args: None },
    CommandSpec { name: "theme", summary: "pick the surface theme", args: None },
    CommandSpec { name: "agents", summary: "toggle the delegation graph panel", args: None },
    CommandSpec { name: "lineage", summary: "toggle the selfmod lineage panel", args: None },
    CommandSpec { name: "scorer", summary: "toggle the scorer stream panel", args: None },
    CommandSpec { name: "evidence", summary: "toggle the evidence claims panel", args: None },
    CommandSpec { name: "time", summary: "toggle the T_mission decomposition panel", args: None },
    CommandSpec { name: "quit", summary: "exit (Ctrl+C works too)", args: None },
];

/// Exact command resolution (the "q" shorthand rides along).
pub fn command_lookup(name: &str) -> Option<&'static CommandSpec> {
    let name = if name == "q" { "quit" } else { name };
    TUI_COMMANDS.iter().find(|c| c.name == name)
}

/// Palette filtering: commands whose name starts with the typed prefix
/// (empty prefix = the whole registry).
pub fn command_matches(prefix: &str) -> Vec<&'static CommandSpec> {
    TUI_COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(prefix))
        .collect()
}

/// The four screen regions. The composer and HUD are pinned at the
/// bottom; the viewport takes everything above the rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiLayout {
    pub viewport: Rect,
    pub rail: Rect,
    pub composer: Rect,
    pub hud: Rect,
}

/// Split a `width`x`height` screen. Degenerate sizes clamp instead of
/// panicking - regions may overlap below ~5 rows; rendering clips.
#[must_use]
pub fn layout(width: u16, height: u16) -> TuiLayout {
    let hud = Rect::new(0, height.saturating_sub(1), width, height.min(1));
    let composer = Rect::new(
        0,
        height.saturating_sub(4),
        width,
        height.saturating_sub(1).min(3),
    );
    let rail = Rect::new(
        0,
        height.saturating_sub(5),
        width,
        height.saturating_sub(4).min(1),
    );
    let viewport = Rect::new(0, 0, width, height.saturating_sub(5));
    TuiLayout {
        viewport,
        rail,
        composer,
        hud,
    }
}

/// M3: map a theme SGR code string ("36;1", "2", "31;1") to a ratatui
/// Style. Supports the codes Theme roles use: 30-37/90-97 fg, 1 bold,
/// 2 dim, 4 underline.
#[must_use]
pub fn sgr_style(code: &str) -> Style {
    let mut style = Style::default();
    for part in code.split(';') {
        style = match part.trim() {
            "0" => Style::default(),
            "1" => style.add_modifier(Modifier::BOLD),
            "2" => style.add_modifier(Modifier::DIM),
            "4" => style.add_modifier(Modifier::UNDERLINED),
            "30" => style.fg(Color::Black),
            "31" => style.fg(Color::Red),
            "32" => style.fg(Color::Green),
            "33" => style.fg(Color::Yellow),
            "34" => style.fg(Color::Blue),
            "35" => style.fg(Color::Magenta),
            "36" => style.fg(Color::Cyan),
            "37" | "97" => style.fg(Color::White),
            "90" => style.fg(Color::DarkGray),
            "91" => style.fg(Color::LightRed),
            "92" => style.fg(Color::LightGreen),
            "93" => style.fg(Color::LightYellow),
            "94" => style.fg(Color::LightBlue),
            "95" => style.fg(Color::LightMagenta),
            "96" => style.fg(Color::LightCyan),
            _ => style,
        };
    }
    style
}

/// M3: convert markdown text into styled ratatui Lines - the same
/// rules the line-mode `MarkdownStreamer` paints (headers, bullets,
/// fences, inline code, bold), driven by the Theme. Fence bodies pass
/// through verbatim, dimmed in the code color.
#[must_use]
pub fn md_to_lines(md: &str, theme: &crate::uipaint::Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let fence_style = sgr_style(&format!("2;{}", theme.code));
    for raw in md.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push(Line::from(Span::styled(raw.to_string(), fence_style)));
            continue;
        }
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if hashes >= 1 && trimmed.chars().nth(hashes) == Some(' ') {
            let text = trimmed[hashes + 1..].trim_end();
            // H1 carries the full header treatment; deeper levels keep
            // the weight but drop the underline (visual hierarchy).
            let style = if hashes == 1 {
                sgr_style(&theme.header)
            } else {
                sgr_style(&theme.header).remove_modifier(Modifier::UNDERLINED)
            };
            out.push(Line::from(inline_spans(text, style, theme)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let indent = raw.len() - trimmed.len();
            let mut spans = vec![
                Span::raw(" ".repeat(indent)),
                Span::styled("\u{2022} ", sgr_style(&theme.bullet)),
            ];
            spans.extend(inline_spans(rest, Style::default(), theme));
            out.push(Line::from(spans));
            continue;
        }
        out.push(Line::from(inline_spans(raw, Style::default(), theme)));
    }
    out
}

/// Inline spans: `code` in the theme's code color, **bold** bold, the
/// rest in the base style. Markers are consumed; unmatched markers
/// render literally.
fn inline_spans(text: &str, base: Style, theme: &crate::uipaint::Theme) -> Vec<Span<'static>> {
    let code_style = sgr_style(&theme.code);
    let bold = base.add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let tick = rest.find('`');
        let star = rest.find("**");
        let next = match (tick, star) {
            (Some(t), Some(s)) => Some(t.min(s)),
            (a, b) => a.or(b),
        };
        let Some(i) = next else {
            spans.push(Span::styled(rest.to_string(), base));
            break;
        };
        if i > 0 {
            spans.push(Span::styled(rest[..i].to_string(), base));
        }
        let after = &rest[i..];
        if let Some(body) = after.strip_prefix('`') {
            if let Some(close) = body.find('`')
                && close > 0 {
                    spans.push(Span::styled(body[..close].to_string(), code_style));
                    rest = &body[close + 1..];
                    continue;
                }
            spans.push(Span::styled("`".to_string(), base));
            rest = body;
        } else if let Some(body) = after.strip_prefix("**") {
            if let Some(close) = body.find("**")
                && close > 0 {
                    spans.push(Span::styled(body[..close].to_string(), bold));
                    rest = &body[close + 2..];
                    continue;
                }
            spans.push(Span::styled("**".to_string(), base));
            rest = body;
        } else {
            spans.push(Span::styled(after.to_string(), base));
            break;
        }
    }
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

/// M2: the composer editor - multi-line editing with cursor movement,
/// history, and submit. Pure state; rendering and crossterm key
/// translation live at the edges.
#[derive(Debug, Clone, Default)]
pub struct EditorState {
    lines: Vec<String>,
    row: usize,
    col: usize, // in chars, not bytes
    history: std::collections::VecDeque<String>,
    hist_idx: Option<usize>,
    stash: String,
}

const HISTORY_CAP: usize = 100;

impl EditorState {
    /// M26: ":history" data - submitted entries, oldest first.
    #[must_use]
    pub fn history_entries(&self) -> Vec<String> {
        self.history.iter().cloned().collect()
    }
}

impl EditorState {
    /// Whole buffer, lines joined by newlines.
    #[must_use]
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// (row, col) in chars.
    #[must_use]
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map_or(0, |l| l.chars().count())
    }

    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(str::to_string).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.row = self.lines.len() - 1;
        self.col = self.line_len(self.row);
    }

    /// Insert one char at the cursor.
    pub fn input_char(&mut self, c: char) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        let line = &mut self.lines[self.row];
        let byte = line.char_indices().nth(self.col).map_or(line.len(), |(b, _)| b);
        line.insert(byte, c);
        self.col += 1;
    }

    /// Delete the char left of the cursor; at column 0 join with the
    /// line above.
    pub fn backspace(&mut self) {
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            let byte = line.char_indices().nth(self.col - 1).map_or(0, |(b, _)| b);
            line.remove(byte);
            self.col -= 1;
        } else if self.row > 0 {
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.line_len(self.row);
            self.lines[self.row].push_str(&cur);
        }
    }

    /// Split the current line at the cursor.
    pub fn insert_newline(&mut self) {
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        let line = &mut self.lines[self.row];
        let byte = line.char_indices().nth(self.col).map_or(line.len(), |(b, _)| b);
        let tail = line[byte..].to_string();
        line.truncate(byte);
        self.lines.insert(self.row + 1, tail);
        self.row += 1;
        self.col = 0;
    }

    pub fn move_left(&mut self) {
        self.col = self.col.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.col = (self.col + 1).min(self.line_len(self.row));
    }

    pub fn move_home(&mut self) {
        self.col = 0;
    }

    pub fn move_end(&mut self) {
        self.col = self.line_len(self.row);
    }

    pub fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    pub fn move_down(&mut self) {
        if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = self.col.min(self.line_len(self.row));
        }
    }

    /// Insert a pasted/typed string: newlines split lines (same as
    /// Alt+Enter), tabs become two spaces for predictable layout, and
    /// other control chars are stripped. Never submits.
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '\n' => self.insert_newline(),
                '\t' => {
                    self.input_char(' ');
                    self.input_char(' ');
                }
                c if c.is_control() => {}
                c => self.input_char(c),
            }
        }
    }

    /// Delete the char under the cursor; at end of line join with the
    /// line below (Delete key, Ctrl+D on a non-empty buffer).
    pub fn delete_forward(&mut self) {
        if self.col < self.line_len(self.row) {
            let line = &mut self.lines[self.row];
            let byte = line.char_indices().nth(self.col).map_or(line.len(), |(b, _)| b);
            line.remove(byte);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    /// Kill the current line's text before the cursor (Ctrl+U).
    pub fn kill_to_start(&mut self) {
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            let byte = line.char_indices().nth(self.col).map_or(line.len(), |(b, _)| b);
            line.replace_range(..byte, "");
            self.col = 0;
        }
    }

    /// Kill the current line's text from the cursor on (Ctrl+K). At end
    /// of a non-last line the newline dies instead, joining the next.
    pub fn kill_to_end(&mut self) {
        let len = self.line_len(self.row);
        if self.col < len {
            let line = &mut self.lines[self.row];
            let byte = line.char_indices().nth(self.col).map_or(line.len(), |(b, _)| b);
            line.truncate(byte);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    /// Column a word-left move would land on: skip whitespace left,
    /// then skip the word left. Line-local (no row hops).
    fn word_left_col(&self) -> usize {
        let line: Vec<char> = self.lines.get(self.row).map(|l| l.chars().collect()).unwrap_or_default();
        let mut c = self.col.min(line.len());
        while c > 0 && line[c - 1].is_whitespace() {
            c -= 1;
        }
        while c > 0 && !line[c - 1].is_whitespace() {
            c -= 1;
        }
        c
    }

    /// Word back (Alt+Left / Alt+B / Ctrl+Left).
    pub fn move_word_left(&mut self) {
        self.col = self.word_left_col();
    }

    /// Word forward (Alt+Right / Alt+F / Ctrl+Right): skip the word,
    /// then the whitespace after it. Line-local.
    pub fn move_word_right(&mut self) {
        let line: Vec<char> = self.lines.get(self.row).map(|l| l.chars().collect()).unwrap_or_default();
        let len = line.len();
        let mut c = self.col.min(len);
        while c < len && !line[c].is_whitespace() {
            c += 1;
        }
        while c < len && line[c].is_whitespace() {
            c += 1;
        }
        self.col = c;
    }

    /// Delete the word before the cursor (Ctrl+W).
    pub fn delete_word_back(&mut self) {
        let target = self.word_left_col();
        while self.col > target {
            self.backspace();
        }
    }

    /// Submit the buffer: returns the text, clears the editor, pushes
    /// history. An empty buffer submits nothing.
    pub fn submit(&mut self) -> Option<String> {
        let text = self.text();
        if text.trim().is_empty() {
            return None;
        }
        self.history.push_back(text.clone());
        while self.history.len() > HISTORY_CAP {
            self.history.pop_front();
        }
        self.set_text("");
        self.hist_idx = None;
        self.stash.clear();
        Some(text)
    }

    /// Walk history toward older entries; returns the now-current text.
    pub fn history_up(&mut self) -> Option<String> {
        if self.history.is_empty() {
            return None;
        }
        let idx = match self.hist_idx {
            None => {
                self.stash = self.text();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.hist_idx = Some(idx);
        let entry = self.history[idx].clone();
        self.set_text(&entry);
        Some(self.text())
    }

    /// Walk history toward newer entries, then back to the stashed live
    /// line.
    pub fn history_down(&mut self) -> Option<String> {
        let idx = self.hist_idx?;
        if idx + 1 < self.history.len() {
            self.hist_idx = Some(idx + 1);
            let entry = self.history[idx + 1].clone();
            self.set_text(&entry);
        } else {
            self.hist_idx = None;
            let stash = self.stash.clone();
            self.set_text(&stash);
        }
        Some(self.text())
    }

    /// Line count (drives composer height).
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.lines.len().max(1)
    }
}

/// M7: what a key event means for the surface. The bin's terminal loop
/// maps every key through `handle_key`; only Submit/Picked leave the UI
/// layer (the caller dispatches the mission / applies the choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Handled inside the surface; keep looping.
    Continue,
    /// Editor submitted a line (mission text or a caller command).
    Submit(String),
    /// Ctrl+C while a mission is in flight: stop it cleanly at the
    /// next step boundary (the bin touches the interrupt file; the
    /// mission is booked "interrupted", never a harness error).
    Interrupt,
    /// Picker chose an entry (kind, entry text).
    Picked(PickerKind, String),
    /// ":agents" toggled the delegation panel.
    ToggleAgents,
    /// ":lineage" toggled the selfmod lineage panel.
    ToggleLineage,
    /// ":scorer" toggled the scorer stream panel.
    ToggleScorer,
    /// ":evidence" toggled the evidence claims panel.
    ToggleEvidence,
    /// ":time" toggled the `T_mission` decomposition panel.
    ToggleTime,
    /// Ctrl+C or ":quit".
    Quit,
}

/// Translate one key event into a surface action. Picker-open mode
/// owns Up/Down/Enter/Esc; otherwise the editor owns keys, PgUp/PgDn
/// scroll the transcript, and Enter submits (Alt+Enter = newline).
pub fn handle_key(state: &mut TuiState, key: ratatui::crossterm::event::KeyEvent) -> KeyAction {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    // Picker mode eats navigation before the editor sees it.
    if state.picker.is_some() {
        match key.code {
            KeyCode::Up => {
                state.picker_up();
                return KeyAction::Continue;
            }
            KeyCode::Down => {
                state.picker_down();
                return KeyAction::Continue;
            }
            KeyCode::Enter => {
                return match state.picker_take() {
                    Some(choice) => KeyAction::Picked(choice.0, choice.1),
                    None => KeyAction::Continue,
                };
            }
            KeyCode::Esc => {
                state.picker_cancel();
                return KeyAction::Continue;
            }
            _ => return KeyAction::Continue,
        }
    }

    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            // Claude Code escalation: interrupt a running mission, else
            // clear a non-empty buffer, else quit. One press, one step.
            if state.phase != LoopPhase::Idle {
                KeyAction::Interrupt
            } else if !state.editor.text().is_empty() {
                state.editor.set_text("");
                state.palette = None;
                KeyAction::Continue
            } else {
                KeyAction::Quit
            }
        }
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
            // EOF semantics: quit on an empty buffer, forward-delete
            // on a non-empty one.
            if state.editor.text().is_empty() {
                KeyAction::Quit
            } else {
                state.editor.delete_forward();
                state.palette_sync();
                KeyAction::Continue
            }
        }
        (KeyCode::Char('a'), KeyModifiers::CONTROL) => {
            state.editor.move_home();
            KeyAction::Continue
        }
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => {
            state.editor.move_end();
            KeyAction::Continue
        }
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            state.editor.kill_to_start();
            state.palette_sync();
            KeyAction::Continue
        }
        (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
            state.editor.kill_to_end();
            state.palette_sync();
            KeyAction::Continue
        }
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
            state.editor.delete_word_back();
            state.palette_sync();
            KeyAction::Continue
        }
        (KeyCode::Char('b'), KeyModifiers::ALT) => {
            state.editor.move_word_left();
            KeyAction::Continue
        }
        (KeyCode::Char('f'), KeyModifiers::ALT) => {
            state.editor.move_word_right();
            KeyAction::Continue
        }
        (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
            state.editor.input_char(c);
            state.palette_sync();
            KeyAction::Continue
        }
        (KeyCode::Enter, KeyModifiers::ALT) => {
            state.editor.insert_newline();
            KeyAction::Continue
        }
        (KeyCode::Enter, _) => {
            // Palette acceptance: with the palette open and the
            // highlight differing from the typed prefix, Enter first
            // accepts the command into the buffer in canonical
            // spelling; the exact name then dispatches below.
            // Arg-taking commands stop at the buffer for the argument.
            if let Some(p) = &state.palette {
                if !p.matches.is_empty() {
                    let cmd = p.matches[p.selected.min(p.matches.len() - 1)];
                    let text = state.editor.text().trim().to_string();
                    let typed = text
                        .strip_prefix('/')
                        .or_else(|| text.strip_prefix(':'))
                        .unwrap_or("");
                    if typed != cmd.name {
                        if cmd.args.is_some() {
                            state.editor.set_text(&format!("/{} ", cmd.name));
                            state.palette_sync();
                            return KeyAction::Continue;
                        }
                        state.editor.set_text(&format!("/{}", cmd.name));
                    }
                }
            }
            match state.editor.submit() {
                None => KeyAction::Continue,
                Some(text) => {
                    state.palette = None;
                    let t = text.trim();
                    match t.strip_prefix('/').or_else(|| t.strip_prefix(':')) {
                        Some(rest) => {
                            let word = rest.split_whitespace().next().unwrap_or("");
                            match command_lookup(word) {
                                None => {
                                    state.push_transcript_line(&format!(
                                        "unknown command {t} (/help lists commands)"
                                    ));
                                    KeyAction::Continue
                                }
                                Some(spec) => match spec.name {
                                    "quit" => KeyAction::Quit,
                                    "agents" => {
                                        state.toggle_agents_panel();
                                        KeyAction::ToggleAgents
                                    }
                                    "lineage" => {
                                        state.toggle_lineage_panel();
                                        KeyAction::ToggleLineage
                                    }
                                    "scorer" => {
                                        state.toggle_scorer_panel();
                                        KeyAction::ToggleScorer
                                    }
                                    "evidence" => {
                                        state.toggle_evidence_panel();
                                        KeyAction::ToggleEvidence
                                    }
                                    "time" => {
                                        state.toggle_time_panel();
                                        KeyAction::ToggleTime
                                    }
                                    _ => KeyAction::Submit(text),
                                },
                            }
                        }
                        None => KeyAction::Submit(text),
                    }
                }
            }
        }
        (KeyCode::Backspace, _) => {
            state.editor.backspace();
            state.palette_sync();
            KeyAction::Continue
        }
        (KeyCode::Delete, _) => {
            state.editor.delete_forward();
            state.palette_sync();
            KeyAction::Continue
        }
        (KeyCode::Left, KeyModifiers::ALT) | (KeyCode::Left, KeyModifiers::CONTROL) => {
            state.editor.move_word_left();
            KeyAction::Continue
        }
        (KeyCode::Right, KeyModifiers::ALT) | (KeyCode::Right, KeyModifiers::CONTROL) => {
            state.editor.move_word_right();
            KeyAction::Continue
        }
        (KeyCode::Left, _) => {
            state.editor.move_left();
            KeyAction::Continue
        }
        (KeyCode::Right, _) => {
            state.editor.move_right();
            KeyAction::Continue
        }
        (KeyCode::Home, _) => {
            state.editor.move_home();
            KeyAction::Continue
        }
        (KeyCode::End, _) => {
            state.editor.move_end();
            KeyAction::Continue
        }
        (KeyCode::Up, _) => {
            // The open palette owns Up/Down (selection); otherwise
            // history when the cursor sits on the first row, plain
            // cursor movement inside a multi-line buffer.
            if state.palette.is_some() {
                state.palette_up();
            } else if state.editor.cursor().0 == 0 {
                state.editor.history_up();
            } else {
                state.editor.move_up();
            }
            KeyAction::Continue
        }
        (KeyCode::Down, _) => {
            if state.palette.is_some() {
                state.palette_down();
                return KeyAction::Continue;
            }
            let (row, _) = state.editor.cursor();
            let last = state.editor.line_count().saturating_sub(1);
            if row >= last {
                state.editor.history_down();
            } else {
                state.editor.move_down();
            }
            KeyAction::Continue
        }
        (KeyCode::PageUp, _) => {
            state.transcript_page_up(20);
            KeyAction::Continue
        }
        (KeyCode::PageDown, _) => {
            state.transcript_wheel_down(20);
            KeyAction::Continue
        }
        (KeyCode::Tab, _) => {
            // Complete the highlighted palette command into the
            // buffer (canonical spelling, no submit).
            state.palette_complete();
            KeyAction::Continue
        }
        (KeyCode::Esc, _) => {
            // Esc closes the palette (the picker handled its own Esc
            // above). A leftover sigil in the composer silently
            // turned the next plain-text goal into an "unknown
            // command" (live capture cap-22, 60x20), so closing also
            // clears a sigil-only buffer; plain text is untouched.
            if state.palette.is_some() {
                state.palette = None;
                let t = state.editor.text().trim_start().to_string();
                if t.starts_with('/') || t.starts_with(':') {
                    state.editor.set_text("");
                }
            }
            KeyAction::Continue
        }
        _ => KeyAction::Continue,
    }
}

/// M29: one pasted block is ONE buffer. The bin enables bracketed paste
/// (ESC[?2004h) so the terminal delivers the whole block as a single
/// `Event::Paste`; newlines inside it become buffer newlines (the same
/// as Alt+Enter), never submits. Without this every pasted line arrived
/// as its own Enter key event and launched its own mission - Eric's
/// "pasting multi-line text creates new missions" report.
pub fn handle_paste(state: &mut TuiState, text: &str) -> KeyAction {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    state.editor.insert_str(&normalized);
    state.palette_sync();
    KeyAction::Continue
}

/// M6: delegation graph - the loop substrate's Spawn/Message structure
/// rendered as a first-class surface element. Nodes derive from Spawn
/// events on the operator stream; completion derives from the child's
/// own `GoalUpdate` (done flag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Done,
    Failed,
}

/// One delegated sub-agent: the Spawn provenance (parent edge, model),
/// not just the child id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNode {
    pub stream_id: uuid::Uuid,
    /// The delegating stream (None = the operator's own stream).
    pub parent: Option<uuid::Uuid>,
    /// The model the child runs, from the Spawn payload.
    pub model: String,
    pub mission: String,
    pub status: AgentStatus,
}

/// The delegation tree for the current session's stream.
#[derive(Debug, Clone, Default)]
pub struct DelegationGraph {
    nodes: Vec<AgentNode>,
}

impl DelegationGraph {
    #[must_use]
    pub fn new() -> Self {
        DelegationGraph::default()
    }

    #[must_use]
    pub fn nodes(&self) -> &[AgentNode] {
        &self.nodes
    }

    /// Record a Spawn: child stream + parent edge + model + mission,
    /// status Running.
    pub fn note_spawn(
        &mut self,
        child: uuid::Uuid,
        parent: Option<uuid::Uuid>,
        mission: &str,
        model: &str,
    ) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.stream_id == child) {
            n.mission = mission.to_string();
            n.model = model.to_string();
            n.parent = parent;
            return;
        }
        self.nodes.push(AgentNode {
            stream_id: child,
            parent,
            model: model.to_string(),
            mission: mission.to_string(),
            status: AgentStatus::Running,
        });
    }

    /// Depth in the delegation tree: ancestors present in the graph.
    /// Cycle-safe (a corrupt graph renders flat rather than hanging).
    fn depth(&self, node: &AgentNode) -> usize {
        let mut d = 0;
        let mut cur = node.parent;
        let mut seen = std::collections::HashSet::new();
        while let Some(p) = cur {
            if !seen.insert(p) {
                break;
            }
            match self.nodes.iter().find(|n| n.stream_id == p) {
                Some(n) => {
                    d += 1;
                    cur = n.parent;
                }
                None => break,
            }
        }
        d
    }

    /// Record a child completion; newest knowledge wins.
    pub fn note_done(&mut self, child: uuid::Uuid, ok: bool) {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.stream_id == child) {
            n.status = if ok { AgentStatus::Done } else { AgentStatus::Failed };
        }
    }

    /// Derive the graph from the durable log: Spawn events on the
    /// operator stream name children; each child's completion comes
    /// from its own stream's latest `GoalUpdate`.
    pub fn scan_stream(
        log_root: &std::path::Path,
        stream: uuid::Uuid,
    ) -> Result<Self, String> {
        let reader = hs_log::StreamReader::open(log_root, stream)
            .map_err(|e| format!("open stream {stream}: {e}"))?;
        let events = reader.events().map_err(|e| format!("read stream: {e}"))?;
        let mut g = DelegationGraph::new();
        for ev in &events {
            if ev.kind != EventKind::Spawn {
                continue;
            }
            let hs_core::Payload::Inline(bytes) = &ev.payload else {
                continue;
            };
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
                continue;
            };
            let Some(child_s) = v.get("child_stream_id").and_then(|c| c.as_str()) else {
                continue;
            };
            let Ok(child) = uuid::Uuid::parse_str(child_s) else {
                continue;
            };
            let mission = v
                .get("mission")
                .and_then(|m| m.as_str())
                .unwrap_or("(delegation)")
                .to_string();
            // Streams booked before the model field existed read as
            // unknown, never as an invented name.
            let model = v
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or("(unknown)")
                .to_string();
            g.note_spawn(child, Some(stream), &mission, &model);
            // Completion: the child stream's latest GoalUpdate done flag.
            if let Ok(cr) = hs_log::StreamReader::open(log_root, child)
                && let Ok(cevents) = cr.events() {
                    for cev in cevents.iter().rev() {
                        if cev.kind != EventKind::GoalUpdate {
                            continue;
                        }
                        if let hs_core::Payload::Inline(cb) = &cev.payload
                            && let Ok(cv) = serde_json::from_slice::<serde_json::Value>(cb) {
                                // M12: terminal = done:true, or any close
                                // carrying an outcome. hs-swarm's spawn-time
                                // GoalUpdate {done:false} has no outcome -
                                // an OPEN goal, still Running.
                                let done = cv.get("done").and_then(serde_json::Value::as_bool);
                                if done == Some(true) || cv.get("outcome").is_some() {
                                    g.note_done(child, done.unwrap_or(false));
                                }
                            }
                        break; // latest GoalUpdate decides
                    }
                }
        }
        Ok(g)
    }
}

/// M14: replay a resumed stream's visible history into the
/// transcript - goal echoes, committed answer blocks, and
/// per-mission done lines, rendered through the same paths as the
/// live surface (`push_goal_echo` / `push_transcript_markdown`).
/// Internal distill calls never render. Pre-M14 a resume switched
/// the stream but left the screen showing only "resumed stream X"
/// (v2 cap6); pi/omp restore history on resume.
pub fn backfill_transcript(
    st: &mut TuiState,
    log_root: &std::path::Path,
    stream_id: uuid::Uuid,
) {
    let Ok(reader) = hs_log::StreamReader::open(log_root, stream_id) else {
        return;
    };
    let Ok(events) = reader.events() else { return };
    let theme = st.theme.clone();
    let mut mission_open = false;
    let mut steps: u64 = 0;
    let mut calls: u64 = 0;
    let mut cost: u64 = 0;
    for ev in &events {
        match ev.kind {
            hs_core::EventKind::ModelCall => {
                let Ok(bytes) = reader.resolve_payload(ev) else { continue };
                let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    continue;
                };
                let distill =
                    v.get("why").and_then(|w| w.as_str()) == Some("distill");
                calls += 1;
                // The recorded per-call cost (Event.cost_usd_micros),
                // never a token-rate estimate: a resumed session's
                // done line must show the SAME cost the live HUD
                // showed (cap11: estimate read $0.0007 where live
                // read $0.0014).
                cost += ev.cost_usd_micros.max(0) as u64;
                if distill {
                    continue; // internal call: counted, never rendered
                }
                steps += 1;
                if !mission_open {
                    mission_open = true;
                    if let Some(goal) = extract_mission_text(&v) {
                        st.push_goal_echo(&goal);
                    }
                }
                let completion =
                    v.get("completion").and_then(|c| c.as_str()).unwrap_or("");
                if !completion.trim().is_empty() {
                    st.push_transcript_markdown(completion, &theme);
                }
            }
            hs_core::EventKind::GoalUpdate => {
                let Ok(bytes) = reader.resolve_payload(ev) else { continue };
                let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    continue;
                };
                // M12 semantics: terminal iff done:true or an outcome
                // key; hs-swarm's spawn-time done:false is an OPEN
                // goal and must not print a done line.
                let terminal = v.get("done").and_then(serde_json::Value::as_bool) == Some(true)
                    || v.get("outcome").is_some();
                if terminal && mission_open {
                    let outcome =
                        v.get("outcome").and_then(|o| o.as_str()).unwrap_or("done");
                    let verdict = if outcome == "verified" {
                        st.theme.ok.clone()
                    } else {
                        st.theme.fail.clone()
                    };
                    st.push_transcript_styled(
                        &format!(
                            "\u{2500}\u{2500} done: {steps} steps, {calls} calls, {} ({outcome})",
                            crate::uipaint::format_usd_micros(cost)
                        ),
                        sgr_style(&verdict).add_modifier(Modifier::BOLD),
                    );
                    mission_open = false;
                    steps = 0;
                    calls = 0;
                    cost = 0;
                }
            }
            _ => {}
        }
    }
}


/// M14: the goal text of a mission, from its first operator call's
/// messages. Prompts carry "MISSION: <goal>"; the goal runs to the
/// first blank line (the MCP catalog suffix), preserving multi-line
/// goals.
fn extract_mission_text(v: &serde_json::Value) -> Option<String> {
    let msgs = v.get("messages")?.as_array()?;
    for m in msgs {
        if let Some(c) = m.get("content").and_then(|c| c.as_str())
            && let Some(i) = c.find("MISSION: ") {
                let rest = &c[i + 9..];
                let end = rest.find("\n\n").unwrap_or(rest.len());
                let goal = rest[..end].trim();
                if !goal.is_empty() {
                    return Some(goal.to_string());
                }
            }
    }
    None
}

/// The live command palette (Eric 2026-09-10). Open while the editor
/// buffer starts with "/" or ":"; matches recompute on every edit.
#[derive(Debug, Clone, Default)]
pub struct PaletteState {
    pub matches: Vec<&'static CommandSpec>,
    pub selected: usize,
}

/// M4: the picker overlay state (resume picker first, model/theme
/// pickers later). Entries are pre-rendered display lines; selection
/// is an index.
#[derive(Debug, Clone, Default)]
pub struct PickerState {
    /// Display lines, one per entry.
    pub entries: Vec<String>,
    /// Current selection.
    pub selected: usize,
    /// What the picker is choosing (M29: resume, model, or theme).
    pub kind: PickerKind,
}

/// Which chooser an open picker serves (Eric's five #4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickerKind {
    /// A prior session to resume.
    #[default]
    Resume,
    /// The operator model for subsequent missions.
    Models,
    /// The surface theme.
    Themes,
}

/// Cap on ticker entries kept; the rail shows the tail.
const TICKER_CAP: usize = 32;

/// Everything the surface needs to render one frame. M1 keeps this
/// small; later milestones grow it (transcript lines, editor buffer,
/// delegation graph).
/// M23: the transcript word-wrapped into physical ROWS at one width,
/// kept in sync with `transcript` (extend on append, rebuild on a
/// width change) so neither the render nor the scroll math re-wraps
/// the whole history per frame. `width == 0` marks an empty cache.
#[derive(Debug, Clone, Default)]
pub struct RowsCache {
    pub width: u16,
    pub lines: Vec<Line<'static>>,
    /// How many source transcript lines `lines` was built from.
    pub src_len: usize,
}

/// M23: word-wrap one line into physical rows of at most `width`
/// terminal cells, preserving span styles. The viewport renders THESE
/// rows directly (no Paragraph reflow), so what the scroll math
/// counts is by construction what the frame draws - ratatui's own
/// `line_count` is feature-gated unstable, and trusting two different
/// wrappers is how counters drift from screens.
///
/// Rules: greedy word wrap; a word longer than the width hard-breaks
/// at the edge; the space at a break is consumed; a wide char that
/// would straddle the edge moves whole to the next row (ratatui
/// renders the same constraint); an empty line is one empty row.
#[must_use]
/// Beat 5 W2: overlay-panel content wraps to the box's inner width.
/// Ratatui's Paragraph CLIPS long lines at the border (live capture
/// cap-11: T_mission rows cut mid-word) - and a clipped number is a
/// wrong number.
pub fn wrap_panel_lines(lines: &[Line<'static>], inner_w: u16) -> Vec<Line<'static>> {
    if inner_w == 0 {
        return lines.to_vec();
    }
    lines.iter().flat_map(|l| wrap_line(l, inner_w)).collect()
}

/// The T_mission panel's inner width for a terminal width (box is
/// 3/4 of the surface, min 50, capped at the surface; two columns of
/// border). Shared by the render path and the fit test.
pub fn panel_inner_width(term_w: u16) -> u16 {
    let box_w = (term_w * 3 / 4).max(50).min(term_w);
    box_w.saturating_sub(2)
}

/// The T_mission panel's display lines, wrapped to the inner width.
pub fn time_panel_lines(
    d: &crate::mission_time::Decomposition,
    inner_w: u16,
) -> Vec<Line<'static>> {
    let raw: Vec<Line<'static>> = d
        .report_lines("session")
        .into_iter()
        .skip(2)
        .map(Line::from)
        .collect();
    wrap_panel_lines(&raw, inner_w)
}

pub fn wrap_line(line: &Line<'static>, width: u16) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthChar;
    let width = width as usize;
    if width == 0 {
        return vec![Line::from("")];
    }
    // Flatten spans to (char, style) cells, dropping zero-width chars.
    let mut cells: Vec<(char, ratatui::style::Style)> = Vec::new();
    for span in &line.spans {
        for c in span.content.chars() {
            cells.push((c, span.style));
        }
    }
    let cw = |c: char| UnicodeWidthChar::width(c).unwrap_or(0);
    let mut rows: Vec<Vec<(char, ratatui::style::Style)>> = Vec::new();
    let mut cur: Vec<(char, ratatui::style::Style)> = Vec::new();
    let mut cur_w = 0usize;
    let mut i = 0usize;
    while i < cells.len() {
        // Word = run of non-space chars; then any trailing spaces ride
        // with the NEXT word decision via the break rule below.
        let (c, st) = cells[i];
        if c == ' ' {
            if cur_w + 1 > width {
                // A space at the edge: break, and drop it.
                rows.push(std::mem::take(&mut cur));
                cur_w = 0;
            } else {
                cur.push((c, st));
                cur_w += 1;
            }
            i += 1;
            continue;
        }
        let w = cw(c);
        if cur_w + w > width {
            // Overflow: prefer breaking at the last space inside cur.
            if let Some(si) = cur.iter().rposition(|(ch, _)| *ch == ' ') {
                let rest = cur.split_off(si + 1);
                cur.pop(); // the break space itself
                rows.push(std::mem::take(&mut cur));
                cur = rest;
                cur_w = cur.iter().map(|(ch, _)| cw(*ch)).sum();
            } else {
                // No space: hard-break the row at the edge.
                rows.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            continue; // retry the same char on the fresh row
        }
        cur.push((c, st));
        cur_w += w;
        i += 1;
    }
    rows.push(cur); // the last row (possibly the only one, possibly empty)
    rows.into_iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            for (c, st) in row {
                if let Some(last) = spans.last_mut() {
                    let last: &mut Span<'static> = last;
                    if last.style == st {
                        last.content.to_mut().push(c);
                        continue;
                    }
                }
                spans.push(Span::styled(c.to_string(), st));
            }
            Line::from(spans)
        })
        .collect()
}

/// M23: visual rows one line occupies at `width` (the same wrapper
/// the viewport renders).
#[must_use]
pub fn wrapped_rows(line: &Line<'static>, width: u16) -> usize {
    wrap_line(line, width).len()
}

#[allow(clippy::struct_excessive_bools)] // UI surface state: panel visibility flags are the honest shape
#[derive(Debug, Clone)]
pub struct TuiState {
    /// Live loop phase for the rail.
    pub phase: LoopPhase,
    /// The composer editor (M2).
    pub editor: EditorState,
    /// Completed transcript lines (M3).
    pub transcript: Vec<Line<'static>>,
    /// Scrollback: Some(h) hides the bottom h visual ROWS (pinned);
    /// None is auto-follow. M23: rows, not lines - a wrapped line
    /// scrolls by the rows it occupies. New content while pinned adds
    /// its wrapped row count so the window holds stable.
    pub transcript_scroll: Option<usize>,
    /// M23: viewport width at the last render. Push-time scroll
    /// accounting wraps new lines at THIS width so a pinned window
    /// stays stable between frames. Set by `render_skeleton`.
    pub last_vp_width: std::cell::Cell<u16>,
    /// M23: per-line wrapped-row counts at the cached width, extended
    /// incrementally as lines land and rebuilt on a width change.
    /// Without it every frame would reflow the whole transcript -
    /// frame cost linear in session length.
    pub rows_cache: std::cell::RefCell<RowsCache>,
    /// Active picker overlay, if any (M4).
    pub picker: Option<PickerState>,
    /// Live command palette, open while the buffer starts with a sigil.
    pub palette: Option<PaletteState>,
    /// In-flight streaming answer text; committed per line (M5).
    pub answer_inflight: String,
    /// Delegation graph for this session (M6).
    pub agents: DelegationGraph,
    /// Whether the agents panel overlay is open (M6).
    pub agents_panel: bool,
    /// B8c: the selfmod lineage overlay is open.
    pub lineage_panel: bool,
    /// B8c: the scorer stream overlay is open.
    pub scorer_panel: bool,
    /// B8c: the evidence claims overlay is open.
    pub evidence_panel: bool,
    /// B8c: the `T_mission` decomposition overlay is open.
    pub time_panel: bool,
    /// B8c: the session layer injects the lineage view on toggle.
    pub lineage_view: Option<crate::tui_views::SelfmodView>,
    /// B8c: the session layer injects the scorer view on toggle.
    pub scorer_view: Option<crate::tui_views::ScorerView>,
    /// B8c: the session layer injects the evidence view on toggle.
    pub evidence_view: Option<crate::tui_views::EvidenceView>,
    /// B8c: the session layer injects the mission decomposition on toggle.
    pub time_view: Option<crate::mission_time::Decomposition>,
    /// Recent stream events, oldest first; the rail ticker shows the tail.
    pub ticker: VecDeque<EventKind>,
    /// Eric's five #1: goals submitted while a mission runs queue here
    /// FIFO instead of being dropped. Drained by the bin's event loop
    /// when the in-flight mission reports Done.
    pub queued_goals: VecDeque<String>,
    /// Surface theme (M9): every style on the surface derives from it;
    /// the bin fills it from `HS_THEME`.
    pub theme: crate::uipaint::Theme,
    /// HUD vitals.
    pub model_label: String,
    pub missions_run: u64,
    pub total_steps: u64,
    pub total_model_calls: u64,
    pub total_cost_micros: u64,
    pub stream_short: String,
}

impl Default for TuiState {
    fn default() -> Self {
        TuiState {
            phase: LoopPhase::Idle,
            editor: EditorState::default(),
            transcript: Vec::new(),
            transcript_scroll: None,
            last_vp_width: std::cell::Cell::new(80),
            rows_cache: std::cell::RefCell::new(RowsCache::default()),
            picker: None,
            palette: None,
            answer_inflight: String::new(),
            agents: DelegationGraph::new(),
            agents_panel: false,
            lineage_panel: false,
            scorer_panel: false,
            evidence_panel: false,
            time_panel: false,
            lineage_view: None,
            scorer_view: None,
            evidence_view: None,
            time_view: None,
            ticker: VecDeque::new(),
            queued_goals: VecDeque::new(),
            theme: crate::uipaint::Theme::dark(),
            model_label: "hs".to_string(),
            missions_run: 0,
            total_steps: 0,
            total_model_calls: 0,
            total_cost_micros: 0,
            stream_short: String::new(),
        }
    }
}

impl TuiState {
    /// Record one stream event on the ticker (oldest first, capped).
    pub fn push_stream_event(&mut self, k: EventKind) {
        self.ticker.push_back(k);
        while self.ticker.len() > TICKER_CAP {
            self.ticker.pop_front();
        }
    }

    /// M5: feed one mission UI event - phase, ticker, vitals, and tool
    /// beats all derive from the same stream the line-mode Painter
    /// consumes.
    /// M13: mission completion reconciliation. Model calls are counted
    /// LIVE by `ModelCallEnd` (M10); the done path must not add them
    /// again - pre-M13 the bin added `MissionResult.model_calls` on top,
    /// and the HUD ended every mission at 2x the real count (cap3:
    /// "done: 2 steps, 2 calls" vs HUD "4 calls", same screen). Steps
    /// have no live event, so they accrue here; cost takes the
    /// session-authoritative total.
    pub fn mission_done(&mut self, steps: u32, cost_total_micros: u64) {
        // M17: nothing is in flight once the mission lands - the rail
        // goes Idle instead of glowing a stale phase over the composer.
        self.phase = LoopPhase::Idle;
        self.missions_run += 1;
        self.total_steps += u64::from(steps);
        self.total_cost_micros = cost_total_micros;
    }

    /// M20: end-of-mission sequencing in one place. The held answer
    /// commits BEFORE the done line - the bin previously pushed the
    /// summary and only then flushed the tail, so the summary landed
    /// above the answer it summarized (live proof: tui-proof-m19).
    /// Keeps the M13 accounting (calls live-counted, cost is the
    /// session-authoritative total) and the done-line format.
    pub fn mission_done_report(
        &mut self,
        steps: u32,
        calls: u32,
        mission_cost_micros: u64,
        cost_total_micros: u64,
        outcome: &str,
    ) {
        self.flush_inflight();
        self.mission_done(steps, cost_total_micros);
        // M21: the line reports the mission's OWN spend; the HUD above
        // it keeps the session total (pre-M21 the line printed the
        // cumulative total, overstating every mission after the first).
        // M22: the outcome is always on the line, in the backfill's
        // exact format (M14) - pre-M22 only budget-killed surfaced, so
        // a steps_exhausted / ratchet_capped / harness_error mission
        // printed the same line as a verified one while a RESUMED
        // session showed the outcome. Live and resume now agree.
        let verdict = if outcome == "verified" {
            self.theme.ok.clone()
        } else {
            self.theme.fail.clone()
        };
        self.push_transcript_styled(
            &format!(
                "\u{2500}\u{2500} done: {} steps, {} calls, {} ({})",
                steps,
                calls,
                crate::uipaint::format_usd_micros(mission_cost_micros),
                outcome
            ),
            sgr_style(&verdict).add_modifier(Modifier::BOLD),
        );
    }

    pub fn on_ui_event(&mut self, ev: &crate::uipaint::UiEvent) {
        use crate::uipaint::UiEvent as U;
        match ev {
            U::ModelCallStart { model } => {
                // M19: reaching the next call means the held text was
                // prose (a rejected no-tool-call reply) - commit it.
                self.flush_inflight();
                // M17: a call issued right after an observation is the
                // operator REASONING OVER that result - Reflect. Any
                // other call start is the operator deciding - Plan.
                self.phase = if matches!(self.phase, LoopPhase::Observe) {
                    LoopPhase::Reflect
                } else {
                    LoopPhase::Plan
                };
                if !model.is_empty() {
                    self.model_label = model.clone();
                }
                self.push_ticker(EventKind::ModelCall);
            }
            U::ModelCallEnd {
                cost_usd_micros, ..
            } => {
                // M17: the END of a call is not a phase - the rail keeps
                // whatever the call start lit (Plan, or Reflect after an
                // observation). Pre-M17 this forced Reflect every time.
                self.total_model_calls += 1;
                // D12: book what the provider REPORTED, never a token-rate
                // estimate - the done/resume path reads the recorded
                // cost_usd_micros, so live and done tell one truth.
                self.total_cost_micros += (*cost_usd_micros).max(0) as u64;
                // M19: do NOT commit here - whether this call was prose
                // or wire JSON is only known when the NEXT event lands
                // (ToolCallStart drops it, anything else commits it).
            }
            U::ToolCallStart {
                plugin,
                args_summary,
            } => {
                // M19: the held text is this call's raw JSON envelope;
                // the beat below narrates it - never scrollback.
                self.answer_inflight.clear();
                self.phase = LoopPhase::Act;
                self.push_ticker(EventKind::ToolCall);
                let accent = sgr_style(&self.theme.accent);
                let tool = sgr_style(&self.theme.tool);
                let dim = sgr_style(&self.theme.dim);
                let mut spans = vec![
                    Span::styled("\u{25b6} ", accent),
                    Span::styled(plugin.clone(), tool),
                ];
                if !args_summary.is_empty() {
                    spans.push(Span::styled(format!("  {args_summary}"), dim));
                }
                self.push_transcript_spans(spans);
            }
            U::SubAgentSpawned {
                child,
                parent,
                mission,
                model,
            } => {
                self.agents.note_spawn(*child, *parent, mission, model);
                self.push_ticker(EventKind::Spawn);
            }
            U::SubAgentFinished { child, ok } => {
                self.agents.note_done(*child, *ok);
                self.push_ticker(EventKind::Consequence);
            }
            U::ToolCallEnd {
                ok,
                output_summary,
                elapsed_ms,
                ..
            } => {
                self.flush_inflight();
                self.phase = LoopPhase::Observe;
                self.push_ticker(EventKind::Observation);
                let (code, mark) = if *ok {
                    (sgr_style(&self.theme.ok), "\u{2713} ok")
                } else {
                    (sgr_style(&self.theme.fail), "\u{2717} fail")
                };
                let dim = sgr_style(&self.theme.dim);
                let mut spans = vec![
                    Span::raw("  "),
                    Span::styled(mark, code),
                    Span::styled(format!("  {elapsed_ms}ms"), dim),
                ];
                if !output_summary.is_empty() {
                    spans.push(Span::styled(format!("  {output_summary}"), dim));
                }
                self.push_transcript_spans(spans);
            }
        }
    }

    fn push_ticker(&mut self, k: EventKind) {
        self.ticker.push_back(k);
        while self.ticker.len() > TICKER_CAP {
            self.ticker.pop_front();
        }
    }

    /// M5: feed one streaming answer delta (the delta-sink channel).
    /// Complete lines commit to the transcript as markdown; the tail
    /// stays in flight until its newline arrives.
    pub fn on_answer_delta(&mut self, text: &str) {
        // M19: hold the whole call in flight. A completion that parses
        // as a tool call is pure wire JSON (lib.rs parses the ENTIRE
        // completion) - committing lines eagerly here is what leaked
        // raw tool-call JSON into scrollback (M17/M18 proof captures).
        // The live tail still renders from answer_inflight; commit or
        // drop happens when the call's disposition arrives.
        self.answer_inflight.push_str(text);
    }

    /// M19: commit the in-flight text as markdown (prose disposition).
    fn flush_inflight(&mut self) {
        if !self.answer_inflight.is_empty() {
            let tail = std::mem::take(&mut self.answer_inflight);
            let theme = self.theme.clone();
            self.push_transcript_markdown(&tail, &theme);
        }
    }

    /// M19: mission boundary - commit whatever prose remains held.
    /// The bin calls this when Done lands (budget-kill can end a
    /// mission with text still in flight).
    pub fn commit_answer_tail(&mut self) {
        self.flush_inflight();
    }

    /// M11: echo a submitted goal; multi-line goals keep their line
    /// breaks (one transcript line per goal line).
    /// Queue a goal submitted mid-mission; echoes its 1-based
    /// position so the operator sees the work was accepted. Returns
    /// the position.
    pub fn queue_goal(&mut self, goal: &str) -> usize {
        self.queued_goals.push_back(goal.to_string());
        let pos = self.queued_goals.len();
        self.push_transcript_line(&format!("queued #{pos} › {goal}"));
        pos
    }

    /// Pop the next queued goal (FIFO) for dispatch after the running
    /// mission reports Done.
    pub fn next_queued_goal(&mut self) -> Option<String> {
        self.queued_goals.pop_front()
    }

    /// How many goals wait behind the running mission.
    pub fn queued_count(&self) -> usize {
        self.queued_goals.len()
    }

    pub fn push_goal_echo(&mut self, text: &str) {
        let accent = sgr_style(&self.theme.accent);
        for (i, line) in text.lines().enumerate() {
            let l = if i == 0 {
                Line::from(vec![
                    Span::styled("\u{203a} ".to_string(), accent),
                    Span::raw(line.to_string()),
                ])
            } else {
                Line::from(line.to_string())
            };
            if let Some(h) = self.transcript_scroll.as_mut() {
                *h += wrapped_rows(&l, self.last_vp_width.get());
            }
            self.transcript.push(l);
        }
    }

    /// Append one plain transcript line (M3). M23: a pinned window
    /// moves by the line's wrapped ROWS at the last rendered width.
    pub fn push_transcript_line(&mut self, text: &str) {
        let line = Line::from(text.to_string());
        if let Some(h) = self.transcript_scroll.as_mut() {
            *h += wrapped_rows(&line, self.last_vp_width.get());
        }
        self.transcript.push(line);
    }

    /// Append one styled transcript line (beat 5 W1): the mission
    /// outcome carries its verdict in color + weight - pass and fail
    /// were visually identical dim text (live capture cap-06).
    pub fn push_transcript_styled(&mut self, text: &str, style: Style) {
        let line = Line::from(Span::styled(text.to_string(), style));
        if let Some(h) = self.transcript_scroll.as_mut() {
            *h += wrapped_rows(&line, self.last_vp_width.get());
        }
        self.transcript.push(line);
    }

    /// Append one transcript line built from styled spans (visual audit:
    /// the TUI speaks the same color language as the line-mode painter).
    pub fn push_transcript_spans(&mut self, spans: Vec<Span<'static>>) {
        let line = Line::from(spans);
        if let Some(h) = self.transcript_scroll.as_mut() {
            *h += wrapped_rows(&line, self.last_vp_width.get());
        }
        self.transcript.push(line);
    }

    /// Append markdown converted to styled lines (M3).
    pub fn push_transcript_markdown(&mut self, md: &str, theme: &crate::uipaint::Theme) {
        let lines = md_to_lines(md, theme);
        if let Some(h) = self.transcript_scroll.as_mut() {
            let w = self.last_vp_width.get();
            *h += lines.iter().map(|l| wrapped_rows(l, w)).sum::<usize>();
        }
        self.transcript.extend(lines);
    }

    /// Toggle the agents panel overlay (M6). Rendering decides whether
    /// a non-empty graph exists; an empty graph shows nothing.
    pub fn toggle_agents_panel(&mut self) {
        self.agents_panel = !self.agents_panel;
    }
    /// B8c: open/close the selfmod lineage overlay.
    pub fn toggle_lineage_panel(&mut self) {
        self.lineage_panel = !self.lineage_panel;
    }
    /// B8c: open/close the scorer stream overlay.
    pub fn toggle_scorer_panel(&mut self) {
        self.scorer_panel = !self.scorer_panel;
    }
    /// B8c: open/close the evidence claims overlay.
    pub fn toggle_evidence_panel(&mut self) {
        self.evidence_panel = !self.evidence_panel;
    }
    /// B8c: open/close the `T_mission` decomposition overlay.
    pub fn toggle_time_panel(&mut self) {
        self.time_panel = !self.time_panel;
    }

    /// Live palette sync: a leading sigil in the buffer opens it, the
    /// typed prefix filters, anything else closes it (Eric 2026-09-10).
    pub fn palette_sync(&mut self) {
        let text = self.editor.text();
        let t = text.trim_start();
        let Some(rest) = t.strip_prefix('/').or_else(|| t.strip_prefix(':')) else {
            self.palette = None;
            return;
        };
        if rest.contains(char::is_whitespace) {
            // accepted into the argument zone: the palette's job is done
            self.palette = None;
            return;
        }
        let matches = command_matches(rest);
        let selected = self.palette.as_ref().map_or(0, |p| {
            p.selected.min(matches.len().saturating_sub(1))
        });
        self.palette = Some(PaletteState { matches, selected });
    }

    /// Command names currently matching, for tests and rendering.
    pub fn palette_matches(&self) -> Option<Vec<&str>> {
        self.palette
            .as_ref()
            .map(|p| p.matches.iter().map(|c| c.name).collect())
    }

    pub fn palette_selected(&self) -> Option<usize> {
        self.palette.as_ref().map(|p| p.selected)
    }

    fn palette_down(&mut self) {
        if let Some(p) = &mut self.palette {
            if !p.matches.is_empty() {
                p.selected = (p.selected + 1).min(p.matches.len() - 1);
            }
        }
    }

    fn palette_up(&mut self) {
        if let Some(p) = &mut self.palette {
            p.selected = p.selected.saturating_sub(1);
        }
    }

    /// Tab acceptance: the highlighted command lands in the buffer in
    /// canonical spelling (arg-taking commands gain a trailing space).
    fn palette_complete(&mut self) {
        let Some(p) = &self.palette else { return };
        let Some(cmd) = p.matches.get(p.selected) else { return };
        let text = if cmd.args.is_some() {
            format!("/{} ", cmd.name)
        } else {
            format!("/{}", cmd.name)
        };
        self.editor.set_text(&text);
        self.palette_sync();
    }

    /// Open a picker overlay over the transcript (M4).
    pub fn open_picker(&mut self, entries: Vec<String>) {
        if entries.is_empty() {
            self.picker = None;
        } else {
            self.picker = Some(PickerState {
                entries,
                selected: 0,
                kind: PickerKind::Resume,
            });
        }
    }

    /// Selected entry index, None when no picker is open.
    pub fn picker_selected(&self) -> Option<usize> {
        self.picker.as_ref().map(|p| p.selected)
    }

    pub fn picker_down(&mut self) {
        if let Some(p) = self.picker.as_mut() {
            p.selected = (p.selected + 1).min(p.entries.len().saturating_sub(1));
        }
    }

    pub fn picker_up(&mut self) {
        if let Some(p) = self.picker.as_mut() {
            p.selected = p.selected.saturating_sub(1);
        }
    }

    /// Take the selected entry and close the overlay.
    pub fn picker_take(&mut self) -> Option<(PickerKind, String)> {
        let p = self.picker.take()?;
        p.entries.get(p.selected).cloned().map(|e| (p.kind, e))
    }

    /// Open the picker for a specific chooser (M29).
    pub fn open_picker_kind(&mut self, kind: PickerKind, entries: Vec<String>) {
        if entries.is_empty() {
            self.picker = None;
        } else {
            self.picker = Some(PickerState {
                entries,
                selected: 0,
                kind,
            });
        }
    }

    /// Live theme switch: recolors from the next frame and echoes the
    /// choice in the transcript.
    pub fn set_theme(&mut self, name: &str, theme: crate::uipaint::Theme) {
        self.theme = theme;
        self.push_transcript_line(&format!("theme › {name}"));
    }

    /// Close the overlay without a choice.
    pub fn picker_cancel(&mut self) {
        self.picker = None;
    }

    /// `PgUp`: scroll one viewport page (M4).
    pub fn transcript_page_up(&mut self, page: usize) {
        self.transcript_scroll_up(page);
    }

    /// Mouse wheel up: three lines at a time (M4).
    pub fn transcript_wheel_up(&mut self, n: usize) {
        self.transcript_scroll_up(n);
    }

    /// Mouse wheel down: toward the tail; reaching it re-engages follow.
    pub fn transcript_wheel_down(&mut self, n: usize) {
        if let Some(h) = self.transcript_scroll {
            let h = h.saturating_sub(n);
            self.transcript_scroll = if h == 0 { None } else { Some(h) };
        }
    }

    /// Pin the view n rows up from the current bottom (M23: rows).
    pub fn transcript_scroll_up(&mut self, n: usize) {
        let cur = self.transcript_scroll.unwrap_or(0);
        let w = self.last_vp_width.get();
        let total: usize = self.transcript.iter().map(|l| wrapped_rows(l, w)).sum();
        self.transcript_scroll = Some((cur + n).min(total.saturating_sub(1)));
    }

    /// Re-engage auto-follow.
    pub fn transcript_scroll_to_bottom(&mut self) {
        self.transcript_scroll = None;
    }

    /// M26: ":status" data - the same vitals the HUD paints, plus the
    /// stream, as one transcript line.
    pub fn status_line(&self) -> String {
        // The HUD line already carries the stream when set - compose,
        // don't repeat (first M26 capture read "... 79adb802 - stream
        // 79adb802").
        format!("status: {} \u{00b7} {}", self.model_label, self.hud_line())
    }

    /// M24: pub so the HUD text is test-pinnable (was private until
    /// the pluralization pin needed it).
    /// The HUD as styled spans: dim separators, metered cost in the
    /// theme's cost color (visual audit - the HUD was monochrome).
    #[must_use]
    pub fn hud_spans(&self) -> Line<'static> {
        let dim = sgr_style(&self.theme.dim);
        let cost = sgr_style(&self.theme.cost);
        let mut spans = vec![
            Span::raw(format!(
                "{} mission{}",
                self.missions_run,
                if self.missions_run == 1 { "" } else { "s" }
            )),
            Span::styled(" \u{00b7} ", dim),
            Span::raw(format!(
                "{} step{}",
                self.total_steps,
                if self.total_steps == 1 { "" } else { "s" }
            )),
            Span::styled(" \u{00b7} ", dim),
            Span::raw(format!("{} calls", self.total_model_calls)),
            Span::styled(" \u{00b7} ", dim),
            Span::styled(
                crate::uipaint::format_usd_micros(self.total_cost_micros),
                cost,
            ),
        ];
        if !self.stream_short.is_empty() {
            spans.push(Span::styled(" \u{00b7} ", dim));
            spans.push(Span::styled(self.stream_short.clone(), dim));
        }
        Line::from(spans)
    }

    pub fn hud_line(&self) -> String {
        format!(
            "{} mission{} \u{00b7} {} step{} \u{00b7} {} calls \u{00b7} {}{}",
            self.missions_run,
            if self.missions_run == 1 { "" } else { "s" },
            self.total_steps,
            if self.total_steps == 1 { "" } else { "s" },
            self.total_model_calls,
            crate::uipaint::format_usd_micros(self.total_cost_micros),
            if self.stream_short.is_empty() {
                String::new()
            } else {
                format!(" \u{00b7} {}", self.stream_short)
            },
        )
    }
}

/// M23: keep the wrapped-row cache in sync with the transcript at
/// `width` (rebuild on a width change, extend on append - transcript
/// lines are only ever pushed).
fn ensure_rows_cache(state: &TuiState, width: u16) {
    let mut cache = state.rows_cache.borrow_mut();
    if cache.width != width || cache.src_len > state.transcript.len() {
        cache.width = width;
        cache.lines = state
            .transcript
            .iter()
            .flat_map(|l| wrap_line(l, width))
            .collect();
    } else if cache.src_len < state.transcript.len() {
        for l in &state.transcript[cache.src_len..] {
            cache.lines.extend(wrap_line(l, width));
        }
    }
    cache.src_len = state.transcript.len();
}

/// Render the M1 skeleton: pinned composer box, loop rail with phases +
/// event ticker, HUD. The viewport is intentionally blank in M1.
pub fn render_skeleton(f: &mut Frame, state: &TuiState) {
    let area = f.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let l = layout(area.width, area.height);
    // Composer grows with the editor buffer; rail and viewport yield.
    // M25: the buffer WORD-WRAPS at the box inner width (same wrapper
    // as the transcript) - pre-M25 a goal longer than the box clipped
    // at the edge while being typed, cursor included. Box height is
    // the WRAPPED row count + borders.
    let inner_w = area.width.saturating_sub(2).max(1);
    let raw = format!("hs> {}", state.editor.text());
    let wrapped_rows: usize = raw
        .split('\n')
        .map(|l| wrap_line(&Line::from(l.to_string()), inner_w).len())
        .sum();
    let max_h = area.height.saturating_sub(4).max(3);
    let want_h = (wrapped_rows as u16 + 2).clamp(3, max_h);
    let composer = Rect::new(0, l.hud.y.saturating_sub(want_h), area.width, want_h);
    let rail = Rect::new(0, composer.y.saturating_sub(1), area.width, composer.y.min(1));
    let viewport = Rect::new(0, 0, area.width, rail.y);

    // M26: a fresh session is not a void - an empty transcript shows
    // a dim hint naming true next actions (pi/omp boot hints); it
    // vanishes the moment real content lands.
    if viewport.height > 0
        && viewport.width > 0
        && state.transcript.is_empty()
        && state.answer_inflight.is_empty()
    {
        let hint = "type a goal and press Enter \u{00b7} /help for commands \u{00b7} /resume to pick a session";
        let hy = viewport.y + viewport.height / 2;
        let hw = hint.chars().count() as u16;
        let hx = viewport.x + viewport.width.saturating_sub(hw) / 2;
        f.render_widget(
            Paragraph::new(hint).style(Style::default().add_modifier(Modifier::DIM)),
            Rect::new(hx, hy, hw.min(viewport.width), 1),
        );
    }

    // Transcript viewport: tail-follows, or pinned at the scrollback
    // window (M3). M23: content word-wraps at the viewport width
    // (nothing clips past the right edge); scroll state is in visual
    // ROWS and ratatui's own reflow does the wrapping, so what the
    // scroll math counts is exactly what the frame draws.
    if viewport.height > 0
        && viewport.width > 0
        && (!state.transcript.is_empty() || !state.answer_inflight.is_empty())
    {
        state.last_vp_width.set(viewport.width);
        let vis = viewport.height as usize;
        ensure_rows_cache(state, viewport.width);
        let following = state.transcript_scroll.is_none();
        let mut lines: Vec<Line> = state.rows_cache.borrow().lines.clone();
        // M11: the streaming tail renders live while tail-following,
        // wrapped like committed content (M23).
        if following && !state.answer_inflight.is_empty() {
            lines.extend(wrap_line(
                &Line::from(state.answer_inflight.clone()),
                viewport.width,
            ));
        }
        let total = lines.len();
        let hidden = state
            .transcript_scroll
            .unwrap_or(0)
            .min(total.saturating_sub(1));
        let scroll = total.saturating_sub(hidden + vis);
        f.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), viewport);
        // M15: a pinned viewport says so - bottom-right, dim, with the
        // count of rows hidden under the window (M23: rows, so the
        // count matches what the wrapped frame actually hides).
        // Without it a scrolled-up screen reads as a stale live view
        // (v3 cap4).
        if hidden > 0 {
            let marker = format!("\u{25bc} {hidden} below ");
            let mw = marker.chars().count() as u16;
            let mx = viewport.x + viewport.width.saturating_sub(mw);
            let my = viewport.y + viewport.height - 1;
            f.render_widget(
                Paragraph::new(marker).style(Style::default().add_modifier(Modifier::DIM)),
                Rect::new(mx, my, mw.min(viewport.width), 1),
            );
        }
    }

    // Loop rail: phases on the left (active accented), ticker on the
    // right (oldest to newest).
    if rail.height > 0 && rail.width > 0 {
        let accent = sgr_style(&state.theme.accent);
        let dim = Style::default().add_modifier(Modifier::DIM);
        let mut spans: Vec<Span> = Vec::new();
        for (i, ph) in LoopPhase::ALL.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" \u{203a} ", dim)); // ›
            }
            let style = if *ph == state.phase { accent } else { dim };
            spans.push(Span::styled(ph.label(), style));
        }
        // M27: phases get their full MEASURED width - pre-M27 a fixed
        // 3/5 split clipped "REFLECT" to "R" at 40 columns. The
        // ticker takes the remainder and yields entirely below a
        // readable minimum (4 cells); only a terminal narrower than
        // the rail itself clips phase names.
        let phases_w: u16 = (LoopPhase::ALL
            .iter()
            .map(|p| p.label().chars().count())
            .sum::<usize>()
            + 3 * (LoopPhase::ALL.len() - 1)) as u16;
        let left_w = phases_w.min(rail.width);
        let phases = Paragraph::new(Line::from(spans));
        f.render_widget(phases, Rect::new(rail.x, rail.y, left_w, 1));
        let right_w = rail.width - left_w;
        if right_w >= 4 {
            let ticker: String = state.ticker.iter().map(|k| kind_glyph(*k)).collect();
            let tick = Paragraph::new(ticker).alignment(ratatui::layout::Alignment::Right);
            f.render_widget(
                tick,
                Rect::new(rail.x + left_w.min(rail.width), rail.y, right_w, 1),
            );
        }
    }

    // Composer: rounded box, model+cost title, editor content, cursor
    // on the editor's cell. The box grows with the buffer (viewport
    // yields), capped so the chrome always fits.
    if composer.height > 0 && composer.width > 0 {
        let title = Line::from(vec![
            Span::styled(
                format!(" {} ", state.model_label),
                sgr_style(&state.theme.accent),
            ),
            Span::styled("\u{00b7} ", sgr_style(&state.theme.dim)),
            Span::styled(
                format!(
                    "{} ",
                    crate::uipaint::format_usd_micros(state.total_cost_micros)
                ),
                sgr_style(&state.theme.cost),
            ),
        ]);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(sgr_style(&state.theme.dim))
            .title(title);
        let mut content: Vec<Line> = Vec::new();
        for l in raw.split('\n') {
            content.extend(wrap_line(&Line::from(l.to_string()), inner_w));
        }
        let input = Paragraph::new(content).block(block);
        f.render_widget(input, composer);
        if composer.height >= 3 && composer.width > 6 {
            let (row, col) = state.editor.cursor();
            // M25: wrap the buffer up to (row, col) the same way the
            // content wrapped; the cursor lands on the last wrapped
            // row's tail, not on the old unwrapped (row, col) cell.
            let mut before = String::from("hs> ");
            for (i, l) in state.editor.text().split('\n').enumerate() {
                if i < row {
                    before.push_str(l);
                    before.push('\n');
                } else {
                    before.extend(l.chars().take(col));
                    break;
                }
            }
            let mut bl: Vec<Line> = Vec::new();
            for l in before.split('\n') {
                bl.extend(wrap_line(&Line::from(l.to_string()), inner_w));
            }
            let cy = composer.y + 1 + bl.len().saturating_sub(1) as u16;
            let cx = composer.x + 1 + bl.last().map_or(0, |l| l.width() as u16);
            if cx < composer.x + composer.width - 1 && cy < composer.y + composer.height - 1 {
                f.set_cursor_position((cx, cy));
            }
        }
    }

    // HUD: session vitals on the last row.
    if l.hud.height > 0 && l.hud.width > 0 {
        let hud = Paragraph::new(state.hud_spans());
        f.render_widget(hud, l.hud);
    }

    // M32: the live command palette - anchored above the composer,
    // capped at 8 rows so short terminals keep the transcript.
    if let Some(p) = &state.palette {
        let rows = p.matches.len().clamp(1, 8) as u16;
        let ph = rows + 2;
        if composer.y > ph + 1 {
            let w = area.width.min(64);
            let rect = Rect::new(composer.x, composer.y - ph, w, ph);
            f.render_widget(ratatui::widgets::Clear, rect);
            let mut lines: Vec<Line> = Vec::new();
            if p.matches.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  no matching commands",
                    Style::default().add_modifier(Modifier::DIM),
                )));
            } else {
                // Scroll window: the highlight is always visible when
                // the registry outgrows the 8-row cap.
                let start = p.selected.saturating_sub(7);
                for (i, cmd) in p.matches.iter().enumerate().skip(start).take(8) {
                    let selected = i == p.selected;
                    let row_style = if selected {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    let arg = cmd.args.map(|a| format!(" {a}")).unwrap_or_default();
                    let marker = if selected { "\u{25b6}" } else { " " };
                    lines.push(Line::from(vec![
                        Span::styled(format!("{marker} /{}{arg}", cmd.name), row_style),
                        Span::styled(
                            format!("  {}", cmd.summary),
                            Style::default().add_modifier(Modifier::DIM),
                        ),
                    ]));
                }
            }
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(sgr_style(&state.theme.dim))
                .title(Span::styled(" commands ", sgr_style(&state.theme.accent)));
            f.render_widget(Paragraph::new(lines).block(block), rect);
        }
    }

    // M4: the picker overlay - centered box over the transcript,
    // selected entry highlighted with the theme accent.
    if let Some(p) = &state.picker {
        let box_w = (area.width * 3 / 4).max(20).min(area.width);
        let box_h = (p.entries.len() as u16 + 2).min(viewport.height.max(3));
        let bx = (area.width - box_w) / 2;
        let by = viewport.y + (viewport.height.saturating_sub(box_h)) / 2;
        let rect = Rect::new(bx, by, box_w, box_h);
        f.render_widget(ratatui::widgets::Clear, rect);
        let accent = sgr_style(&state.theme.accent);
        let lines: Vec<Line> = p
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                if i == p.selected {
                    Line::from(Span::styled(format!("\u{25b6} {e}"), accent))
                } else {
                    Line::from(format!("  {e}"))
                }
            })
            .collect();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(sgr_style(&state.theme.dim))
            .title(Span::styled(
                match p.kind {
                    PickerKind::Resume => " resume ",
                    PickerKind::Models => " models ",
                    PickerKind::Themes => " theme ",
                },
                sgr_style(&state.theme.accent),
            ));
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }

    // M6: the agents panel - delegation graph as a right-docked
    // overlay, present only when open AND the graph is non-empty (zero
    // chrome for the single-agent case).
    if state.agents_panel && !state.agents.nodes().is_empty() {
        let nodes = state.agents.nodes();
        let box_w = (area.width * 2 / 3).max(30).min(area.width);
        let box_h = (nodes.len() as u16 + 2).min(viewport.height.max(3));
        let rect = Rect::new(area.width - box_w, viewport.y, box_w, box_h);
        f.render_widget(ratatui::widgets::Clear, rect);
        // Indented tree: parents before children, depth from the
        // parent chain, model on every node.
        let mut order: Vec<&AgentNode> = Vec::with_capacity(nodes.len());
        let mut stack: Vec<&AgentNode> = nodes
            .iter()
            .filter(|n| n.parent.is_none() || !nodes.iter().any(|m| Some(m.stream_id) == n.parent))
            .collect();
        stack.reverse();
        let mut visited = std::collections::HashSet::new();
        while let Some(n) = stack.pop() {
            if !visited.insert(n.stream_id) {
                continue;
            }
            order.push(n);
            let mut kids: Vec<&AgentNode> = nodes
                .iter()
                .filter(|k| k.parent == Some(n.stream_id))
                .collect();
            kids.reverse();
            for k in kids {
                stack.push(k);
            }
        }
        let lines: Vec<Line> = order
            .iter()
            .map(|n| {
                let (glyph, style) = match n.status {
                    AgentStatus::Running => ("\u{25b6}", sgr_style(&state.theme.accent)),
                    AgentStatus::Done => ("\u{2713}", Style::default().fg(Color::Green)),
                    AgentStatus::Failed => ("\u{2717}", Style::default().fg(Color::Red)),
                };
                let short: String = n.stream_id.to_string().chars().take(8).collect();
                let indent = "  ".repeat(state.agents.depth(n));
                Line::from(vec![
                    Span::raw(indent),
                    Span::styled(format!("{glyph} "), style),
                    Span::styled(short, Style::default().add_modifier(Modifier::DIM)),
                    Span::raw(format!("  {}", n.mission)),
                    Span::styled(
                        format!("  \u{00b7} {}", n.model),
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                ])
            })
            .collect();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" agents ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }

    // B8c: the lineage overlay - the selfmod stream as booked
    // mutations and deltas; an absent or empty stream renders one
    // honest line, never a blank box.
    if state.lineage_panel {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let lines: Vec<Line> = match &state.lineage_view {
            None => vec![Line::styled("no selfmod cycle this session", dim)],
            Some(v) => {
                let mut ls: Vec<Line> = Vec::new();
                for m in &v.mutations {
                    let short: String = m.fork.to_string().chars().take(8).collect();
                    ls.push(Line::from(format!(
                        "\u{270e} {short}  {} changes",
                        m.changes
                    )));
                }
                for c in &v.capability_deltas {
                    ls.push(Line::from(format!(
                        "\u{25b2} {} {}  prompts={} tools={}",
                        c.candidate,
                        if c.promoted { "promoted" } else { "demoted" },
                        c.prompts,
                        c.tools
                    )));
                }
                for fd in &v.fitness_deltas {
                    ls.push(Line::from(format!(
                        "\u{25c6} {}  held-out={:.1}%",
                        fd.candidate,
                        fd.held_out_pass_rate * 100.0
                    )));
                }
                if ls.is_empty() {
                    ls.push(Line::styled("no selfmod cycle this session", dim));
                }
                ls
            }
        };
        let box_w = (area.width * 2 / 3).max(40).min(area.width);
        let lines = wrap_panel_lines(&lines, box_w.saturating_sub(2));
        let box_h = (lines.len() as u16 + 2).min(viewport.height.max(3));
        let rect = Rect::new(area.width - box_w, viewport.y, box_w, box_h);
        f.render_widget(ratatui::widgets::Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" lineage ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }

    // B8c: the scorer overlay - pins, tier scores, canary results.
    if state.scorer_panel {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let lines: Vec<Line> = match &state.scorer_view {
            None => vec![Line::styled("no scorer stream this session", dim)],
            Some(v) => {
                let mut ls: Vec<Line> = Vec::new();
                for p in &v.pins {
                    let h8: String = p.hash.chars().take(8).collect();
                    ls.push(Line::from(format!(
                        "\u{2691} {} \u{00b7} {} \u{00b7} {h8}",
                        p.version, p.conditions
                    )));
                }
                for sc in &v.scores {
                    ls.push(Line::from(format!(
                        "{} cand={} suite={} {} {}/{}",
                        sc.tier,
                        sc.candidate,
                        sc.suite,
                        if sc.passed { "PASS" } else { "FAIL" },
                        sc.correct,
                        sc.total
                    )));
                }
                for cg in &v.canaries {
                    let flag = if cg.error { " ERROR" } else { "" };
                    ls.push(Line::from(format!(
                        "\u{25c8} {} gt={} scorer={}{flag}",
                        cg.id,
                        if cg.ground_truth_good { "good" } else { "bad" },
                        if cg.scorer_said_good { "good" } else { "bad" },
                    )));
                }
                if ls.is_empty() {
                    ls.push(Line::styled("no scorer stream this session", dim));
                }
                ls
            }
        };
        let box_w = (area.width * 2 / 3).max(40).min(area.width);
        let lines = wrap_panel_lines(&lines, box_w.saturating_sub(2));
        let box_h = (lines.len() as u16 + 2).min(viewport.height.max(3));
        let rect = Rect::new(area.width - box_w, viewport.y, box_w, box_h);
        f.render_widget(ratatui::widgets::Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" scorer ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }

    // B8c: the evidence overlay - the GATE 9c claim record plus the
    // unfoldable drift regressions, from the registered scorer stream.
    if state.evidence_panel {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let lines: Vec<Line> = match &state.evidence_view {
            None => vec![Line::styled("no evidence claims this session", dim)],
            Some(v) => {
                let mut ls: Vec<Line> = Vec::new();
                for c in &v.claims {
                    use crate::tui_views::{EvidenceClaimKind as K, EvidenceClaimStatus as S};
                    let refs = |u: Option<uuid::Uuid>| {
                        u.map(|u| {
                            let s = u.to_string();
                            s.chars().take(8).collect::<String>()
                        })
                        .unwrap_or_else(|| "-".to_string())
                    };
                    let line = match c.kind {
                        K::Verified => {
                            format!("\u{2713} {} verified @{}", c.subject, refs(c.verified_at))
                        }
                        K::OpenFailure => format!("\u{2717} {} open failure", c.subject),
                        K::Regression => format!(
                            "\u{26a0} {} REGRESSED {} \u{2192} {}{}",
                            c.subject,
                            refs(c.verified_at),
                            refs(c.regressed_at),
                            if c.status == S::Superseded {
                                " superseded"
                            } else {
                                ""
                            },
                        ),
                    };
                    ls.push(Line::from(line));
                }
                if !v.unresolved_regressions.is_empty() {
                    ls.push(Line::styled(
                        "drift regressions (not in the claim record):",
                        dim,
                    ));
                    for r in &v.unresolved_regressions {
                        ls.push(Line::from(format!("  \u{25e6} {r}")));
                    }
                }
                if ls.is_empty() {
                    ls.push(Line::styled("no evidence claims this session", dim));
                }
                ls
            }
        };
        let box_w = (area.width * 2 / 3).max(40).min(area.width);
        let lines = wrap_panel_lines(&lines, box_w.saturating_sub(2));
        let box_h = (lines.len() as u16 + 2).min(viewport.height.max(3));
        let rect = Rect::new(area.width - box_w, viewport.y, box_w, box_h);
        f.render_widget(ratatui::widgets::Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" evidence ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }

    // B8c: the T_mission overlay - the session stream's measured
    // decomposition (the published report lines, header rows dropped).
    if state.time_panel {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let inner = panel_inner_width(area.width);
        let lines: Vec<Line> = match &state.time_view {
            None => vec![Line::styled("no mission decomposition yet", dim)],
            Some(d) => time_panel_lines(d, inner),
        };
        let box_w = (inner + 2).min(area.width);
        let box_h = (lines.len() as u16 + 2).min(viewport.height.max(3));
        let rect = Rect::new(area.width - box_w, viewport.y, box_w, box_h);
        f.render_widget(ratatui::widgets::Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(" T_mission ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }
}
