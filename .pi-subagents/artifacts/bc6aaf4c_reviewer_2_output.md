## Review

Scaffold is small and readable (~1.3k LOC). Module docs are good. Main risks before Explorer/SCM: **theme weight**, **dependency bloat**, **`app` as god-module**, and **premature public surface**.

---

### Correct (keep)
- **Sidebar-only shape** is consistent: `src/app.rs`, `src/layout.rs`, `README.md`, `herdr-plugin.toml` all agree one left pane + later preview.
- **Activity chips** correctly isolated in `src/ui/activity.rs` (half-blocks not leaking into app).
- **Host degradation** in `src/host.rs` is solid (no panic on bad env/JSON).
- **`herdr_cli`** is thin and best-effort (`src/herdr_cli.rs:11-24`).
- **Binary thin** (`src/main.rs` → `corral::run()`).
- **Release profile** is lean (`Cargo.toml` opt-level z / LTO / strip); binary ~1.1M.

---

### P0 — do before Explorer/SCM land

1. **Carve feature view boundary out of `app`**
   - Evidence: `src/app.rs:224-233` hardcodes Explorer/SCM/GitHub placeholders; `draw_body` / `handle_key` / `State` will balloon.
   - Action: introduce `FeatureView` (or similar) with `id`, `title`, `icon`, `draw`, `handle_key/mouse` before any real tree/git code.
   - Why now: retrofitting later means rewriting `app` while features grow.

2. **Shrink / fence `theme` (39% of tree)**
   - Evidence: `src/theme.rs` 525 lines; used tokens today are only `panel_bg`, `text`, `subtext0`, `overlay1`, `surface1`, `name` (`app.rs`, `ui/activity.rs`).
   - Dead/over-public API:
     - `TOKEN_NAMES` (`theme.rs:105`) — unused
     - `Palette::token` (`theme.rs:81-101`) — unused
     - `Palette::named` / free `from_name` + 17 `pub fn` palettes (`theme.rs:299-528`) — only needed for resolve
     - `Serialize` on `Palette` (`theme.rs:28`) — never serialized; enables `ratatui` `serde` feature
   - Action: keep `Palette::resolve` + private tables; make palette constructors/`parse_color` crate-private; drop unused helpers; consider `theme/palettes.rs` only if tables stay.

3. **Revisit `has-nerd-font` dependency weight**
   - Evidence: `Cargo.toml` dep; tree pulls `clap`, `pest*`, `serde_json5`, second `toml` line for env detection (`src/icons.rs:31-34`).
   - App only needs `should_use_icons()` / `available` (`app.rs:67`, `248-251`).
   - Action: replace with small env heuristic (`TERM_PROGRAM`, `NERD_FONT`, known terminals) **or** feature-gate; drop unused fields/`has_nerd_font()` reexport.

---

### P1 — structure / naming / API hygiene

4. **Trim dead host fields until needed**
   - Evidence: `LaunchContext` stores `workspace_id`, `tab_id`, `plugin_id`, `entrypoint_id` (`host.rs:37-41`) and `is_plugin()` (`48-50`) — never read outside host.
   - Action: keep `mode`, `cwd`, `focused_pane_id`, `herdr_bin`; parse rest later when SCM/GitHub need them.

5. **Narrow crate public surface**
   - Evidence: `lib.rs:18-30` reexports `Feature`, `LaunchContext`, `Mode`, `NerdFontSupport`, `has_nerd_font`, `Palette`; every module is `pub`. Only consumer is own bin.
   - Action: `pub(crate)` internals; export only `run` (and maybe `Feature` later for tests).

6. **README ↔ code drift**
   - Evidence: README module table lists `host/theme/icons/feature/layout/app` but omits `ui` and `herdr_cli` (real modules in `src/`).
   - README says “workbench plugin” vibe while code is sidebar-only (OK if wording stays “left sidebar”).
   - Action: update table; document open script / python3 need for `scripts/open-corral.sh`.

7. **Move feature chrome off `Feature` enum if views own it**
   - Evidence: `feature.rs` owns icons + digit keys + titles; `icon_double_width` is just `nerd_font` (`feature.rs:60-62`) — not per-glyph.
   - Action: either real per-icon width, or drop the flag and always slack when NF on; let each view supply icon metadata.

8. **Add focused unit tests now (cheap wins)**
   - Evidence: `cargo test` → 0 lib/bin tests; pure parsers exist (`host::parse_context` is private, `theme` resolve/parse, `Feature` nav).
   - Action: tests for feature cycle, color parse, context JSON fallback — before tree I/O makes testing hard.

9. **`open-corral.sh` complexity / host coupling**
   - Evidence: python JSON scrape + zoom on/off focus hack (`scripts/open-corral.sh:20-57`).
   - Action: document as host workaround; if Herdr gains left-dock, delete swap/zoom; avoid growing more UI policy in bash.

---

### P2 — polish / later

10. **Footer is debug chrome** (`app.rs:247-257`: mode, palette name, nf?) — gate behind debug or strip for product UI.
11. **`layout` is one helper** (`layout.rs:13-24`) — fine; fold into `ui` if no second layout appears.
12. **Naming:** `Scm` vs title `"Source Control"` vs id `"scm"` is OK; avoid renaming mid-feature work.
13. **`.gitignore`:** no `progress.md`/`plan.md` ignore (files absent now); add if used as local scratch.
14. **Ctl/preview protocol** not present — good; land as `preview` / `ctl` module, not inside feature views.

---

### Suggested target module map (next phase)

```text
src/
  main.rs                 # bin only
  lib.rs                  # pub fn run(); minimal reexports

  host/
    mod.rs                # Mode, LaunchContext (slim), from_env
    context.rs            # JSON/env parse (testable)

  herdr/
    mod.rs
    cli.rs                # pane rename, future ctl/preview calls

  theme/
    mod.rs                # Palette, resolve()
    parse.rs              # parse_color (crate-private)
    palettes.rs           # built-ins (private tables)

  icons.rs                # light NF detect (or host/icons)

  shell/                  # sidebar chrome (rename from app+layout+ui)
    mod.rs                # run event loop
    layout.rs             # split_sidebar
    activity.rs           # chips + hit test
    footer.rs             # optional/debug

  feature/
    mod.rs                # FeatureId enum + registry
    view.rs               # trait FeatureView { draw, on_key, on_click }
    explorer/
      mod.rs
      tree.rs             # later
    scm/
      mod.rs             # later
    github/
      mod.rs             # later

  preview/                # later: ctl protocol + open preview pane
    mod.rs
```

**Boundary rules**
- `shell` never imports git/fs details — only `dyn FeatureView` / registry.
- Features never call crossterm setup or Herdr env detection.
- `herdr` is the only process-spawn boundary.
- `theme` is data-only (no UI widgets).
- Preview/ctl stays out of feature modules.

---

### Residual risks
- Ported Herdr theme tables will drift from Herdr upgrades (manual port, `theme.rs` header cites v0.7.4).
- Silent `herdr_cli` / open-script failures hard to debug in plugin mode.
- No automated UI/render tests; activity hit geometry is easy to regress.
- If `has-nerd-font` stays, compile graph stays heavier than the rest of the app warrants.

---

### Note
- `plan.md` / `progress.md` were missing at review time (ENOENT); review used tree + sources only.
- Review-only: no files edited.