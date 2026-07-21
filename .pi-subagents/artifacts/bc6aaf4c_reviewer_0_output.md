## Review
- Correct: post-pivot shape matches herdr-sidebar intent — one pane, in-process feature switch, preview deferred (`herdr-plugin.toml:13-19`, `src/lib.rs:3-7`, `src/layout.rs:3-5`, `src/app.rs:3-6`). Left/right workbench split, `Focus`, `PanelView` are gone (commit `a79b91b`).
- Correct: shell modules are mostly right-sized for identity/host/theme/icons/activity (`feature`, `host`, `icons`, `ui/activity`, `herdr_cli`). Plugin entry is a single `sidebar` pane + dock script.
- Correct: code compiles; lib has 0 tests but builds clean.

### Findings (architecture & shape)

| Sev | Where | Finding | Why it matters for Explorer growth |
|---|---|---|---|
| **should-fix** | `src/app.rs:126-144`, `138-141` | Shell keys and future body keys share one `handle_key`. `j`/`k`/`Tab` always cycle **features**. | Explorer needs `j`/`k` for tree rows. Without activity-vs-body focus (or digit-only feature switch), first real feature will fight the shell. |
| **should-fix** | `src/app.rs:31-45`, `190-233` | No feature mount point. `State` is shell-only; body is a `match feature` placeholder in `app`. | Next modules will dump tree/selection/scroll into `app.rs` and recreate a workbench god-object. Need `FeatureView` (or per-feature state + draw/key handlers) **before** Explorer code lands. |
| **should-fix** | `src/host.rs:39`, `96-99`; `src/app.rs:112-116` | Pane identity is muddled: `focused_pane_id` is filled from context **and** from `HERDR_PANE_ID`; rename re-reads env and falls back to “focused”. | Launch JSON “focused” is often the **neighbor** before/after dock. Preview/ctl + rename need explicit `self_pane_id` vs `neighbor_pane_id`. |
| **should-fix** | `src/main.rs:6-8` | Binary is only `corral::run()` — no subcommand surface. | README already plans `preview` + ctl (`README.md:49-50`). Without `corral [sidebar\|preview\|…]` now, preview will either fork another binary or force awkward env flags. |
| **should-fix** | repo-wide tests | `cargo test --lib` → **0 tests**. `host::parse_context` is marked testable but private/untested (`host.rs:115`). | Pure pieces (Feature cycle, hit tests, layout split, theme parse, host JSON) should lock the shell before Explorer churn. |
| **should-fix** | `src/app.rs:173`, `237-263`; `199-215` | Debug footer always on; body re-draws feature title while pane is also renamed (`107-123`). | Permanent −1–2 rows in a narrow sidebar; double title is workbench leftover UX, not herdr-sidebar density. |
| **nit** | `Cargo.toml:5`, `src/main.rs:4`, `src/feature.rs:1,7` | Stale “workbench” wording after sidebar pivot. | Mis-teaches shape; new feature PRs will reintroduce left+right thinking. Plugin TOML/README already say sidebar. |
| **nit** | `src/theme.rs` (~525 / ~1360 src lines) | Theme port dominates the crate vs shell surface. | Not wrong for Herdr parity, but module gravity is inverted until Explorer/SCM exist. Keep as leaf; don’t route feature code through it. |
| **nit** | `scripts/open-corral.sh:54-58` | Focus restored via zoom on/off after swap. | Dock path is host-fragile; acceptable v1, but not a substitute for stable pane identity inside the binary. |
| **note** | `src/layout.rs:13-24` | Layout is correctly sidebar-only (activity \| body \| footer). | Keep feature-local layouts out of this module (tree indent, section headers live under feature views). |
| **note** | `src/ui/activity.rs` | Activity chips/hit-tests encapsulated; good shell boundary. | Keep mouse hits on the strip; don’t overload body clicks into `nav_hits`. |

No **blockers** for “framework mostly OK” — binary shape is already sidebar-only. The real risk is **missing shell contracts** (focus/keys, feature trait, pane ids, CLI entry) right before Explorer.

### Top 5 cleanups worth doing NOW (before Explorer)

1. **Key routing / focus zones** — Shell owns `1`/`2`/`3` (and maybe activity-only `Tab`); body owns `j`/`k`/arrows. Stop feature-cycling on list-nav keys (`app.rs:138-141`).
2. **`FeatureView` (or equivalent) + per-feature state** — `draw`/`handle_key`/`on_activate` behind a small trait; `app` only switches active view. Kill the body `match` in `draw_body`.
3. **Normalize pane identity in `LaunchContext`** — `self_pane_id` (`HERDR_PANE_ID`) vs launch `focused_pane_id` / neighbor; single place used by rename and future preview open.
4. **CLI entry skeleton** — e.g. default = sidebar TUI; reserve `preview` (and later ctl) so the binary stays one artifact without reshaping `main` mid-feature.
5. **Shell polish that free vertical space + lock contracts** — hide or gate debug footer; drop redundant in-body title once rename works; add unit tests for host parse, Feature ids, layout split, activity hit math; scrub remaining “workbench” strings.

---