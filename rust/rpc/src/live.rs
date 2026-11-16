//! Push updates from the bridge's per-user live socket (`/ws/live`).
//!
//! The bridge sends a `playback_state` event whenever what the phone plays
//! visibly changes (track, play/pause, seek, art), so the daemon reacts at
//! once instead of polling every second. A background thread keeps the
//! socket up (pinging every 25 s, reconnecting with backoff) and forwards
//! each state to the main loop; while it is down the daemon falls back to
//! polling.

use serde_json::Value;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// Close code the bridge uses for a revoked/expired session.
const UNAUTHORIZED: u16 = 4401;
const PING_EVERY: Duration = Duration::from_secs(25);

pub enum Event {
    State(Value),
    /// The session was refused: stop trying (polling reports the 401).
    Unauthorized,
}

/// `https://host` → `wss://host/ws/live?token=…`.
pub fn live_url(bridge_base: &str, token: &str) -> String {
    let base = bridge_base.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    let enc: String = token.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("{ws}/ws/live?token={enc}")
}

/// The playback state in a live message, if it is one.
pub fn parse(text: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(text).ok()?;
    (v["type"] == "playback_state").then(|| v["state"].clone()).filter(Value::is_object)
}

fn set_timeout(ws: &WebSocket<MaybeTlsStream<TcpStream>>, d: Duration) {
    let tcp = match ws.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => &s.sock,
        _ => return,
    };
    let _ = tcp.set_read_timeout(Some(d));
}

fn is_timeout(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

/// Run one connection until it drops. Ok(true) = refused (unauthorized).
fn session(url: &str, tx: &Sender<Event>, up: &AtomicBool) -> Result<bool, String> {
    let (mut ws, _) = tungstenite::connect(url).map_err(|e| e.to_string())?;
    set_timeout(&ws, Duration::from_secs(5));
    up.store(true, Ordering::Relaxed);
    crate::log("Live updates connected");
    let (mut last_ping, mut last_heard) = (Instant::now(), Instant::now());
    let r = loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                last_heard = Instant::now();
                if let Some(state) = parse(t.as_str())
                    && tx.send(Event::State(state)).is_err() {
                        break Ok(false);
                    }
            }
            Ok(Message::Close(frame)) => {
                break Ok(frame.is_some_and(|f| u16::from(f.code) == UNAUTHORIZED));
            }
            Ok(_) => last_heard = Instant::now(),
            Err(e) if is_timeout(&e) => {}
            Err(e) => break Err(e.to_string()),
        }
        if last_heard.elapsed() > PING_EVERY * 3 {
            break Err("no reply from the bridge".into());
        }
        if last_ping.elapsed() >= PING_EVERY {
            last_ping = Instant::now();
            if let Err(e) = ws.send(Message::Ping(Default::default()))
                && !is_timeout(&e) {
                    break Err(e.to_string());
                }
        }
    };
    up.store(false, Ordering::Relaxed);
    r
}

/// Start the background connection. `up` tells the main loop whether pushes
/// are flowing (it polls slowly while they are).
pub fn spawn(bridge_base: &str, token: &str, tx: Sender<Event>) -> Arc<AtomicBool> {
    let up = Arc::new(AtomicBool::new(false));
    let (url, flag) = (live_url(bridge_base, token), up.clone());
    std::thread::Builder::new()
        .name("lumisound-live".into())
        .spawn(move || {
            let mut backoff = Duration::from_secs(2);
            loop {
                let started = Instant::now();
                match session(&url, &tx, &flag) {
                    Ok(true) => {
                        crate::log("Live updates refused (session revoked or expired); polling only");
                        let _ = tx.send(Event::Unauthorized);
                        return;
                    }
                    Ok(false) if started.elapsed() < Duration::from_secs(1) => {}
                    Ok(false) => crate::log("Live updates closed; reconnecting"),
                    Err(e) => crate::log(&format!("Live updates unavailable ({e}); polling until they're back")),
                }
                // A connection that lasted a while starts the backoff over.
                if started.elapsed() > Duration::from_secs(60) {
                    backoff = Duration::from_secs(2);
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }
        })
        .ok();
    up
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_and_parse() {
        assert_eq!(live_url("https://b.example/", "a.b+c"), "wss://b.example/ws/live?token=a.b%2Bc");
        assert_eq!(live_url("http://127.0.0.1:8002", "t"), "ws://127.0.0.1:8002/ws/live?token=t");
        let s = parse(r#"{"type":"playback_state","state":{"title":"S","is_playing":true}}"#).unwrap();
        assert_eq!(s["title"], "S");
        assert!(parse(r#"{"type":"presence","user_id":"x"}"#).is_none());
        assert!(parse("not json").is_none());
    }
}
