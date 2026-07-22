//! Pure launcher planning for the Unix `open-corral.sh` adapter.
//!
//! Herdr only splits right/down. To dock Corral on the left we pick the
//! layout's leftmost/topmost pane, split it to the right with a ratio that
//! targets ~32 columns, then swap the new pane into the left slot. All JSON
//! parsing and pane-id validation lives here so shell never invents policy or
//! passes untrusted ids into argv.

use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

use super::cli::SIDEBAR_TOKEN;

const TARGET_COLS: f64 = 32.0;
const HEARTBEAT_STALE_SECS: u64 = 20;

#[derive(Deserialize)]
struct PaneListMsg {
    result: PaneListResult,
}

#[derive(Deserialize)]
struct PaneListResult {
    #[serde(default)]
    panes: Vec<Pane>,
}

#[derive(Deserialize)]
struct Pane {
    pane_id: Option<String>,
    cwd: Option<String>,
    #[serde(default)]
    focused: bool,
    tab_id: Option<String>,
    #[serde(default)]
    tokens: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct LayoutMsg {
    result: LayoutResult,
}

#[derive(Deserialize)]
struct LayoutResult {
    layout: Layout,
}

#[derive(Deserialize)]
struct Layout {
    #[serde(default)]
    panes: Vec<LayoutPane>,
    #[serde(default)]
    splits: Vec<LayoutSplit>,
}

#[derive(Deserialize)]
struct LayoutPane {
    pane_id: Option<String>,
    rect: Option<Rect>,
}

#[derive(Deserialize)]
struct LayoutSplit {
    direction: Option<String>,
    ratio: Option<f64>,
    rect: Option<Rect>,
}

#[derive(Deserialize)]
struct Rect {
    x: i64,
    y: i64,
    width: i64,
}

fn strip_bom(input: &str) -> &str {
    input.trim_start_matches('\u{feff}')
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn heartbeat_fresh(pane: &Pane, now: u64) -> bool {
    let Some(value) = pane.tokens.get(SIDEBAR_TOKEN) else {
        return false;
    };
    let timestamp = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()));
    timestamp.is_some_and(|timestamp| now.saturating_sub(timestamp) <= HEARTBEAT_STALE_SECS)
}

/// `OPEN`, `FOCUS <id>`, or `REPLACE <id>`, scoped to a concrete focused tab.
/// Stable metadata identifies live Corral processes; the unique label detects
/// restored/dead panes whose TTL token has expired.
pub fn launch_decision(pane_list_json: &str) -> String {
    launch_decision_at(pane_list_json, now_secs())
}

fn launch_decision_at(pane_list_json: &str, now: u64) -> String {
    let Ok(msg) = serde_json::from_str::<PaneListMsg>(strip_bom(pane_list_json)) else {
        return "OPEN".into();
    };
    let Some(focused) = msg.result.panes.iter().find(|pane| pane.focused) else {
        return "OPEN".into();
    };
    let Some(tab_id) = focused.tab_id.as_deref().filter(|id| flag_safe(id)) else {
        return "OPEN".into();
    };
    let candidates: Vec<(&Pane, &str)> = msg
        .result
        .panes
        .iter()
        .filter(|pane| {
            pane.tab_id.as_deref() == Some(tab_id)
                // Labels are cosmetic and user-controlled. Only our metadata
                // token authorizes focus or destructive stale replacement.
                && pane.tokens.contains_key(SIDEBAR_TOKEN)
        })
        .filter_map(|pane| {
            pane.pane_id
                .as_deref()
                .filter(|id| flag_safe(id))
                .map(|id| (pane, id))
        })
        .collect();
    if let Some((_, id)) = candidates
        .iter()
        .find(|(pane, _)| heartbeat_fresh(pane, now))
    {
        return format!("FOCUS {id}");
    }
    candidates
        .first()
        .map(|(_, id)| format!("REPLACE {id}"))
        .unwrap_or_else(|| "OPEN".into())
}

/// `LIVE` when `pane_id` has a fresh identity heartbeat.
pub fn pane_live(pane_list_json: &str, pane_id: &str) -> String {
    let Ok(msg) = serde_json::from_str::<PaneListMsg>(strip_bom(pane_list_json)) else {
        return String::new();
    };
    msg.result
        .panes
        .iter()
        .find(|pane| pane.pane_id.as_deref() == Some(pane_id))
        .filter(|pane| heartbeat_fresh(pane, now_secs()))
        .map(|_| "LIVE".to_string())
        .unwrap_or_default()
}

/// `<pane_id>\t<cwd>` for the focused pane, or empty on invalid input.
pub fn focused_pane(pane_list_json: &str) -> String {
    let Ok(msg) = serde_json::from_str::<PaneListMsg>(strip_bom(pane_list_json)) else {
        return String::new();
    };
    let Some(pane) = msg.result.panes.iter().find(|pane| pane.focused) else {
        return String::new();
    };
    let Some(id) = pane.pane_id.as_deref().filter(|id| flag_safe(id)) else {
        return String::new();
    };
    format!("{id}\t{}", pane.cwd.as_deref().unwrap_or_default())
}

/// `<leftmost_pane_id>\t<ratio>` from `herdr pane layout` JSON.
/// Ratio targets 32 columns, clamped for very wide/narrow panes.
pub fn open_plan(layout_json: &str) -> String {
    let Ok(msg) = serde_json::from_str::<LayoutMsg>(strip_bom(layout_json)) else {
        return String::new();
    };
    let best = msg
        .result
        .layout
        .panes
        .iter()
        .filter_map(|pane| Some((pane.pane_id.as_deref()?, pane.rect.as_ref()?)))
        .filter(|(id, rect)| flag_safe(id) && rect.width > 0)
        .min_by_key(|(_, rect)| (rect.x, rect.y));
    let Some((id, rect)) = best else {
        return String::new();
    };
    let ratio = (TARGET_COLS / rect.width as f64).clamp(0.05, 0.9);
    format!("{id}\t{ratio:.4}")
}

/// `<direction>\t<ratio-delta>` to return an existing left Corral pane to 32
/// host-layout columns. The nearest innermost horizontal split whose divider
/// matches the pane's right edge owns that width.
pub fn resize_plan(layout_json: &str, pane_id: &str) -> String {
    resize_plan_to(layout_json, pane_id, TARGET_COLS)
}

/// Before splitting a narrow leftmost pane, grow it enough to hold a 32-column
/// Corral plus Herdr's minimum surviving right child (~4 columns). Without this
/// step a 32-column target splits into a 29+3 pair and cannot grow further once
/// the inner ratio reaches its 0.9 ceiling.
pub fn prepare_split_plan(layout_json: &str, pane_id: &str) -> String {
    let Ok(msg) = serde_json::from_str::<LayoutMsg>(strip_bom(layout_json)) else {
        return String::new();
    };
    let width = msg
        .result
        .layout
        .panes
        .iter()
        .find(|pane| pane.pane_id.as_deref() == Some(pane_id))
        .and_then(|pane| pane.rect.as_ref())
        .map(|rect| rect.width as f64);
    if width.is_none_or(|width| width >= TARGET_COLS + 4.0) {
        return String::new();
    }
    resize_plan_to(layout_json, pane_id, TARGET_COLS + 4.0)
}

fn resize_plan_to(layout_json: &str, pane_id: &str, target_cols: f64) -> String {
    let Ok(msg) = serde_json::from_str::<LayoutMsg>(strip_bom(layout_json)) else {
        return String::new();
    };
    let layout = &msg.result.layout;
    let Some(pane_rect) = layout
        .panes
        .iter()
        .find(|pane| pane.pane_id.as_deref() == Some(pane_id))
        .and_then(|pane| pane.rect.as_ref())
    else {
        return String::new();
    };
    let divider_x = pane_rect.x + pane_rect.width;
    let split = layout
        .splits
        .iter()
        .filter(|split| split.direction.as_deref() == Some("right"))
        .filter_map(|split| Some((split.rect.as_ref()?, split.ratio?)))
        .filter(|(rect, ratio)| {
            let divider = rect.x + (rect.width as f64 * ratio).round() as i64;
            rect.x <= pane_rect.x && (divider - divider_x).abs() <= 2 && rect.width > 0
        })
        .min_by_key(|(rect, _)| rect.width);
    let Some((split_rect, _)) = split else {
        return String::new();
    };
    if pane_rect.width == target_cols.round() as i64 {
        return String::new();
    }
    let delta = (target_cols - pane_rect.width as f64) / split_rect.width as f64;
    let direction = if delta > 0.0 { "right" } else { "left" };
    format!("{direction}\t{:.6}", delta.abs())
}

/// Pane id from a `pane split` response, validated before shell argv use.
pub fn split_pane_id(response_json: &str) -> String {
    #[derive(Deserialize)]
    struct Msg {
        result: ResultBody,
    }
    #[derive(Deserialize)]
    struct ResultBody {
        pane: Option<PaneInfo>,
    }
    #[derive(Deserialize)]
    struct PaneInfo {
        pane_id: Option<String>,
    }

    serde_json::from_str::<Msg>(strip_bom(response_json))
        .ok()
        .and_then(|msg| msg.result.pane)
        .and_then(|pane| pane.pane_id)
        .filter(|id| flag_safe(id))
        .unwrap_or_default()
}

fn flag_safe(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane_list(panes: &str) -> String {
        format!(r#"{{"result":{{"panes":[{panes}]}}}}"#)
    }

    fn layout(panes: &str) -> String {
        format!(r#"{{"result":{{"layout":{{"panes":[{panes}]}}}}}}"#)
    }

    #[test]
    fn launch_is_tab_scoped_and_heartbeat_driven() {
        let json = pane_list(
            r#"{"pane_id":"w:p1","focused":true,"tab_id":"w:t1"},
               {"pane_id":"w:p2","label":"Corral","tab_id":"w:t1","tokens":{"corral-sidebar":"95"}},
               {"pane_id":"w:p3","label":"Corral","tab_id":"w:t2","tokens":{"corral-sidebar":"95"}}"#,
        );
        assert_eq!(launch_decision_at(&json, 100), "FOCUS w:p2");
        let other = pane_list(
            r#"{"pane_id":"w:p1","focused":true,"tab_id":"w:t1"},
               {"pane_id":"w:p3","label":"Corral","tab_id":"w:t2","tokens":{"corral-sidebar":"95"}}"#,
        );
        assert_eq!(launch_decision_at(&other, 100), "OPEN");

        // Pane-list order must not let a stale candidate hide a later live one.
        let stale_before_live = pane_list(
            r#"{"pane_id":"w:p1","focused":true,"tab_id":"w:t1"},
               {"pane_id":"stale","label":"Corral","tab_id":"w:t1","tokens":{"corral-sidebar":"50"}},
               {"pane_id":"live","label":"Corral","tab_id":"w:t1","tokens":{"corral-sidebar":"95"}}"#,
        );
        assert_eq!(launch_decision_at(&stale_before_live, 100), "FOCUS live");
    }

    #[test]
    fn stale_owned_heartbeat_is_replaced_but_label_only_is_ignored() {
        let stale = pane_list(
            r#"{"pane_id":"w:p1","focused":true,"tab_id":"w:t1"},
               {"pane_id":"w:p2","label":"Corral","tab_id":"w:t1","tokens":{"corral-sidebar":"50"}}"#,
        );
        assert_eq!(launch_decision_at(&stale, 100), "REPLACE w:p2");
        let missing = pane_list(
            r#"{"pane_id":"w:p1","focused":true,"tab_id":"w:t1"},
               {"pane_id":"w:p2","label":"Corral","tab_id":"w:t1"}"#,
        );
        assert_eq!(launch_decision_at(&missing, 100), "OPEN");
    }

    #[test]
    fn missing_tab_ids_degrade_to_open() {
        let json = pane_list(
            r#"{"pane_id":"w:p1","focused":true},
               {"pane_id":"w:p2","label":"Corral","tokens":{"corral-sidebar":"95"}}"#,
        );
        assert_eq!(launch_decision_at(&json, 100), "OPEN");
    }

    #[test]
    fn unsafe_ids_degrade_safely() {
        let json = pane_list(
            r#"{"pane_id":"w:p1","focused":true,"tab_id":"w:t1"},
               {"pane_id":"--evil","label":"Corral","tab_id":"w:t1","tokens":{"corral-sidebar":"95"}}"#,
        );
        assert_eq!(launch_decision(&json), "OPEN");
    }

    #[test]
    fn plan_picks_leftmost_topmost_and_targets_columns() {
        let json = layout(
            r#"{"pane_id":"right","rect":{"x":90,"y":0,"width":90}},
               {"pane_id":"lower","rect":{"x":0,"y":40,"width":90}},
               {"pane_id":"left","rect":{"x":0,"y":0,"width":90}}"#,
        );
        assert_eq!(open_plan(&json), "left\t0.3556");
    }

    #[test]
    fn plan_clamps_ratio() {
        assert_eq!(
            open_plan(&layout(
                r#"{"pane_id":"p","rect":{"x":0,"y":0,"width":400}}"#
            )),
            "p\t0.0800"
        );
        assert_eq!(
            open_plan(&layout(
                r#"{"pane_id":"p","rect":{"x":0,"y":0,"width":40}}"#
            )),
            "p\t0.8000"
        );
    }

    #[test]
    fn resize_plan_restores_32_columns() {
        let json = r#"{"result":{"layout":{
            "panes":[{"pane_id":"corral","rect":{"x":0,"y":0,"width":50}}],
            "splits":[{"direction":"right","ratio":0.25,"rect":{"x":0,"y":0,"width":200}}]
        }}}"#;
        assert_eq!(resize_plan(json, "corral"), "left\t0.090000");

        let narrow = r#"{"result":{"layout":{
            "panes":[{"pane_id":"target","rect":{"x":0,"y":0,"width":32}}],
            "splits":[{"direction":"right","ratio":0.32,"rect":{"x":0,"y":0,"width":100}}]
        }}}"#;
        assert_eq!(prepare_split_plan(narrow, "target"), "right\t0.040000");

        // Even a one-column error matters in a wide owning split.
        let off_by_one = r#"{"result":{"layout":{
            "panes":[{"pane_id":"corral","rect":{"x":0,"y":0,"width":33}}],
            "splits":[{"direction":"right","ratio":0.0825,"rect":{"x":0,"y":0,"width":400}}]
        }}}"#;
        assert_eq!(resize_plan(off_by_one, "corral"), "left\t0.002500");
    }

    #[test]
    fn split_response_is_validated() {
        assert_eq!(
            split_pane_id(r#"{"result":{"pane":{"pane_id":"w:p9"}}}"#),
            "w:p9"
        );
        assert_eq!(
            split_pane_id(r#"{"result":{"pane":{"pane_id":"--bad"}}}"#),
            ""
        );
    }
}
