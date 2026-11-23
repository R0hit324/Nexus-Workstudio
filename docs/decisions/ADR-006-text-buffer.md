# ADR-006: Gap-Buffer Text Buffer In The Editor

- **Status**: Accepted
- **Deciders**: WorkStudio Nexus maintainers
- **Date**: 2026-09-13

## Context

Part 2 requires a working editor at the core of the client TUI. The editor is a
many-pane surface (text editor, file explorer, tabs, splits, status line,
users list, terminals, build output, chat, command palette, dialogs). The
plain-text editing core must be:

- **Cheap to type into** — an editor is judged on keystroke latency; the common
  case is an insertion near the cursor with small edits scattered around.
- **Cheap to revert** — local edits are marked dirty until synced; we may want
  undo/redo later, and re-rendering a pane must not rebuild the whole document.
- **Terminal-friendly** — rendering is line-oriented; the buffer must expose
  lines cheaply and track a selection/cursor in character coordinates.
- **Small** — this is a TUI "research-y" codebase; a full rope or piece-table
  engine is overkill for the current scope.

A `Vec<String>` plain line array satisfies line rendering but makes a Cursor Up
spanning wrapped lines, selection expansion, and dirty tracking awkward, and
insertions mid-file shift the whole tail on every keystroke.

## Decision

Use a **gap buffer** as the editor's text document:

- A `GapBuffer` holds two `Vec<char>`s — `left` (text before the cursor gap)
  and `right` (text after the gap). The gap sits between them and is always at
  the cursor position, so `left.last()` / `right.first()` are O(1).
- Core ops in `GapBuffer`:

  | Op | Cost |
  |---|---|
  | `insert(ch)` | O(1) amortized (grow by doubling as needed) |
  | `delete_back()` | O(1) (pop `left`) |
  | `delete_forward()` | O(1) (pop `right`) |
  | `move_gap(delta)` | O(n) worst case, but proportional to distance moved |
  | `char_at(idx)` | O(1) *
  | `to_string()` / `line_at(n)` | O(n), only when rendering/exporting |

  \* after a `move_gap(delta_index)` walk that lands the gap at `idx`.

- `Document` composes a `GapBuffer`, a `selection: Range`, a `path`, `file_id`,
  `version`, and `dirty: bool`. Selection is stored as an anchor/head in
  absolute character indices so word/line select, extend, and clip operations
  are unchanged by wrapping.
- The gap is forced to the cursor position before any read or render buffer
  export, so "current line under cursor" and `line_at` stay exact.

Alternatives:
- **Plain `String` / `Vec<char>` single buffer** — O(n) shifts on every
  mid-file edit; typing at file top rescans the tail each keystroke. Rejected.
- **Piece table / rope** — great for large files and undo, but heavier to
  implement and maintain; current files are small. Rejected for now.
- **Line array (`Vec<String>`)** — natural for pane line rendering, but moves
  editing logic into per-line re-splitting and makes selection/vicinity math
  leak across module boundaries. Rejected; the document owns a flat
  character buffer and the render layer derives lines from it.

## Consequences

- Keystroke insert/delete stay O(1) amortized; scanning is confined to
  gap-moves whose cost is proportional to the distance moved.
- Cursor and selection are absolute character indices, so multi-pane layout,
  word/line selection, and dirty tracking share one simple coordinate space.
- Rendering and I/O pay an O(n) full pass when exporting a snapshot; acceptable
  at TUI scale and only on paint/tick, never per keystroke.
- `Document::dirty` marks the workspace entry via `is_dirty(file_id)`, giving
  the status/explorer a cheap "unsaved changes" signal without re-reading text.

## Referenced by

- `crates/nexus-tui/src/editor/buffer.rs` — `GapBuffer` implementation.
- `crates/nexus-tui/src/editor/document.rs` — `Document` compositing the gap
  buffer with undo, search, and cursor.
- `crates/nexus-tui/src/editor/cursor.rs` — caret and selection in absolute
  character indices.
- `crates/nexus-client/src/app/state.rs` — dirty-file tracking keyed by
  `FileId`.