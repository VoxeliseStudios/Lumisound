//! Per-route response times for /admin: count, errors and p50/p95/p99 per
//! matched route over the last hour (12 five-minute slots) and since start.
//! Recording is one short mutex hold per request (a bucket increment).

use axum::body::Body;
use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Upper bounds (ms) of the latency buckets; the last bucket is open.
const BOUNDS: [f64; 16] = [1.0, 2.0, 5.0, 10.0, 20.0, 35.0, 50.0, 75.0, 100.0, 200.0, 350.0, 500.0, 1000.0, 2000.0, 5000.0, 15000.0];
const SLOT_SECS: u64 = 300;
const SLOTS: usize = 12;

#[derive(Clone, Default)]
struct Hist {
    n: u64,
    errors: u64,
    sum_ms: f64,
    max_ms: f64,
    buckets: [u64; BOUNDS.len() + 1],
}

impl Hist {
    fn add(&mut self, ms: f64, error: bool) {
        self.n += 1;
        self.errors += error as u64;
        self.sum_ms += ms;
        self.max_ms = self.max_ms.max(ms);
        let i = BOUNDS.iter().position(|b| ms <= *b).unwrap_or(BOUNDS.len());
        self.buckets[i] += 1;
    }

    fn merge(&mut self, o: &Hist) {
        self.n += o.n;
        self.errors += o.errors;
        self.sum_ms += o.sum_ms;
        self.max_ms = self.max_ms.max(o.max_ms);
        for (a, b) in self.buckets.iter_mut().zip(o.buckets.iter()) {
            *a += b;
        }
    }

    /// Bucket upper bound holding quantile `q` (the open bucket reports max).
    fn quantile(&self, q: f64) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        let target = (q * self.n as f64).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (i, c) in self.buckets.iter().enumerate() {
            seen += c;
            if seen >= target {
                return BOUNDS.get(i).copied().unwrap_or(self.max_ms).min(self.max_ms);
            }
        }
        self.max_ms
    }
}

#[derive(Default)]
struct Route {
    total: Hist,
    /// (slot number, histogram) ring for the rolling hour.
    slots: Vec<(u64, Hist)>,
}

struct Stats {
    started: Instant,
    routes: HashMap<String, Route>,
}

fn stats() -> &'static Mutex<Stats> {
    static S: OnceLock<Mutex<Stats>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Stats { started: Instant::now(), routes: HashMap::new() }))
}

fn record(route: String, ms: f64, error: bool) {
    let mut s = stats().lock().unwrap();
    let slot = s.started.elapsed().as_secs() / SLOT_SECS;
    let r = s.routes.entry(route).or_default();
    r.total.add(ms, error);
    match r.slots.iter_mut().find(|(n, _)| *n == slot) {
        Some((_, h)) => h.add(ms, error),
        None => {
            r.slots.retain(|(n, _)| slot - *n < SLOTS as u64);
            let mut h = Hist::default();
            h.add(ms, error);
            r.slots.push((slot, h));
        }
    }
}

/// Middleware: time every request under its matched route ("GET /api/search").
/// Streaming bodies count until the handler returns (time to first byte).
pub async fn track(req: Request<Body>, next: Next) -> Response {
    let route = req.extensions().get::<MatchedPath>().map(|m| m.as_str().to_string());
    let method = req.method().clone();
    let start = Instant::now();
    let res = next.run(req).await;
    if let Some(r) = route {
        record(format!("{method} {r}"), start.elapsed().as_secs_f64() * 1000.0, res.status().is_server_error());
    }
    res
}

fn row(name: &str, h: &Hist) -> Value {
    json!({
        "route": name,
        "n": h.n,
        "errors": h.errors,
        "avg_ms": if h.n > 0 { (h.sum_ms / h.n as f64 * 10.0).round() / 10.0 } else { 0.0 },
        "p50_ms": h.quantile(0.50),
        "p95_ms": h.quantile(0.95),
        "p99_ms": h.quantile(0.99),
        "max_ms": (h.max_ms * 10.0).round() / 10.0,
    })
}

/// Routes by total time spent (n × avg) over the last hour, plus since start.
pub fn report(limit: usize) -> Value {
    let s = stats().lock().unwrap();
    let slot = s.started.elapsed().as_secs() / SLOT_SECS;
    let mut hour: Vec<(String, Hist)> = Vec::new();
    let mut total: Vec<(String, Hist)> = Vec::new();
    for (name, r) in &s.routes {
        let mut h = Hist::default();
        for (n, sh) in &r.slots {
            if slot - *n < SLOTS as u64 {
                h.merge(sh);
            }
        }
        if h.n > 0 {
            hour.push((name.clone(), h));
        }
        total.push((name.clone(), r.total.clone()));
    }
    hour.sort_by(|a, b| b.1.sum_ms.total_cmp(&a.1.sum_ms));
    total.sort_by(|a, b| b.1.sum_ms.total_cmp(&a.1.sum_ms));
    json!({
        "uptime_secs": s.started.elapsed().as_secs(),
        "last_hour": hour.iter().take(limit).map(|(n, h)| row(n, h)).collect::<Vec<_>>(),
        "since_start": total.iter().take(limit).map(|(n, h)| row(n, h)).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_from_buckets() {
        let mut h = Hist::default();
        for ms in [0.5, 3.0, 4.0, 8.0, 9.0, 15.0, 30.0, 40.0, 90.0, 1500.0] {
            h.add(ms, false);
        }
        assert_eq!(h.n, 10);
        assert_eq!(h.quantile(0.5), 10.0);
        assert_eq!(h.quantile(0.95), 1500.0);
        assert!(h.quantile(0.1) <= 1.0);
    }

    #[test]
    fn report_lists_recorded_routes() {
        record("GET /test/route".into(), 12.0, false);
        record("GET /test/route".into(), 600.0, true);
        let r = report(500);
        let hour = r["last_hour"].as_array().unwrap();
        let row = hour.iter().find(|r| r["route"] == "GET /test/route").unwrap();
        assert_eq!(row["n"], 2);
        assert_eq!(row["errors"], 1);
    }
}
