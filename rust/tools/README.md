# rust/tools — host-side tools

| Crate | Binary | Replaces | Run |
|---|---|---|---|
| `cookies` | `lumisound-refresh-cookies` | `ios-bridge/refresh_youtube_cookies.sh` | `cargo install --path rust/tools/cookies --root ~/.local`, then the systemd user units in `ios-bridge/` |
| `branding` | `lumisound-branding` | `ios/Branding/make_icon.py` + `render.sh` | `cargo run --release -p lumisound-branding` (`--svg-only` for just the SVGs) |

## To do
- [x] Cookie refresher installed 2026-11-23 (~/.local/bin, user units enabled, env
      ~/.config/lumisound-youtube-cookies.env). Drop a fresh `cookies.txt` export in ~/Downloads when the
      admin page or the alert says the shared jar is walled.
- [ ] Re-render the icons only when the design changes (the committed PNGs are current).
