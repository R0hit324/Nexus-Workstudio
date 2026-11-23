# Nexus TUI Design Philosophy — Phase 2

Date: 2026-11-23
Project: Nexus (`D:\WorkStudio\Nexus`)
Context: Phase 0 (design-skill audit) ✅ approved → Phase 1 (current-state audit) ✅ → Phase 2 (design philosophy).

## 1. Purpose

Define a distinct, Nexus-specific design philosophy for the complete TUI UX/interface redesign, to be
operationalized through the design system (Phase 3), screens (Phase 5), interaction (Phase 6), responsive
rules (Phase 7), design review (Phase 8), implementation (Phase 9+), QA (Phase 10), acceptance (Phase 11), and
final report (Phase 12).

## 2. Design identity statement

> **Nexus is a collaborative development environment that lives in the terminal. Its interface should feel
> calm, powerful, and keyboard-driven — focused on the code being written, contextually honest about what the
> system is doing, and quietly sophisticated.**

The redesign must not look like:

- a generic terminal,
- a plain command-line application,
- a copy of VS Code or Neovim,
- a random collection of bordered panels,
- an overdecorated dashboard,
- an information-dense interface where everything competes for attention.

It should feel like:

- a professional collaborative development environment,
- a powerful keyboard-first developer tool,
- a modern terminal-native IDE,
- a focused environment,
- a lightweight but sophisticated application,
- a product with its own identity.

## 3. Core design principles (Nexus-specific)

1. **Focus is the product.** The editor (or the primary working surface) owns the visual center; panels serve
   context without taking over. Derived from Neovim/LazyVim/Zellij/Helix/Linear/tmux principles of focus.
2. **Calm by default.** Quiet surfaces, restrained accent use, hierarchy through structure not decoration.
   Avoids over-decoration and info-density.
3. **Keyboard-first, but discoverable.** Modal/panel keyboard model with command palette as the discovery
   mechanism; instant feedback; no animation for keyboard-initiated actions (emil-design-eng).
4. **Contextual honesty.** Status, empty, loading, error states always reflect real state; no fabricated data.
5. **Hierarchy through structure.** Header / content / footer regions; borders and focus indicators used
   sparingly and meaningfully.
6. **Terminal-native.** Character-cell geometry, no pixel assumptions, Windows Terminal-safe rendering,
   graceful small-terminal behavior.

## 4. Visual direction (synthesized)

- **Surfaces:** dark graphite / deep charcoal backgrounds; elevated panel surfaces slightly lighter; no pure
  black/white extremes.
- **Accent:** restrained primary accent (electric cyan/teal direction) with semantic colors (green success,
  amber warning, red error, violet/blue secondary) — semantic, not decorative.
- **Typography:** terminal font default; heading/labels hierarchy via weight/case/spacing only; no decorative
  fonts; readable at cell size.
- **Spacing:** cell-count based, consistent paddings; no px assumptions.
- **Borders:** full borders sparingly; focused panel distinguished by border/focus treatment; dividers where
  needed.

## 5. Inspiration extraction (principles only, not copies)

- **Neovim / LazyVim:** modal editing minimalism, quiet keybindings, leader efficiency.
- **Zellij:** pane/status clarity, keyboard-driven layout switching, status line discipline.
- **Helix:** calm single-purpose editing feel, selection/grammar integration.
- **VS Code / Linear / tmux:** activity-bar minimalism (VS Code), calm focus + command palette (Linear),
  minimal status line (tmux) — as *principles* only.

Nexus-specific synthesis: collaborative status (member presence, active file, cursors) layered onto the
focused editor workspace.

## 6. Phase 2 status

- Deliverable created: `docs/design/NEXUS_TUI_DESIGN_PHILOSOPHY.md` (this file).
- No implementation started.
- **Awaiting approval before Phase 3 (Design System) begins.**

## 7. Approval gate

Phase 2 is complete and documented.

⏸ **Please approve** the Nexus TUI design philosophy before Phase 3 (Design System) begins.
