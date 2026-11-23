# Nexus TUI Responsive Design — Phase 7

Date: 2026-11-23
Project: Nexus (`D:/WorkStudio/Nexus`)
Context: Phases 0–6 complete.

## 1. Purpose

Ensure the redesigned Nexus TUI remains usable across terminal sizes and shapes, per Phase 2 philosophy
("terminal-native") and Phase 3 design system.

## 2. Existing responsive behavior (from audit)

- `layout_main`: compact when `rows < 8 || cols < 40` → optional panels hidden, editor = content.
- `App::resize` clamps to min 40x8; recomputes layout each frame; main re-reads size.
- Panels with fixed minimum widths; bottom panels stack via split.

## 3. Design rules for redesign

- Primary content always visible; optional panels hidden in compact mode (already).
- Header 1 row; status bar 1 row; editor flexible; sidebar min 18 cols; bottom panels max 12 rows.
- Small terminals: hide sidebar/bottom panels progressively; keep status bar + at least editor; show guidance.
- Large terminals: allow optional max widths; no fixed pixel assumptions.
- Text clipping/wrapping rules: no overflow past borders; Paragraph clipping; status line hard-truncate to width.
- Unicode/emoji width: avoid emoji where cell width matters (Windows Terminal); use ASCII/symbols.

## 4. Phase 7 status

- Deliverable created: `docs/design/NEXUS_TUI_RESPONSIVE.md` (this file).
- No implementation started.
- **Awaiting approval before Phase 8** — Visual Design Review / Redesign Spec.

⏸ **Please approve** the responsive design to proceed to Phase 8 — Visual Design Review and consolidated redesign spec.
