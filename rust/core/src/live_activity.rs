//! Live Activity telemetry: counters for everything the app's
//! `LiveActivityManager` and the Live Activity / Control intents do, plus the
//! marks the widget extension leaves in the App Group (it can't reach the
//! network, and doesn't link this core). Summarised into one
//! `live_activity_summary` event; the rare important moments (start failed,
//! dismissed, Live Activities turned off) are sent at once by the app.
//!
//! Counters persist (store namespace `live_activity_counts`), so activity in
//! the background — where most Live Activity trouble happens — survives the
//! app being killed before the next summary.

use crate::store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

const NS: &str = "live_activity_counts";
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize, Default)]
struct Counts {
    since: f64,
    counts: BTreeMap<String, u64>,
}

fn bump(kind: &str, by: u64) {
    if kind.is_empty() || by == 0 {
        return;
    }
    let _g = LOCK.lock().unwrap();
    let mut c: Counts = store::load(NS);
    if c.since == 0.0 {
        c.since = store::now();
    }
    *c.counts.entry(kind.to_string()).or_default() += by;
    store::save(NS, &c);
}

/// Count one occurrence of `kind` (e.g. "started", "delivered_bg", "intent.next").
#[uniffi::export]
pub fn live_activity_count(kind: String) {
    bump(&kind, 1);
}

/// Folds the widget extension's marks file into the counters and deletes it.
/// Lines are "<unix seconds> <kind> <key>"; each kind counts its lines as
/// "widget.<kind>" and its distinct keys as "widget.<kind>.distinct".
#[uniffi::export]
pub fn live_activity_ingest_marks(path: String) {
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let _ = std::fs::remove_file(&path);
    let mut lines: BTreeMap<String, u64> = BTreeMap::new();
    let mut keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in text.lines() {
        let mut it = line.splitn(3, ' ');
        let (Some(ts), Some(kind)) = (it.next(), it.next()) else { continue };
        if ts.parse::<f64>().is_err() || kind.is_empty() || kind.len() > 40 {
            continue;
        }
        *lines.entry(kind.to_string()).or_default() += 1;
        keys.entry(kind.to_string()).or_default().insert(it.next().unwrap_or("").to_string());
    }
    for (kind, n) in lines {
        bump(&format!("widget.{kind}"), n);
        bump(&format!("widget.{kind}.distinct"), keys.get(&kind).map_or(0, |k| k.len() as u64));
    }
}

/// JSON {"periodSeconds": …, "counts": {…}} of everything counted since the
/// last summary, then starts over. None when nothing was counted.
#[uniffi::export]
pub fn live_activity_take_summary() -> Option<String> {
    let _g = LOCK.lock().unwrap();
    let c: Counts = store::load(NS);
    if c.counts.values().all(|v| *v == 0) {
        return None;
    }
    store::save(NS, &Counts::default());
    let period = if c.since > 0.0 { (store::now() - c.since).max(0.0).round() } else { 0.0 };
    Some(serde_json::json!({"periodSeconds": period, "counts": c.counts}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_marks_and_summary() {
        let (_g, d) = store::test_store();
        live_activity_count("started".into());
        live_activity_count("started".into());
        live_activity_count("delivered_bg".into());
        let marks = d.join("LiveActivityMarks.log");
        std::fs::write(&marks, "1700000000 self_advance a->b\n1700000001 self_advance a->b\n1700000002 self_advance b->c\njunk\n1 finished x\n").unwrap();
        live_activity_ingest_marks(marks.to_string_lossy().into_owned());
        assert!(!marks.exists());
        let s: serde_json::Value = serde_json::from_str(&live_activity_take_summary().unwrap()).unwrap();
        let c = &s["counts"];
        assert_eq!((c["started"].as_u64(), c["delivered_bg"].as_u64()), (Some(2), Some(1)));
        assert_eq!((c["widget.self_advance"].as_u64(), c["widget.self_advance.distinct"].as_u64()), (Some(3), Some(2)));
        assert_eq!(c["widget.finished"].as_u64(), Some(1));
        assert!(live_activity_take_summary().is_none());
    }
}
