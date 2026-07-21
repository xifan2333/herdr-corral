## Review
- **Correct:** Module split matches the intended herdr-sidebar shape (`host` / `theme` / `icons` / `feature` / `layout` / `ui` / `app`); bodies are still placeholders. Host parse never panics and degrades on bad JSON (`src/host.rs:71–100`). Theme mirrors Herdr tables with standalone `terminal` fallback (`src/theme.rs:51–72`). Activity half-block chips are properly encapsulated (`src/ui/activity.rs`). Pane title sync prefers `HERDR_PANE_ID` and debounces via `labeled_as` (`src/app.rs:107–122`). `cargo test` / `cargo check` compile; **0 unit tests**.

---

### Must-fix-now (framework correctness / API that will poison features)

| Sev | Location | Finding | Suggested fix |
|-----|----------|---------|---------------|
| **High** | `src/app.rs:49–57`, `280–288` | Terminal teardown is not panic/Drop-safe. `restore()` only runs on the happy path after `event_loop` returns. Panic, early `?`, or abort leaves raw mode + alt screen + mouse capture on. | Introduce a RAII guard (`struct TermGuard; impl Drop`) that always disables raw mode / mouse / alt screen; call `restore` from `Drop` (and keep explicit restore for normal exit). |
| **High** | `src/host.rs:33–44`, `96–99` + `src/app.rs:112–116` | **Pane-id semantics are muddled.** `LaunchContext.focused_pane_id` is filled from JSON `focused_pane_id` *or* `HERDR_PANE_ID` (`host.rs:96–99`). Those are different concepts: “pane focused at action time” vs “this plugin process’s pane”. App correctly re-reads `HERDR_PANE_ID` for rename, but the context field is a footgun for future preview/ctl (easy to rename or target the wrong pane). | Split into `pane_id` (always `HERDR_PANE_ID`) and `focused_pane_id` (JSON only). Never alias them. Document both on `LaunchContext`. |
| **Med** | `src/app.rs:117–122` + `src/herdr_cli.rs:11–23` | **`labeled_as` is set even when rename is a no-op or fails.** `set_pane_label` returns `()` and swallows all errors; app still sets `labeled_as = Some(title)`. Initial rename failure (herdr not ready, missing bin) permanently sticks until feature changes. | Return `bool`/`Result` from `set_pane_label`; only update `labeled_as` on success. Optionally retry once on next poll if still unlabeled. |
| **Med** | `src/app.rs:50` | **`set_current_dir` result is discarded.** Invalid/missing `ctx.cwd` leaves process cwd unchanged while UI still displays `ctx.cwd` — future relative FS work will silently root wrong. | Check result; on failure keep process cwd *and* record `cwd_error` / fall back to `current_dir()`, surface in footer/debug until Explorer exists. Prefer absolute paths from context for FS ops. |
| **Med** | *(gap)* host / theme / activity | **Zero unit tests** despite pure, testable surfaces (`parse_context` is even commented as such at `host.rs:112`). No lock on cwd priority, mode detect, feature cycle, hit tests, `parse_color`. | Add `#[cfg(test)]` for: JSON cwd precedence, empty-field filtering, `Feature::{next,prev,from_digit}`, `ui::hit`, a few `parse_color` cases. |

---

### Can-wait-until-feature (but plan before Explorer/SCM)

| Sev | Location | Finding | Suggested fix |
|-----|----------|---------|---------------|
| **High** *(pre-Explorer)* | `src/app.rs:125–143` | **Global key ownership:** `j`/`k`/`↑`/`↓`/`Tab` always cycle features. Explorer trees need those keys. No key-routing layer. | Before Explorer: introduce `FeatureView` trait / match on active feature with shell-only chords (`1`–`3`, maybe `Ctrl-1..`) and feature-local nav. Do not keep shell j/k once a body is focusable. |
| **Med** | `src/herdr_cli.rs:18–23` | **Blocking `Command::status` on the UI/event path.** Slow/hung `herdr` freezes the sidebar. Fully silent (by design for standalone). | Keep best-effort, but consider timeout (`std::process` + kill) or spawn+ignore; log failures behind `CORRAL_DEBUG` / `HERDR_PLUGIN_DEBUG`. |
| **Med** | `src/host.rs:122–128` | **cwd priority = `focused_pane_cwd` > `workspace_cwd`.** Sidebar may root at a random focused shell dir instead of workspace root. May match herdr-sidebar; risky for Explorer defaults. | Confirm product intent; if workspace-rooted, prefer `workspace_cwd` and keep focused pane cwd as secondary “reveal” path. |
| **Low** | `src/theme.rs:58–64` | `auto_switch` always takes `dark_name` (no host light/dark). Comment acknowledges it; light-mode Herdr users get wrong palette. | Later: read Herdr appearance if exposed, or env; until then document in README. |
| **Low** | `src/theme.rs:252` | Invalid custom color → `Color::Cyan` (Herdr parity). Silent bad config. | Keep parity; optional debug log for bad tokens. |
| **Low** | `src/app.rs:240–265` | Footer is **dev chrome** (mode/theme/feature/nf). Fine for scaffold; not product UI. | Gate behind debug or strip when Explorer ships. |
| **Low** | `src/lib.rs:18–25`, `theme.rs` constructors | Wide `pub` surface (`app`, `herdr_cli`, every palette fn, `parse_color`). Early-stage OK; hard to tighten later if anything depends on it. | Mark internal modules `pub(crate)` unless binary/plugin needs them; keep re-exports minimal (`Feature`, `LaunchContext`, `Palette`, `run`). |
| **Low** | `src/feature.rs:60–62` | `icon_double_width` is “any nerd font” — over-pads true Mono NF. Intentional vs herdr-sidebar. | Leave until visual QA; don’t special-case per glyph yet. |
| **Low** | `src/icons.rs:37–39` | `has_nerd_font()` re-detects every call; `app` correctly caches `detect()`. | Doc “call once”, or cache with `OnceLock`. |
| **Note** | `scripts/open-corral.sh:56–57` | Zoom on/off focus hack + `\|\| true` swallows open failures. Outside Rust framework but Herdr integration path. | Tighten when dock story stabilizes; surface open failures to user. |

---

### Not issues (intentional / verified)
- Half-block `▄`/`▀` chips in `ui/activity` — intentional TUI technique; app only passes data.
- No outer border/title in draw (`app.rs:168–171`) — correct; Herdr frames the pane.
- Silent CLI when standalone / missing ids — correct degrade path.
- Esc does not quit — documented herdr-sidebar parity.
- `force_color_output(true)` — deliberate against agent `NO_COLOR`.

---

### Residual risks
1. No panic-hook/Drop terminal restore → first real panic during feature work leaves a broken TTY.
2. Pane-id aliasing will mis-target control-file / preview / rename once multi-pane exists.
3. Feature-global `j`/`k` will force a key-routing rewrite at Explorer time if not designed now.
4. Untested host/theme contracts can regress silently (no CI signal beyond compile).
5. Theme `auto_switch` light path is effectively unimplemented.

---

## Acceptance
Review-only (no edits). Evidence from full `src/` read + `cargo test`/`cargo check`.