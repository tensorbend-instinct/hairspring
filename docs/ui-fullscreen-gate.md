# UI gap #10 evaluation: ratatui-class full-screen TUI

Eric 2026-09-08 13:12 steering: the #10 deferral is REVERSED, full-screen is on
the table, bar = pi/omp-level polish, deliverable = an honest effort read.

## What we have today (post-#9)

Line-based REPL, 1,626 LOC in the UI layer (repl.rs 853 / uipaint.rs 557 /
bin 216). Streaming deltas paint inline as the transcript scrolls; the
composer frame is re-printed per entry; status bar re-prints under the
transcript. Input goes through an `Editor` trait with two impls
(rustyline TTY, plain piped stdin), so mission dispatch is already
decoupled from input mechanics. Theme is a first-class struct driving
every painted surface (dark/light/TOML, HS_THEME).

Structural limit, stated plainly: in a line-based terminal the transcript
owns the scroll region. Nothing can stay pinned. The composer box, the
status bar, a spinner - all of them scroll away. pi and omp solve this by
taking the whole screen (alternate-screen TUI): the editor box never
moves, the transcript is a scrollable viewport, chrome repaints in place.

## What pi/omp actually do

- Alternate screen; layout = transcript viewport (top, grows) + pinned
  multi-line editor box (bottom) + status line.
- Editor box has real borders that persist; typing never scrolls it.
- Viewport: mouse-wheel + PgUp/PgDn scrollback, auto-follow at bottom.
- Markdown renders into the viewport as styled blocks; resize re-wraps.
- Pickers (resume, model, theme) are overlays, not inline dumps.
- Themes as files (matched by gap #9).

## What full-screen buys hs-repl

1. Persistent editor box (the #8 compromise - per-entry re-print - goes
   away; the box is just there, like pi/omp).
2. In-place status bar / spinner, no re-print lines in the transcript.
3. Real scrollback widget (wheel/PgUp) independent of terminal scrollback.
4. Markdown re-layout on resize; no mid-stream wrapping artifacts.
5. Overlays for the resume picker and future pickers.

## What it costs

1. Native terminal scrollback + selection inside the alt screen is gone;
   the viewport must reimplement scroll (pi/omp accept this; users copy
   via the viewport or the answer files).
2. Two UI paths to maintain: full-screen (TTY) + line mode (piped,
   dumb terminals). Line mode stays - it is the automation contract and
   the test seam.
3. New deps: ratatui + crossterm (both pure-Rust, no native libs).

## Effort read (honest)

| component | LOC (new) | risk | notes |
|---|---|---|---|
| terminal bootstrap/teardown, panic-restore | ~100 | low | raw mode + alt screen + drop guard |
| event loop: crossterm events + kernel UiEvent channel + tick | ~200 | med | replaces read_line blocking; kernel events already arrive via sink callbacks |
| editor widget (multi-line, cursor, history, :completion) | ~150 w/ tui-textarea, 400+ hand-rolled | med | tui-textarea is the sane choice |
| transcript viewport: styled buffer, auto-follow + manual scroll | ~250 | med-high | MarkdownStreamer must emit ratatui Text spans instead of ANSI strings (adapter, theme already abstracted) |
| status bar + composer frame as widgets | ~100 | low | Theme ports directly |
| resume picker as overlay list | ~80 | low | |
| resize + mouse | ~50 | low | |
| piped/line-mode fallback glue | ~100 | low | Editor trait seam already exists |
| **total** | **~1,000-1,250 + tests** | | |

Testability is good: ratatui's TestBackend renders to an in-memory
buffer, so RED/GREEN stays exact (assert cell contents/styles, no
screenshots needed for unit tests; tmux stays the live-proof rig).

Estimate: 1.5-2.5 days at today's velocity (each line-mode gap closed in
under an hour; this is roughly ten gaps' worth with two medium-risk
components). The single biggest risk is the streaming-markdown-to-
ratatui-Text adapter; the second is keeping line mode byte-identical for
the existing 564-test suite.

## Recommendation

BUILD. The Editor-trait seam and the Theme abstraction mean this is an
additive frontend, not a rewrite: line mode stays for piped/automation,
full-screen takes over on a TTY. Milestones, each TDD + pushed:

- M1: bootstrap + layout skeleton (viewport/editor/status regions render
  in TestBackend; alt-screen enter/exit clean on :quit and on panic).
- M2: editor widget + input routing (:commands, history, Tab completion).
- M3: transcript viewport + streaming markdown adapter + scroll.
- M4: picker overlays, mouse, resize, polish pass vs pi/omp.

## ADDENDUM (Eric 2026-09-08 14:53, via Main): the loop substrate is the
## differentiator - it must be VISIBLE

Eric: loop/graph mechanisms are still missing from the surface. Phase
transitions, the event stream, and the delegation graph are first-class
design elements alongside frame/composer/scrollback, not afterthoughts.

What the substrate already emits (hs-core EventKind, 18 kinds):
ModelCall, ToolCall, Observation, Decision, ContextInject, Feedback,
SnapshotRef, Proposal, Consequence, GoalUpdate, BudgetUpdate, Spawn,
Message, Mutation, Score, ScorerPin, CanaryResult, Prefetch - plus the
gates 7-8 CapabilityDelta/FitnessDelta/Regression/CapabilityChange
space. hs-swarm carries the delegation structure. Everything the UI
needs is already on the stream; today none of it is rendered.

Design - one surface, four regions:

    +------------------------------------------------------+
    | transcript viewport (markdown, tool cards, scroll)   |
    +------------------------------------------------------+
    | loop rail: [PHASE] ticker of recent stream events    |
    +------------------------------------------------------+
    | /- model . $cost ----------------\                   |
    | | hs> composer (pinned)          |                   |
    | \--------------------------------/                   |
    | HUD: missions . steps . calls . $ . time . stream-id |
    +------------------------------------------------------+

1. LOOP-PHASE INDICATOR (loop rail, left): where in the cycle the agent
   is RIGHT NOW, derived from the latest stream events:
   PLAN (ModelCall in flight) -> ACT (ToolCall running) ->
   OBSERVE (Observation landed) -> REFLECT (Decision/Feedback) ->
   back to PLAN. Named phases with a spinner on the active one; theme
   accent for the live phase, dim for the rest.

2. EVENT-STREAM VISUALIZATION (loop rail, right): a ticker of the last
   N stream events, one glyph + theme color per EventKind, newest
   rightmost, with the live one pulsing. `:events` expands it into a
   scrollable side panel: time, kind, one-line summary per event - the
   stream made inspectable without leaving the surface.

3. DELEGATION GRAPH (overlay/side panel, appears when swarm is active):
   built from Spawn (edge parent->child) and Message (edge activity)
   events. Nodes: short agent id, model, live status (running/done/
   failed from Consequence). Rendered as an indented tree in a panel;
   `:agents` toggles it. When no swarm has spawned, the panel is absent
   - zero chrome for the single-agent case.

Milestone plan absorbs these: M1 layout skeleton now renders FOUR
regions (viewport, loop rail, composer, HUD) in TestBackend; M2 editor
+ input; M3 viewport + streaming markdown; M4 pickers/mouse/resize;
M5 loop rail live (phase derivation + ticker, unit-tested against
synthetic event sequences); M6 delegation graph panel.
