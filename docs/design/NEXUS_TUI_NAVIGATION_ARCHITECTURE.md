# Nexus TUI Navigation Architecture — Phase 4

Date: 2026-11-23
Project: Nexus (`D:/WorkStudio/Nexus`)
Context: Phase 0 (design-skill audit) ✅ → Phase 1 (current-state audit) ✅ → Phase 2 (philosophy) ✅ → Phase 3 (design system) ✅ → Phase 4 (navigation architecture).

## 1. Purpose

Redesign the Nexus TUI navigation architecture so users always understand: where they are, what they are doing,
which Cube/workspace is active, which panel has focus, and what to do next — within terminal screen constraints.

## 2. Audit baseline (from Phase 1)

Current navigation mechanisms (all existing, to be refined — not replaced):

- Panel toggles: explorer, terminal, users, chat, build (`PanelToggles`), toggled via commands (`e/t/u/c`) or palette.
- Focus graph: `FocusGraph` over visible panels + editor; Tab / Ctrl+H/J/K/L / explicit focus commands.
- Command palette: `Ctrl-P`, fuzzy search over `CommandRegistry`.
- Direct commands: `:` command line, keymap-bound commands, mouse click hit-test (`panel_at`).

Gaps: no Home screen; hardcoded focus targets; no navigation history; no breadcrumbs.

## 3. Proposed navigation architecture

### 3.1 Primary areas

Design a navigation model with primary areas (not necessarily ten permanently visible panels):

1. **Home** — connection, identity, Cubes (recent/available), quick actions, guidance. (New; replaces first-launch
   welcome as the persistent landing surface when no Cube is active; dashboard content only when real data exists.)
2. **Cubes** — Cube list/join/create/leave/manage. (Commands exist; UI surface to be designed.)
3. **Workspace** — file explorer + editor + collaboration + terminal + build integrated working area.
4. **Collaboration** — users/presence/chat overlays.
5. **Terminal** — PTY panel.
6. **Builds** — build output/diagnostics.
7. **Diagnostics** — dedicated view over diagnostics (design; build panel exists).
8. **Settings** — design only if implemented (none currently).

### 3.2 Hierarchy rules

- Home → (join/select Cube) → Workspace → contextual panels.
- Within workspace: editor primary; explorer sidebar; contextual collab/chat/terminal/build.
- Focus movement: graph order (explorer → editor → users → chat → terminal → build), wraps; keyboard mapping to
  be made vim-consistent and documented.
- Command palette: single discovery surface for all commands (existing registry reused).

### 3.3 Navigation flows

- First launch (no Cube): Home/welcome → guidance → create/join Cube → workspace.
- Returning user: Home → recent Cube → workspace.
- In workspace: editor-focused; panel switches via toggles/focus/palette.
- Context switch: leaving Cube → Home/Cubes.

### 3.4 Constraints honored

- Terminal screen size limits navigation richness (Phase 7 responsive rules).
- No new navigation architecture; reuse `CommandRegistry`, `Keymap`, `PanelRuntime`, `FocusGraph`.

## 4. Phase 4 status

- Deliverable created: `docs/design/NEXUS_TUI_NAVIGATION_ARCHITECTURE.md` (this file).
- No implementation started.
- **Awaiting approval before Phase 5** — Screen Redesign.

⏸ **Please approve** the navigation architecture to proceed to Phase 5 — Screen Redesign.
