# Nexus TUI Visual Redesign — Implementation Plan (current task)

Date: 2026-11-23
Context: Continuation of the `# NEXUS — COMPLETE TUI UX & INTERFACE REDESIGN` program (Phases 0–12 complete at
HEAD `2a535aa`). This plan covers the next implementation task: *TUI Visual Redesign & Implementation* against the
attached reference images (could not be read — image input unsupported; proceeding from the written brief).

## 1. Audit findings (root causes)

1. **Welcome rendered inside the editor frame.** `render_editor` (app.rs) wraps content in a bordered editor block
   (title = active cube or `"Editor"`); when no document is open it calls `render_welcome` into the block's inner
   area. `welcome_lines` pushes lines without padding, so content hugs the top-left and the rest of the large
   bordered frame stays empty — the reported "debug output" look.
2. **No dedicated Home dashboard composition.** First-launch content exists only as `welcome_lines`; no branding
   header block, welcome section, primary actions, recent cubes, quick start, activity, or shortcut footer.
3. **Create Cube not reachable.** `Action::CreateCube(String)` exists and is applied via `SessionService`, but no
   `Command::CreateCube` variant/registry/dispatch was wired — the real flow could not be triggered from the UI.
4. **Join Cube** is command-dispatched (`C::JoinCube` → `Action::JoinCube(selected_cube_id)` → session join).
5. **Real data sources available:** `SessionService.state` (connection, user_label, active_cube, cubes,
   members, last_error, chat); `App::apply_action` for Create/Join/Leave (Remote). No activity feed exists — must
   show a clean empty state, never fake events.

## 2. Phase A — Home Dashboard (no active Cube) — IMPLEMENTATION PLAN

- [x] 1. Add `Command::CreateCube` (enum + ALL + `name()` + registry spec) and dispatch →
      `Action::CreateCube(generated name)` → `SessionService::create_cube` (real flow).
- [ ] 2. Branch in `render::draw`: when `session.state.active_cube.is_none()` → render home content into content
      area instead of editor panel frame.
- [ ] 3. Add `render_home(app, frame, area)` + `home_lines(app) -> Vec<Line>` (brand, tagline, status, cube,
      welcome, primary actions, recent cubes, quick start, activity empty state, shortcuts).
- [ ] 4. Add tests for home content (no cube: actions/recent-cubes/guidance; joined cube: not used).

## 3. Phase B — Active Cube Workspace — IMPLEMENTATION PLAN

- [ ] 1. `render_header`: include active Cube name and real command hints (Ctrl-P, Ctrl-O, `/`, Ctrl-Q) when inside
      a Cube; keep compact (single row).
- [ ] 2. Editor empty state for "Cube active but no file open" (calm guidance, distinct from first-launch welcome).
- [ ] 3. Status bar already enriched (focus/members/cube/file/ln-col/mode) — verified, no change needed.

## 4. Constraints

- Reuse single render path, Action/Command, pure layout, theme styles; no new architecture.
- No fake data; actions must hit real `Action` → `SessionService` flows.
- Minimal targeted changes; run fmt/clippy/check/build/test before commit.

## 5. Verification targets

- `cargo check -p nexus-tui --all-targets`, `cargo check --workspace --all-targets`, `cargo build --workspace`,
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test -p nexus-tui`.
- Existing welcome tests must keep passing unless intentionally replaced.
