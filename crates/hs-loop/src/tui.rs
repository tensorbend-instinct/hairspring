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

/// Cap on ticker entries kept; the rail shows the tail.
const TICKER_CAP: usize = 32;

/// Everything the surface needs to render one frame. M1 keeps this
/// small; later milestones grow it (transcript lines, editor buffer,
/// delegation graph).
#[derive(Debug, Clone)]
pub struct TuiState {
    /// Live loop phase for the rail.
    pub phase: LoopPhase,
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

    // Loop rail: phases on the left (active accented), ticker on the
    // right (oldest to newest).
    if l.rail.height > 0 && l.rail.width > 0 {
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
        let left_w = l.rail.width * 3 / 5;
        let phases = Paragraph::new(Line::from(spans));
        f.render_widget(
            phases,
            Rect::new(l.rail.x, l.rail.y, left_w.min(l.rail.width), 1),
        );
        let right_w = l.rail.width - left_w.min(l.rail.width);
        if right_w > 0 {
            let ticker: String = state.ticker.iter().map(|k| kind_glyph(*k)).collect();
            let tick = Paragraph::new(ticker).alignment(ratatui::layout::Alignment::Right);
            f.render_widget(
                tick,
                Rect::new(l.rail.x + left_w.min(l.rail.width), l.rail.y, right_w, 1),
            );
        }
    }

    // Composer: rounded box, model+cost title, hs> input line.
    if l.composer.height > 0 && l.composer.width > 0 {
        let title = format!(
            " {} \u{00b7} {} ",
            state.model_label,
            crate::uipaint::format_usd_micros(state.total_cost_micros)
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(title);
        let input = Paragraph::new("hs> ").block(block);
        f.render_widget(input, l.composer);
    }

    // HUD: session vitals on the last row.
    if l.hud.height > 0 && l.hud.width > 0 {
        let hud = Paragraph::new(state.hud_line());
        f.render_widget(hud, l.hud);
    }
}
