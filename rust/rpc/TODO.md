# rust/rpc — to do

Discord Rich Presence daemon (`lumisound-discord-rpc`). History: [`RUST-PORT.md`](../../RUST-PORT.md).

## Done (2026-11-16) — v2.2.0
- Live updates: `src/live.rs` listens on the bridge's `/ws/live` (same session token) for
  `playback_state` events, which the bridge sends when the phone's state visibly changes (track,
  play/pause, seek > 2 s, art/engine text). The card updates at once; HTTP polling drops to every 30 s
  (keeps `updated_at` fresh, catches misses) and goes back to `poll_interval_seconds` while the socket is
  down. Pings every 25 s, reconnect with backoff (2 s → 60 s), stops on close code 4401 (revoked).
- Discord card: `status_display_type` (member list shows "Listening to <song>"; config `status_display`
  = "title" | "artist" | "app") and `details_url` (title links to the track). If Discord answers ERROR
  the card is re-sent without them for the rest of the session (`discord_new_fields: false` forces it).
- SET_ACTIVITY replies are matched by nonce and errors reported; Discord's socket is retried with
  backoff (1 s → 30 s) instead of scanning every candidate path each second.
- `--version`; daily check for a newer `rpc/v*` release (`check_updates: false` turns it off).
- The app's engine line now carries LMS3 bitrate and "Premium" (card's third line).
- Own version (2.2.0) instead of the workspace's.

## Done (2026-11-11)
- Third card line (`large_text`) shows engine/format/BPM instead of repeating the title.
- YouTube art goes through the bridge's `/api/rp-art/yt/` copy so 16:9 frames aren't cropped to the middle.
- Start + end timestamps (ms): Discord draws the progress bar for the Listening card.
- Re-sends immediately on any text/art change or a seek (> 1.5 s jump); refresh every ~13.5 s otherwise.

## Open
- `state_url` (artist link) once playback state carries an artist URL.
- Windows/macOS: live socket verified on Linux only (same rustls/tungstenite code on all three).
