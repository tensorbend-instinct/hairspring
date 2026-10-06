# HAIRSPRING desktop app (Tauri v2) - design

Status: scaffold only (apps/desktop). Not yet a faithful dsh-parity app.

## Principle
The app is a thin shell over the `hairspring` binary. The loop, checker, critic,
held-out promotion and log stay in the CLI; the UI never reimplements them.

## Fresh install (Codex / Claude Code style)
1. First launch runs `hairspring setup --check`. Exit 0 = a provider is ready.
2. Otherwise show a setup screen: pick provider, paste key, app runs
   `hairspring setup` (validates, saves owner-only under ~/.config/hairspring/keys/).
3. Pick a project folder (missions are confined to it). Then the main view.
4. Bundle: `hairspring` and its sibling plugin binaries ship as Tauri sidecars.

## Main view (parity targets from the CLI/TUI already built)
- Transcript with folded reasoning (`/reasoning`), tool rows "Completed in Ns".
- Always-on footer: elapsed, steps, calls, tok/s, cache %, context remaining %,
  running-tool ticker, doom-loop / REPEAT n signal.
- Session switcher grouped by workspace (SessionInfo.workspace).
- Lineage and evidence panels (selfmod_view, evidence_view).
- Resume / fork sessions (`--resume`, `--fork`).

## Not built yet
Setup screen, project picker, sidecar bundling, event-structured mission output
(the scaffold streams raw stdout lines), session list/switcher UI, footer vitals,
fold toggles, lineage/evidence panels, packaging and signing, fresh-machine regression.

## Build notes (sandbox)
Needs webkit2gtk-4.1 + gtk3 dev packages. Sandbox has no root: dev packages were
unpacked with dpkg -x into a sysroot and linked via PKG_CONFIG_SYSROOT_DIR.
