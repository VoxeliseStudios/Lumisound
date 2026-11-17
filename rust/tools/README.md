# rust/tools — host-side tools

| Crate | Binary | Replaces | Run |
|---|---|---|---|
| `cookies` | `lumisound-refresh-cookies` | `ios-bridge/refresh_youtube_cookies.sh` | `cargo install --path rust/tools/cookies --root ~/.local`, then the systemd user units in `ios-bridge/` |
| `branding` | `lumisound-branding` | `ios/Branding/make_icon.py` + `render.sh` | `cargo run --release -p lumisound-branding` (`--svg-only` for just the SVGs) |

## To do
- [ ] Install the cookie refresher + units on the host if YouTube cookies need automatic refresh
      (not installed as of 2026-11-16).
- [ ] Re-render the icons only when the design changes (the committed PNGs are current).
