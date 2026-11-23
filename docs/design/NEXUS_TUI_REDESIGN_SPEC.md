# Nexus TUI Redesign Spec — Phase 8 (Consolidated)

Date: 2026-11-23
Project: Nexus (`D:/WorkStudio/Nexus`)
Context: Phases 0–7 complete; Phase 8 consolidates the design work into the final redesign specification before implementation (Phase 9).

## 1. Design skill stack (Phase 0)

- Primary: `ui-ux-pro-max`; Design system: `hallmark`; Implementation polish: `emil-design-eng`; Supplemental:
  `apple-design`, `learn-taste`. Terminal-specific design skill: missing (reported; not installed).

## 2. Design philosophy (Phase 2)

- Nexus identity: collaborative dev environment in terminal; calm, powerful, keyboard-driven, focused, quiet.

## 3. Design tokens (Phase 3)

- Semantic color tokens table (background/elevated/panel/border/border.active/primary/secondary/muted/accent/
  success/warning/error/info/focus/selection/cursor/collaboration/modified); typography hierarchy; spacing; borders;
  component states.

## 4. Information architecture (Phase 4)

- Primary areas: Home, Cubes, Workspace, Collaboration, Terminal, Builds, Diagnostics, Settings; hierarchy rules;
  flows; constraints.

## 5. Screen designs (Phase 5)

- Screens 1–15 designed: Home, Cubes, Workspace, Editor, Explorer, Collaboration, Chat, Terminal, Build, Diagnostics,
  Command Palette, Settings, Dialogs, Notifications, Status Bar; implemented/remaining per screen.

## 6. Keyboard & interaction design (Phase 6)

- Principles, proposed key model, feedback/states, mouse.

## 7. Responsive design (Phase 7)

- Rules, existing behavior, design rules.

## 8. Design review findings (Phase 8)

- From audits and design cross-checks: first-launch blank state (repaired in HEAD), missing Home, minimal status bar,
  empty states, placeholder actions, navigation/focus semantics, persistence gaps.

## 9. Implementation plan inputs (Phase 9)

- Reuse architecture: layout engine, panel trait, command registry, keymap, theme, editor engine, status snapshot.
- Slices: welcome/Home alignment, header/status enrichment, navigation/focus fixes, responsive polish, states.

## 10. Acceptance criteria inputs (Phase 11)

- Derived from redesign goals: no blank first launch, understandable screen, nav guidance, shortcuts visible,
  resize, no panic, connectivity, no breakage.

## 11. Phase 8 status

- Deliverable created: `docs/design/NEXUS_TUI_REDESIGN_SPEC.md` (this file).
- No implementation started.
- **Awaiting approval before Phase 9** — Implementation.

⏸ **Please approve** the consolidated redesign spec to proceed to Phase 9 — Implementation.
