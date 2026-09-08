//! UI gap #10: the ratatui-class full-screen surface (Eric 2026-09-08
//! via Main: full-screen is THE BUILD; loop substrate - phase indicator,
//! event-stream ticker, delegation graph - is first-class).
//!
//! Four regions: transcript viewport (grows), one-row loop rail, pinned
//! composer box, one-row HUD. M1 ships the layout + skeleton rendering
//! against ratatui's TestBackend; line mode stays the piped fallback.

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
    /// Model call in flight (or idle before the first one).
    #[default]
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
            LoopPhase::Plan => "PLAN",
            LoopPhase::Act => "ACT",
            LoopPhase::Observe => "OBSERVE",
            LoopPhase::Reflect => "REFLECT",
        }
    }
}

/// One glyph per stream event kind for the loop-rail ticker.
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
        EventKind::Prefetch => '\u{21bb}',        // ↻
        EventKind::CapabilityDelta => '\u{0394}', // Δ
        EventKind::FitnessDelta => '\u{2206}',    // ∆
        EventKind::Regression => '\u{26a0}',      // ⚠
        EventKind::CapabilityChange => '\u{21c4}',// ⇄
    }
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
            "37" => style.fg(Color::White),
            "90" => style.fg(Color::DarkGray),
            "91" => style.fg(Color::LightRed),
            "92" => style.fg(Color::LightGreen),
            "93" => style.fg(Color::LightYellow),
            "94" => style.fg(Color::LightBlue),
            "95" => style.fg(Color::LightMagenta),
            "96" => style.fg(Color::LightCyan),
            "97" => style.fg(Color::White),
            _ => style,
        };
    }
    style
}

/// M3: convert markdown text into styled ratatui Lines - the same
/// rules the line-mode MarkdownStreamer paints (headers, bullets,
/// fences, inline code, bold), driven by the Theme. Fence bodies pass
/// through verbatim, dimmed in the code color.
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
        if hashes >= 2 && trimmed.chars().nth(hashes) == Some(' ') {
            let text = trimmed[hashes + 1..].trim_end();
            out.push(Line::from(inline_spans(text, sgr_style(&theme.header), theme)));
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
            if let Some(close) = body.find('`') {
                if close > 0 {
                    spans.push(Span::styled(body[..close].to_string(), code_style));
                    rest = &body[close + 1..];
                    continue;
                }
            }
            spans.push(Span::styled("`".to_string(), base));
            rest = body;
        } else if let Some(body) = after.strip_prefix("**") {
            if let Some(close) = body.find("**") {
                if close > 0 {
                    spans.push(Span::styled(body[..close].to_string(), bold));
                    rest = &body[close + 2..];
                    continue;
                }
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
    /// Whole buffer, lines joined by newlines.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// (row, col) in chars.
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map(|l| l.chars().count()).unwrap_or(0)
    }

    fn set_text(&mut self, text: &str) {
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
        let byte = line.char_indices().nth(self.col).map(|(b, _)| b).unwrap_or(line.len());
        line.insert(byte, c);
        self.col += 1;
    }

    /// Delete the char left of the cursor; at column 0 join with the
    /// line above.
    pub fn backspace(&mut self) {
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            let byte = line.char_indices().nth(self.col - 1).map(|(b, _)| b).unwrap_or(0);
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
        let byte = line.char_indices().nth(self.col).map(|(b, _)| b).unwrap_or(line.len());
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
    pub fn line_count(&self) -> usize {
        self.lines.len().max(1)
    }
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
}

/// Cap on ticker entries kept; the rail shows the tail.
const TICKER_CAP: usize = 32;

/// Everything the surface needs to render one frame. M1 keeps this
/// small; later milestones grow it (transcript lines, editor buffer,
/// delegation graph).
#[derive(Debug, Clone)]
pub struct TuiState {
    /// Live loop phase for the rail.
    pub phase: LoopPhase,
    /// The composer editor (M2).
    pub editor: EditorState,
    /// Completed transcript lines (M3).
    pub transcript: Vec<Line<'static>>,
    /// Scrollback: Some(h) hides the bottom h lines (pinned); None is
    /// auto-follow. New lines while pinned increment h so the window
    /// holds the same absolute lines.
    pub transcript_scroll: Option<usize>,
    /// Active picker overlay, if any (M4).
    pub picker: Option<PickerState>,
    /// Recent stream events, oldest first; the rail ticker shows the tail.
    pub ticker: VecDeque<EventKind>,
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
            phase: LoopPhase::Plan,
            editor: EditorState::default(),
            transcript: Vec::new(),
            transcript_scroll: None,
            picker: None,
            ticker: VecDeque::new(),
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

    /// Append one plain transcript line (M3).
    pub fn push_transcript_line(&mut self, text: &str) {
        if let Some(h) = self.transcript_scroll.as_mut() {
            *h += 1;
        }
        self.transcript.push(Line::from(text.to_string()));
    }

    /// Append markdown converted to styled lines (M3).
    pub fn push_transcript_markdown(&mut self, md: &str, theme: &crate::uipaint::Theme) {
        let lines = md_to_lines(md, theme);
        if let Some(h) = self.transcript_scroll.as_mut() {
            *h += lines.len();
        }
        self.transcript.extend(lines);
    }

    /// Open a picker overlay over the transcript (M4).
    pub fn open_picker(&mut self, entries: Vec<String>) {
        if entries.is_empty() {
            self.picker = None;
        } else {
            self.picker = Some(PickerState { entries, selected: 0 });
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
    pub fn picker_take(&mut self) -> Option<String> {
        let p = self.picker.take()?;
        p.entries.get(p.selected).cloned()
    }

    /// Close the overlay without a choice.
    pub fn picker_cancel(&mut self) {
        self.picker = None;
    }

    /// PgUp: scroll one viewport page (M4).
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

    /// Pin the view n lines up from the current bottom.
    pub fn transcript_scroll_up(&mut self, n: usize) {
        let cur = self.transcript_scroll.unwrap_or(0);
        self.transcript_scroll = Some((cur + n).min(self.transcript.len().saturating_sub(1)));
    }

    /// Re-engage auto-follow.
    pub fn transcript_scroll_to_bottom(&mut self) {
        self.transcript_scroll = None;
    }

    fn hud_line(&self) -> String {
        format!(
            "{} mission{} \u{00b7} {} steps \u{00b7} {} calls \u{00b7} {}{}",
            self.missions_run,
            if self.missions_run == 1 { "" } else { "s" },
            self.total_steps,
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

/// Render the M1 skeleton: pinned composer box, loop rail with phases +
/// event ticker, HUD. The viewport is intentionally blank in M1.
pub fn render_skeleton(f: &mut Frame, state: &TuiState) {
    let area = f.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let l = layout(area.width, area.height);
    // Composer grows with the editor buffer; rail and viewport yield.
    let max_h = area.height.saturating_sub(4).max(3);
    let want_h = (state.editor.line_count() as u16 + 2).clamp(3, max_h);
    let composer = Rect::new(0, l.hud.y.saturating_sub(want_h), area.width, want_h);
    let rail = Rect::new(0, composer.y.saturating_sub(1), area.width, composer.y.min(1));
    let viewport = Rect::new(0, 0, area.width, rail.y);

    // Transcript viewport: tail-follows, or pinned at the scrollback
    // window (M3).
    if viewport.height > 0 && viewport.width > 0 && !state.transcript.is_empty() {
        let vis = viewport.height as usize;
        let hidden = state.transcript_scroll.unwrap_or(0).min(state.transcript.len().saturating_sub(1));
        let end = state.transcript.len() - hidden;
        let start = end.saturating_sub(vis);
        let window: Vec<Line> = state.transcript[start..end].to_vec();
        f.render_widget(Paragraph::new(window), viewport);
    }

    // Loop rail: phases on the left (active accented), ticker on the
    // right (oldest to newest).
    if rail.height > 0 && rail.width > 0 {
        let accent = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().add_modifier(Modifier::DIM);
        let mut spans: Vec<Span> = Vec::new();
        for (i, ph) in LoopPhase::ALL.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" \u{203a} ", dim)); // ›
            }
            let style = if *ph == state.phase { accent } else { dim };
            spans.push(Span::styled(ph.label(), style));
        }
        let left_w = rail.width * 3 / 5;
        let phases = Paragraph::new(Line::from(spans));
        f.render_widget(phases, Rect::new(rail.x, rail.y, left_w.min(rail.width), 1));
        let right_w = rail.width - left_w.min(rail.width);
        if right_w > 0 {
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
        let title = format!(
            " {} \u{00b7} {} ",
            state.model_label,
            crate::uipaint::format_usd_micros(state.total_cost_micros)
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title);
        let text = state.editor.text();
        let mut content = String::from("hs> ");
        content.push_str(&text);
        let input = Paragraph::new(content).block(block);
        f.render_widget(input, composer);
        if composer.height >= 3 && composer.width > 6 {
            let (row, col) = state.editor.cursor();
            let cx = composer.x + 1 + 4 + col as u16;
            let cy = composer.y + 1 + row as u16;
            if cx < composer.x + composer.width - 1 && cy < composer.y + composer.height - 1 {
                f.set_cursor_position((cx, cy));
            }
        }
    }

    // HUD: session vitals on the last row.
    if l.hud.height > 0 && l.hud.width > 0 {
        let hud = Paragraph::new(state.hud_line());
        f.render_widget(hud, l.hud);
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
        let accent = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
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
            .title(" resume ");
        f.render_widget(Paragraph::new(lines).block(block), rect);
    }
}
