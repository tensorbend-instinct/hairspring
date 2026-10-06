# HAIRSPRING desktop app (Tauri v2)

apps/desktop. The Rust backend links `hs_loop::engine::Engine` in-process: missions run inside the
app, UI events stream to the webview as JSON (`hs-event`, `hs-done`, `hs-queue`, `hs-scheduled`).
Nothing shells out to the CLI.

## What the app does
- First run: setup screen saves a provider key owner-only under ~/.config/hairspring/keys/.
- Chat: user bubbles, folded reasoning, tool rows "Completed in Ns", live footer (elapsed, steps, calls, tokens, cache hit).
- Sidebar: sessions grouped by workspace, title from the goal, age. Double-click exports a session ZIP.
- Trajectory tab: Duration / Model / Tools bars and Turns / Calls / Events / Cost for the selected session.
- Slash menu as you type: /goal /plan /feedback /compact /permission /model /export /add-file /mode /queue.
- Queue-while-busy: a goal submitted during a mission queues and drains in order.
- Question card: `ask_user_question` calls show options in the app and the click answers the tool.
- Plugins tab: tools and models configured in the rig. Settings tab: mode, permission, max steps.
- Schedule firing loop: every 30s, if idle, due scheduled prompts run as goals.

## Known limits
- `permission` ask shows an approval card for every mutating tool call (default is auto).
- Voice is the `speak` tool and needs a piper install and voice model (HS_TTS_MODEL). No packaging or signing. Not rebuilt from a fresh clone.
- Verified with a scripted model under Xvfb, not a live-model run.

## Build notes (sandbox)
Needs webkit2gtk-4.1 + gtk3 dev packages. The sandbox has no root, so headers and libs came from a
user-space sysroot (PKG_CONFIG_SYSROOT_DIR, RUSTFLAGS -L, and a bundled libsqlite3.a link arg).
These are sandbox-only workarounds; a normal machine needs none of them.
