# Nexus TUI Design Skill Audit — Phase 0

Date: 2026-11-23
Project: Nexus (`D:\WorkStudio\Nexus`) — collaborative LAN-first development TUI (Rust / ratatui / crossterm)
Task: `# NEXUS — COMPLETE TUI UX & INTERFACE REDESIGN` (Phase 0 of the redesign program)

## 1. Skill discovery

Discovery was performed through the OpenCode `find-skills` skill and by direct inspection of
installed skill directories:

- User-level agent skills: `C:\Users\HP\.agents\skills\` → `find-skills`, `hallmark`, `ui-ux-pro-max`
- OpenCode harness skills: `C:\Users\HP\.config\opencode\skills\` → 44 skills including design-relevant ones
  (`emil-design-eng`, `modern-web-design`, `learn-taste`, `apple-design`, `break-ui`, `prototype`, …)
- No project-local skills were present under the Nexus repository.

A filesystem/skill-category sweep was used to detect design-adjacent, terminal-specific, and
interaction-design skills. No terminal/TUI-specific design skill was found.

## 2. Skills inspected (design-relevant)

| Skill | Location | Capability summary | Relevance to Nexus TUI redesign |
|---|---|---|---|
| `find-skills` | `C:\Users\HP\.agents\skills\find-skills` | Discovery/orchestration for the agent-skills ecosystem (`npx skills find/add/update`, skills.sh) | Used this phase to conduct discovery; not a design skill. |
| `ui-ux-pro-max` | `C:\Users\HP\.agents\skills\ui-ux-pro-max` | UX design intelligence: 120 UX guidelines (accessibility, interaction, performance, style, layout, typography, color, animation, forms, navigation, charts); searchable local DB (80 styles, 192 palettes, 74 font pairings, 105 icons, 22 stacks); workflow: design-system generation, domain searches, stack guidelines, pre-delivery checklist. | **Primary UX design source.** Web/mobile/desktop scoped; terminal stack not in its stack list, but general UX rules (contrast, keyboard nav, focus, motion) transfer. |
| `hallmark` | `C:\Users\HP\.agents\skills\hallmark` | Design-system creation for new pages: design tokens/themes, layout, anatomy, craft, interaction-and-states, responsive, gates, finish review; `sloplint` quality sweep; one-idea-per-screen; invariants (tokens, no `transition: all`, no italic-heading, focus-visible, reduced-motion, no invented metrics). | **Design-system source.** Heavily web/CSS-centric; invariants partially transferable to terminal. |
| `emil-design-eng` | `C:\Users/HP/.config/opencode/skills/emil-design-eng/SKILL.md` | Design-engineering philosophy: taste, unseen details, animation decision framework (should animate at all; never animate keyboard-initiated actions; respond on press; continuous feedback during interaction; motion from current value/momentum/interruptible), component polish review format (Before/After table with Why). | **Implementation/polish source.** Directly transferable to TUI interaction design (feedback, focus, motion). |
| `modern-web-design` | `C:/Users/HP/.config/opencode/skills/modern-web-design/SKILL.md` | Modern design principles: performance-first, bold minimalism, fluid typography, color systems (OKLCH), micro-interactions. | Supplemental; web-centric, limited terminal transfer. |
| `learn-taste` | `C:/Users/HP/.config/opencode/skills/learn-taste/SKILL.md` | Turns user design corrections into lasting rules (DESIGN.md / project skill / lint). | Process source for iterative redesign feedback. |
| `apple-design` | `C:/Users/HP/.config/opencode/skills/apple-design/SKILL.md` | Apple design language: response latency, direct manipulation, springs/momentum, materials, typography, reduced motion, design foundations. | Supplemental interaction/motion foundation source; web/gesture-centric. |

## 3. Skills selected — final design stack for the Nexus TUI redesign

| Role | Skill | Status | Rationale |
|---|---|---|---|
| Primary UX design | `ui-ux-pro-max` | Selected | Strongest overall design-intelligence capability (color, typography, layout, IA, interaction, accessibility, consistency, component design, pre-delivery checklist). Not terminal-native, but its general UX rules are directly applicable and its data-driven style/palette selection is valuable for a distinctive Nexus visual direction. |
| Design system | `hallmark` | Selected | Best available design-system creation process (tokens, states, responsive, gates, finish review, lint). Web-centric tooling, but its token/state/gate methodology is portable. Used as design-system methodology, adapted to terminal cells. |
| Design engineering / implementation polish | `emil-design-eng` | Selected | Best implementation-side design guidance (animation decision framework, interaction feedback, review format). Directly informs TUI feedback/focus/motion decisions. |
| Interaction foundations (supplemental) | `apple-design` | Selected (light) | Useful for feedback, restraint, spatial consistency, reduced motion, fluid interaction principles; subset applicable to keyboard-first TUIs. |
| Taste/process (supplemental) | `learn-taste` | Selected (light) | Method for capturing redesign corrections into lasting design rules. |
| Terminal-specific design | *(none found)* | **Missing** | No installed skill covers character-cell constraints, ratatui, terminal resizing, Windows Terminal, terminal color support, keyboard-first terminal UX. |

**Final design-skill stack:** `ui-ux-pro-max` (primary) + `hallmark` (design system) + `emil-design-eng` (implementation polish) + `apple-design` (interaction foundations, supplemental) + `learn-taste` (feedback loop, supplemental).

## 4. Selection reasoning

- The redesign brief demands UX auditing, design systems, information architecture, interaction design, visual hierarchy, color/typography/layout systems, keyboard-first interaction, terminal-native usability, responsive terminal layouts, accessibility/readability, developer-tooling UX, component design. No single installed skill covers all of this; the selected combination was chosen to collectively cover it.
- `ui-ux-pro-max` covers the audit/design-direction breadth; `hallmark` covers system/token/state discipline; `emil-design-eng` covers how it must feel/respond when implemented; `apple-design` adds interaction foundations; `learn-taste` manages iteration rules.
- No two selected skills duplicate the same capability in a conflicting way at the methodology level; their overlap (animation/interaction knowledge) is resolved by assigning animation/interaction to `emil-design-eng` as implementation authority and `ui-ux-pro-max`/`hallmark` for principles.
- Terminal-specific design knowledge is absent; this is the one material gap (see §6).

## 5. Overlapping capabilities

- Animation/interaction knowledge overlaps between `ui-ux-pro-max` (§7), `hallmark` (motion/reduced-motion), `emil-design-eng` (animation decision framework), and `apple-design` (springs/fluidity). Resolution: `emil-design-eng` is the implementation/decision authority for interaction behavior; `ui-ux-pro-max` and `hallmark` provide principle and system-level checks; `apple-design` informs foundational interaction quality.
- Design-system/token process overlaps between `hallmark` and `ui-ux-pro-max` (both produce design systems). Resolution: `hallmark` owns the design-system build/gates; `ui-ux-pro-max` owns UX strategy/search data; they are used complementarily, not competing.

## 6. Missing capability and installation status

- Missing: a terminal/TUI-specific design skill (character-cell geometry, ratatui/crossterm rendering constraints, terminal resize behavior, Windows Terminal compatibility, terminal color palettes, keyboard-only navigation design, terminal accessibility).
- Installation status: **not installed** (no new skills installed in Phase 0; nothing installed automatically).
- Recommendation (reported separately; awaiting owner approval): install a terminal/TUI design skill, e.g. via `npx skills find terminal` / `npx skills add <owner/repo@skill>` or author a custom skill via `npx skills init`, once an appropriate package is identified from a trustworthy source. No recommendation is being executed now.

## 7. Final design methodology for the Nexus TUI redesign

1. **Design direction** (Phase 2–3): synthesize `ui-ux-pro-max` style/product/color/typography outputs with the Nexus brief (developer-tool, keyboard-first, collaborative, terminal-native) into a Nexus-specific visual direction (dark graphite + restrained accent; to be validated by implementation review).
2. **Design system** (Phase 3): translate design tokens/states/gates from `hallmark`/`ui-ux-pro-max` into terminal-cell-based tokens (background, elevated, borders, accent, status colors, focus) adapted for ratatui.
3. **Screen design** (Phase 5): apply IA/navigation/layout/interaction guidance from `ui-ux-pro-max`; one-idea-per-screen and Floor-style invariants from `hallmark`; implementation polish from `emil-design-eng`.
4. **Interaction/motion** (Phase 6): `emil-design-eng` decision framework (no animation for keyboard-initiated actions; instant feedback; 1:1; interruptible) applied to TUI navigation/editor feedback.
5. **Review** (Phase 8): pre-delivery checklist from `ui-ux-pro-max`; design gates from `hallmark`; design-engineering review format from `emil-design-eng`.
6. **Iteration** (Phases 9–12): use `learn-taste` to convert owner/designer corrections into durable rules; `apple-design` for interaction foundations where applicable.

## 8. How each selected skill will be applied during the redesign

- `ui-ux-pro-max`: design-system generation (`--design-system`), targeted domain searches (color, typography, layout, navigation, accessibility, interaction), pre-delivery checklist, style/palette selection for Nexus identity.
- `hallmark`: design-system build, tokens, interaction-and-states, responsive, gates, finish review, `sloplint`-equivalent quality sweep adapted to TUI output.
- `emil-design-eng`: interaction decision framework and component polish reviews during implementation of each redesigned screen.
- `apple-design`: interaction foundations (feedback, spatial consistency, restraint, reduced motion) supplemental.
- `learn-taste`: capture corrections during redesign iterations and persist design rules.

## 9. Phase 0 deliverable status

- Created: `docs/design/NEXUS_TUI_DESIGN_SKILL_AUDIT.md` (this file).
- No application code modified during this phase.
- No redesign implemented yet.
- Phase 0 not yet signed off by project owner.

## 10. Approval gate

Phase 0 is complete on the documented basis above.

**Presented skill-selection summary for owner approval:**

> **Final design-skill stack for the Nexus TUI redesign:** `ui-ux-pro-max` (primary UX design intelligence) + `hallmark` (design system) + `emil-design-eng` (design engineering / interaction polish) + `apple-design` (interaction foundations, supplemental) + `learn-taste` (iteration rules, supplemental). Terminal-specific design capability: **not available** (reported as a gap; installation not performed).

The project owner must approve this stack before Phase 1 (current-state audit) begins.
