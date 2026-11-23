# Nexus TUI Design System — Phase 3

Date: 2026-11-23
Project: Nexus (`D:/WorkStudio/Nexus`)
Inputs: Phase 0 design-skill audit (approved stack), Phase 1 current-state audit, Phase 2 design philosophy.
Context: Phases 0–2 complete; design system is established before screen redesign implementation (Phase 9).

## 1. Purpose and scope

Establish a complete, terminal-native design system for the Nexus TUI redesign, translating the Phase 2 design
philosophy into concrete, implementable design tokens and component-state rules that the screen redesign (Phase 5)
and implementation (Phase 9) will follow.

## 2. Color system

Semantic token model (not component-local color assignment). Tokens:

| Token | Meaning | Recommended value (dark graphite direction) |
|---|---|---|
| `background` | App/base background | deep graphite `#14161C` (approx; tuned for terminal) |
| `elevated` | Input areas, palettes, popups, editors | slightly lighter graphite `#1B1F27` |
| `panel` | Panel fill behind borders | `#171A21` |
| `border` | Default border | muted gray `#3A4150` |
| `border.active` | Focused panel border | accent cyan/teal `#3FD6E0` |
| `primary-text` | Primary content | `#E8EAF0` |
| `secondary-text` | Secondary content | `#9AA3B2` |
| `muted-text` | Metadata/status | `#6B7280` |
| `accent` | Primary interactive/identity accent | electric cyan/teal `#3FD6E0` |
| `success` | Success/connected-good | green `#4ADE80` |
| `warning` | Warning/modified/attention | amber `#FBBF24` |
| `error` | Error | red `#F87171` |
| `info` | Informational | cyan/blue `#56CCF2` or accent family |
| `focus` | Focus indicator | accent `#3FD6E0` (or selection color) |
| `selection` | Selection/cursor background | blue-violet `#5B8DEF` / accent-tinted |
| `cursor` | Text cursor | accent-tinted background |
| `collaboration` | Collaboration indicators (cursors, members) | violet `#A78BFA` |
| `modified` | Modified-file indicator | amber `#FBBF24` (or accent) |

Design rules:

- Colors are semantic and carry meaning; no rainbow panels.
- Status colors reserved for status meaning (success/warning/error/info/collaboration).
- Reduced reliance on color-only meaning (pair with symbols/text where feasible).
- Terminal compatibility: prefer 16-color/256-color-safe values; avoid truecolor-only requirements; ensure
  contrast (WCAG-ish 4.5:1 for text) in dark theme; respect terminal color capability detection where architecture
  permits.

## 3. Typography

Hierarchy (terminal cell constrained):

| Role | Treatment |
|---|---|
| Screen title | Bold, primary-text color, sentence case |
| Panel title | Bold, accent or primary, with border prominence for focused |
| Section title | Bold/semibold, secondary-text |
| Primary text | primary-text, normal weight |
| Secondary text | secondary-text |
| Metadata | muted-text, normal |
| Status text | semantic color, normal/bold per severity |
| Command description | muted-text, normal |
| Highlighted text | accent/info color, optional bold |

Rules: bold sparingly; consistent capitalization; meaningful symbols (not emoji); readable alignment; consistent
spacing; no custom fonts; no decorative Unicode that breaks in Windows Terminal.

## 4. Spacing

- Cell-based scale: padding values in terminal rows/columns (e.g., panel inner padding ~1 row; section spacing
  1–2 rows; header/footer 1 row).
- Sidebar width: design range ~20–30% of width, min ~18 cols, max ~40%.
- Avoid unnecessary borders and double-border overload; use dividers where appropriate.

## 5. Borders and panels

- Full borders for focused/primary panels; borderless or partial for secondary.
- Active panel border uses `border.active`; unfocused uses `border`.
- Focus identifiable without overwhelming (accent border + focus style).
- Dividers for section separation.

## 6. Component states

Defined for TUI components:

- **Editor**: focused (accent border + cursor accent), unfocused, tab active/inactive, modified (`modified`),
  dirty/save state, empty (welcome/empty state), loading (large file), error (syntax/diagnostic).
- **Explorer**: focused/unfocused/selected, expanded/collapsed dir, file/dir distinction, empty, loading.
- **Users/presence**: online/busy/away/offline, empty, disconnected.
- **Chat**: focused/unfocused, composing/sent, empty, error.
- **Terminal**: focused/unfocused, running, exited (exit code), scrollback, empty.
- **Build**: running/success/failed/queued/cancelled, output, diagnostics.
- **Status bar**: connected/disconnected/connecting, mode, file, cube, sync, runtime — single-line, truncating.
- **Palette**: open/query/results/selected/empty.
- **Dialogs**: confirm/input/error/info, destructive emphasis.
- **Notifications**: info/success/warning/error, sticky vs transient, empty.

## 7. Mapping to existing architecture

- Tokens/states map onto existing `theme.rs` StyleKeys and `status.rs` UiSnapshot — design system refines, does
  not replace.
- Borders/panels per `layout.rs` rects.
- Empty states per panel render paths (welcome, explorer/users added context in HEAD).

## 8. Phase 3 status

- Deliverable created: `docs/design/NEXUS_TUI_DESIGN_SYSTEM.md` (this file).
- No implementation started.
- **Awaiting approval before Phase 4** (Global Information Architecture).

## 9. Approval gate

Phase 3 complete and documented.

⏸ **Please approve** the Nexus TUI design system to proceed to Phase 4 — Global Information Architecture.
