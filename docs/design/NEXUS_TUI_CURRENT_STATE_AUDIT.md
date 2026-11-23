# Nexus TUI Current State Audit — Phase 1

Date: 2026-11-23
Project: Nexus (`D:\WorkStudio\Nexus`)
TUI crate: `crates/nexus-tui`
Audit context: preceded by Phase 0 design-skill audit (`docs/design/NEXUS_TUI_DESIGN_SKILL_AUDIT.md`) and approval.

## 1. Purpose and scope

Audit the existing Nexus TUI implementation **as it exists in the repository at HEAD** (commit `9fe420d`,
the end of the first-launch repair arc) before the UX/interface redesign begins. The audit is read-only:
**no application code was modified during this phase.**

## 2. Repository and crate map

| Crate | Role relevant to TUI audit |
|---|---|
| `nexus-core` | Domain types, config (`LoggingConfig`, `NexusConfig`), diagnostics, build types, ids, events, collaboration, terminal abstractions. |
| `nexus-client` | `SessionService`, `ClientState`, `WorkspaceService`, `NetworkEvent`, `EventSink`, `Client` transport. |
| `nexus-server` | Health, config validation, migrations, diagnostics, storage, sessions, collaboration, builds. |
| `nexus-tui` | **The audited surface**: terminal setup, input translation, app state, layout, render, panels, editor, status, dialogs, notifications, palette, persistence. |
| `nexus-terminal` | PTY/terminal abstraction used by client/server for terminal streams. |
| `nexus-protocol`, `nexus-events`, `nexus-workspace`, `nexus-sync`, `nexus-ai`, `nexus-build`, `nexus-runtime`, `nexus-snapshot`, `nexus-plugin` | Supporting domains (event sync, builds, collaboration, plugins). |

## 3. Current UI architecture

- **Layering** (as documented in `nexus-tui/src/lib.rs`): TUI panels/widgets (ratatui, terminal-only) →
  Application (`App`, state, command dispatch) → Client services (`nexus-client`) → protocol/transport.
- **Render path**: single `render_frame(app, frame)` → internal `render::draw(app, frame, &map)`.
  No second render architecture exists. Widgets consume pre-computed `Rect`s from `LayoutMap`.
- **State flow**: crossterm events → `InputTranslator` → `UiEvent` → `App::handle_event` (mode-aware:
  Normal/Insert/Search/Command) → keymap resolve → `Command` → `dispatch_command` → `apply_action` (local or
  session/network) → `tick` drains `SessionService` events → network events → notifications/state updates →
  next frame renders. Widgets never touch the network.
- **Layout**: pure functions (`layout_main`, splits, focus graph) in `layout.rs`; `PanelRuntime` holds toggles,
  focus, rects; `App::resize` recomputes; compact mode (<8 rows or <40 cols) hides optional panels.

## 4. Existing screens and their real state

1. **First launch / no Cube (as of current HEAD):** App starts, connects, no document, no Cube.
   - Pre-repair behavior: blank editor pane (empty bordered block titled via fallback) + status bar
     `Nexus │ CONNECTED │ NORMAL`.
   - **Post-repair behavior (in HEAD):** welcome content rendered in the editor pane (`welcome_lines`:
     branding, connection status, Cube status, next step, shortcuts); header rendered (brand + user + connection);
     status bar footer; empty-state context for users/explorer panels. *This repair was delivered in commits
     `9f18b95`, `9fe420d`.*
2. **Cube joined, file open:** editor pane renders the active document (tab line, gutter, syntax highlighting,
   cursor, scroll window); status bar shows file/line/col/mode; header shows identity.
3. **Panel-driven screens:** explorer / users / chat / terminal / build panels toggled via keymap/commands;
   editor always present; palette overlay; dialogs overlay; notifications overlay.

**There is no dedicated Home/Dashboard screen** — navigation is panel/command-palette based.

## 5. Existing components — implemented vs partial vs placeholder vs missing

| Component | Status | Notes |
|---|---|---|
| Terminal setup (`main.rs`) | Implemented | Raw mode, alt screen, mouse capture, panic hook + restore, run loop (33 ms tick, resize re-read, single draw). |
| Input translation | Implemented | crossterm → `Key`/`UiEvent`/`MouseEvent`; releases dropped; paste limited. |
| Layout engine | Implemented | Pure + tested (rect math, overlap, compact, focus graph). |
| Editor engine (`editor/*`) | Implemented | Gap buffer, cursor, document, undo, search, syntax, remote cursors — terminal-free and unit-tested. |
| Explorer | Partially implemented | Listing state + FS read implemented; **not wired** to workspace listings from network (`WorkspaceListed`/`FileChanged` no-op in `App::on_network_event`). |
| Users panel | Implemented (state) | Presence sync via `UsersChanged` implemented in `App::sync_users`; render added empty-state text. |
| Chat | Partially implemented | State/history/composer primitives implemented; chat input send path not wired into key flow; messages render. |
| Terminal panel | Implemented (state) | Scrollback bounded; run/build output ingestion via session. |
| Build panel | Implemented (state) | Build lifecycle + diagnostics state implemented; network event wiring present. |
| Status bar | Implemented (basic) | Snapshot + trimming; missing permission/sync/runtime segments; `permission` always `None`. |
| Command palette | Implemented (state + fuzzy) | Rendering + input implemented; command list from registry. |
| Dialogs | Implemented (state) | Queue + kinds implemented; **no flows currently push dialogs** (audit finding). |
| Notifications | Implemented | Center + TTL/sticky; pushed from network/actions. |
| Theme | Implemented | Style keys + default theme; no theme switching UI. |
| Persistence (`persistence.rs`) | Partial | `LocalPreferences` (recent_cubes, visible_panels, bindings, theme, editor, window) loaded in `App::new` but **only bindings applied**; recent_cubes, visible_panels, theme, window not applied. |

## 6. Existing navigation and keyboard design (current state)

- Default keymap (Normal): Ctrl-P palette, Ctrl-S save, Ctrl-B build, Ctrl-O open, Ctrl-Q quit, Tab focus-next,
  Ctrl-H/J/K/L focus-*, h/j/k/l move, arrows move, PgUp/PgDn, e/t/u/c toggles, `/` search, r reconnect, Ctrl-N
  goto-line, Ctrl-W close tab, Alt-W/Ctrl-Space next tab, s split-editor (placeholder-ish).
- Insert/Search/Command modes implemented; unhandled keys ignored.
- **Audit findings (usability-relevant):**
  - Focus commands `FocusLeft/FocusRight/FocusUp/FocusDown` are hardcoded to specific panels rather than
    graph-driven context (e.g., `FocusLeft` → Explorer regardless of current focus).
  - Key semantics are inconsistent for vim users: `h/j/k/l` move cursor left/down/up/right, but Ctrl+H/J/K/L are
    focus commands — conflating movement and panel focus.
  - Plain-mode fallbacks (`i/a/o` → insert) exist outside the keymap system.

## 7. UX/design problems identified during audit

1. **Blank first-launch screen** (now repaired in HEAD via welcome/header/empty-state work; diagnosis documented
   in prior task phase) — no welcome, no home, no guidance; primary first-launch UX gap.
2. No Home/dashboard screen; navigation is panel/toggle/palette-centric — navigation architecture to be
   redesigned (Phase 4 of the redesign program).
3. Status bar minimal/incomplete (no permission, sync, runtime, collaboration context; cube/file segments empty
   on first launch).
4. Empty panels without contextual information (explorer/users) — addressed in HEAD with placeholder text.
5. No distinct visual identity (thematic tokens beyond single default theme; border/title treatment minimal).
6. Layout persistence not applied (`visible_panels`, `recent_cubes`, `theme`, `window` in prefs unused).
7. Placeholder actions surfaced as real (split-editor stub, paste no-op) — must not be presented as functional.
8. Dialogs exist but no user flows push them — missing workflows for create/join Cube, file ops.

## 8. Architectural constraints (must be preserved during redesign)

- Single render architecture; widgets never touch network; state via Action/Command; layout pure.
- No duplicate TUI/renderer/editor/command systems may be introduced.
- No fake functionality; mark unavailable features as unavailable rather than faking.
- Minimal targeted changes preferred over rewrites.

## 9. Missing workflows / capabilities

- Home/dashboard content (recent Cubes, quick actions, guidance) — not implemented.
- Create/Join Cube flows end-to-end (UI + server confirmation) — partially present (actions exist; UI wiring missing).
- Settings screen — not implemented.
- Diagnostics screen — not implemented as a screen (build panel exists).
- Workspace file-open flow from explorer — no-op listing wiring.

## 10. Existing components recommended for reuse

- Layout engine (`layout.rs`), `Panel` trait + toggles/focus (`panels`, `layout`), status snapshot (`status.rs`),
  theme styles (`theme.rs`), notifications (`notifications.rs`), dialogs (`dialogs.rs`), palette (`palette.rs`),
  editor engine (`editor/*`), terminal/build/chat/users state, command registry (`commands.rs`), keymap (`keymap.rs`),
  input translator (`input.rs`).

## 11. Components needing modification for redesign

- `app.rs` render module (welcome/header now added; will be redesigned), status bar snapshot, layout toggles
  persistence, focus command semantics, explorer wiring, chat input path, dialog-driven flows, welcome content.

## 12. Phase 1 status

- Deliverable created: `docs/design/NEXUS_TUI_CURRENT_STATE_AUDIT.md` (this file).
- No code modified.
- **Awaiting design-approval gate outcome before Phase 2 (Nexus design philosophy) begins.**

## 13. Phase 1 — audit summary for owner

- Architecture: clean hexagonal-ish split, no architectural debt that blocks redesign; constraints are explicit.
- Primary UX problems: first-launch blank screen (repaired in HEAD), missing home/dashboard, minimal status bar,
  empty-state gaps, placeholder actions, navigation/focus semantics issues, persistence of layout prefs unused.
- Ready for design phases (2–8) to proceed on top of this audited foundation.

**Phase 1 complete.** Awaiting approval before Phase 2 begins.
