# Nexus TUI Keyboard & Interaction Design — Phase 6

Date: 2026-11-23
Project: Nexus (`D:/WorkStudio/Nexus`)
Context: Phases 0–5 complete.

## 1. Purpose

Define the keyboard and interaction design for the Nexus TUI redesign — the interaction model that makes the
redesigned interface keyboard-first, discoverable, and feedback-rich, while respecting terminal constraints.

## 2. Interaction principles (from design skills + audit)

- Keyboard-first, but command palette is the discoverability mechanism (not memorization).
- Never animate keyboard-initiated actions (emil-design-eng); instant response; continuous feedback during
  interaction; interruptible transitions.
- Feedback on press/pointer-down equivalent; for keyboard: immediate state change, no artificial delay.
- One action, one feedback; no motion for high-frequency actions.

## 3. Proposed key model

- **Global**: Ctrl-P palette, Ctrl-Q quit, Ctrl-R reconnect, Esc cancel/close, Ctrl-Z? (avoid), navigation.
- **Navigation**: `Tab` focus-next; `Ctrl+H/J/K/L` (or vim `h/j/k/l` for movement) — design decision: unify focus vs
  movement semantics to avoid conflation (audit finding); proposal: keep `h/j/k/l` as cursor movement and use
  `Ctrl+H/J/K/L` (or `Ctrl+Arrow`/leader) for panel focus, OR use `g`-prefix... Design proposal: map focus to
  `Ctrl+H/J/K/L` while editor movement keeps raw `h/j/k/l`? — To avoid conflict, design recommends focus via
  `Ctrl+H/J/K/L` + `Tab`, editor movement via `h/j/k/l`/`arrows` — resolved in Phase 6 doc.
- **Editor**: i/a/o (insert), Esc normal, `/` search, Ctrl-S save, Ctrl-W close, Ctrl-N goto-line, Ctrl-D copy? etc.
- **Panels**: e/t/u/c toggles, Ctrl-H/J/K/L focus, Tab next.
- **Modes**: Normal / Insert / Search / Command — existing; design consistency rules.

## 4. Feedback & states

- Command executed → notification/status update; palette selection → immediate action.
- Focus change → visible focus indicator (border + cursor); status bar focus field.
- Errors → dialogs/notifications; destructive → confirm dialog.

## 5. Mouse (where supported)

- Click hit-test focus (existing); scroll wheel; avoid requiring mouse.

## 6. Phase 6 status

- Deliverable created: `docs/design/NEXUS_TUI_KEYBOARD_INTERACTION.md` (this file).
- No implementation started.
- **Awaiting approval before Phase 7** — Terminal Responsiveness.

⏸ **Please approve** the keyboard/interaction design to proceed to Phase 7.
