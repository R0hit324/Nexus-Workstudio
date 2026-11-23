# Nexus TUI Screen Redesigns — Phase 5

Date: 2026-11-23
Project: Nexus (`D:/WorkStudio/Nexus`)
Context: Phases 0–4 complete (skill audit ✅, current-state audit ✅, philosophy ✅, design system ✅, navigation ✅).

## Screen 1 — Home / Dashboard

- **Purpose:** orient the user: identity, connection, Cubes, quick actions, next steps.
- **States:** first-launch (welcome/guidance — delivered in HEAD via welcome content), returning (recent Cubes when
  real data exists), disconnected.
- **Design:** centered-ish focused layout; connection status card; Cube list/quick actions; no fabricated activity.
- **Implemented:** welcome content (HEAD `9f18b95`), header, status bar. **Remaining:** Home dashboard composition.

## Screen 2 — Cube Management

- **Purpose:** list/join/create/leave/manage Cubes.
- **Design:** Cube list with status/members; join flow (design: command palette + flow); create (design).
- **Implemented:** actions exist (`JoinCube`, `CreateCube`, `LeaveCube`) via Action/Command; UI surfaces design-stage.
- **Remaining:** UI surfaces for Cube management.

## Screen 3 — Main Development Workspace

- **Purpose:** primary working area: explorer + editor + contextual collab/chat/terminal/build.
- **Design:** editor primary; sidebar explorer; bottom contextual panels; status bar; header.
- **Implemented:** editor engine, panels, layout (editor always present, toggles, compact mode); welcome in editor.
- **Remaining:** panel layout flexibility, focus UX.

## Screen 4 — File Explorer

- **Design:** files vs dirs vs selected vs expanded vs modified vs new; context actions; refresh; empty state
  (`No files yet.` delivered).
- **Implemented:** explorer state + FS read; empty-state text; network wiring missing.

## Screen 5 — Editor

- **Design:** tabs, title, modified indicator, line numbers, cursor, selection, syntax, diagnostics, search,
  scroll, collab indicators, mode, file info; focused = accent border/cursor; empty = welcome.
- **Implemented:** editor engine + welcome fallback; title fallback fixed to `Editor`.

## Screen 6 — Collaboration

- **Design:** users/presence panel, chat, presence indicators, active file/cursors; empty state.
- **Implemented:** users state + sync + empty-state text; chat partial; presence collab tokens in design system.

## Screen 7 — Chat

- **Design:** history, author, input, send, scroll, empty state, error state.
- **Implemented:** chat state primitives; send path partial.

## Screen 8 — Terminal

- **Design:** terminal tabs, status, process, output, input, exit status; focused/unfocused/running states.
- **Implemented:** terminal panel state; run/output wiring.

## Screen 9 — Build and Run

- **Design:** target, status, duration, exit code, output, errors, warnings, progress, cancel; success/failure states.
- **Implemented:** build state machine + diagnostics; protocol wiring partial.

## Screen 10 — Diagnostics

- **Design:** errors/warnings/info with file/line/column/message; filter/navigation where supported; distinct from
  build output.

## Screen 11 — Command Palette

- **Design:** query, results, title, description, shortcut, category, selected, empty state; single discovery
  surface reusing `CommandRegistry`.

## Screen 12 — Settings

- **Design:** only existing/real implementable settings; current/default/unsupported distinction.

## Screen 13 — Dialogs and Modals

- **Design:** titles, purpose, fields, actions, primary vs destructive, keyboard nav, validation errors, size.
- **Implemented:** dialog state exists; no flows push dialogs.

## Screen 14 — Notifications

- **Design:** short, contextual, non-intrusive; success/info/warning/error; sticky vs transient; important errors
  discoverable.

## Screen 15 — Status Bar

- **Design:** connection, Cube, file, mode, focus, collaboration, sync, diagnostics, runtime; predictable placement;
  truncating; empty-state minimal (`Nexus │ CONNECTED │ NORMAL` delivered).

## Phase 5 status

- All major screens designed.
- No implementation started beyond prior first-launch repair (which is design-aligned).
- **Awaiting approval before Phase 6** — Keyboard and Interaction Design.

⏸ **Please approve** the screen redesigns to proceed to Phase 6.
