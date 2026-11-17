# Lumisound Discord Rich Presence

A small daemon that mirrors your Lumisound "now playing" state to your
Discord profile via Rich Presence. It is one self-contained Rust binary,
`lumisound-discord-rpc` (source: [`rust/rpc`](../rust/rpc)) — no Python,
shell or PowerShell scripts needed: the binary installs itself
(`lumisound-discord-rpc install`). Get it from the Lumisound GitHub releases
(`rpc/v*` tags) or build it with `cargo build --release -p lumisound-rpc`.

## Why this can't be fully server-side

Discord's Rich Presence (the "Playing/Listening to..." card on a profile) is
only settable over Discord's **local IPC** — a Unix socket (or, on Windows, a
named pipe) that the Discord desktop client opens on the machine it's running
on. There is no Discord API that lets a remote server set another account's
Rich Presence directly; the only first-party way is this local connection.

So **something has to run next to your Discord desktop client** — there's no
way around that part. What Lumisound *does* centralize is everything else:
your Discord Application Client ID, Rich Presence art, and the on/off toggle
all live on the Lumisound server (`/user/discord-rpc-config`, set from
**Account → Discord Rich Presence**) and the daemon fetches them
automatically. Locally you just **sign in and run one script** — see below.

## What the card shows

- **Title** and **by Artist** on the first two lines.
- **Third line**: how it is playing — engine, format, sample rate and BPM (for example
  `Rust engine · LMS OPUS · 48 kHz · 124 BPM`). It used to repeat the title.
- **Artwork**: Discord always draws a square. Music-video thumbnails (16:9) are shown whole on a blurred fill
  instead of being cropped to their middle; square album art fills the square.
- **Progress bar** with elapsed and total time, re-synced immediately after a seek or track change.

## 1. Register your Discord app (once)

In the app, go to **Account → Discord Rich Presence** and enter a **Discord
Application Client ID** (create one for free at
https://discord.com/developers/applications — only the name/icon matter), plus
optionally a Rich Presence art asset name, then save. This is stored
server-side, so the daemon picks it up automatically — nothing to copy into a
config file.

That's all you need here. You do **not** have to generate a token by hand
anymore — the installer signs in for you.

## 2. Install and run the daemon

Put the binary somewhere permanent (the service runs it from that path), for
example `~/.local/bin/lumisound-discord-rpc` (or `%LOCALAPPDATA%\Programs\`
on Windows), then:

```sh
lumisound-discord-rpc install
```

Sign in with your **Lumisound email (or username) and password** when
prompted. Everything else has a sensible default (the hosted Lumisound
bridge) or comes from your server-side registration. `install` then registers
the daemon to start with your session:

| Platform | Registered as | Check it |
|---|---|---|
| Linux | systemd user unit `~/.config/systemd/user/lumisound-discord-rpc.service` | `systemctl --user status lumisound-discord-rpc.service` · `journalctl --user -u lumisound-discord-rpc.service -f` |
| macOS | LaunchAgent `~/Library/LaunchAgents/com.lumisound.discord-rpc.plist` | `launchctl list \| grep lumisound` · `tail -f ~/.config/lumisound-discord-rpc/daemon.log` |
| Windows | Scheduled Task `LumisoundDiscordRPC` (at log-on, restarts on failure) | `schtasks /Query /TN LumisoundDiscordRPC` |

Remove it with `lumisound-discord-rpc uninstall` (your sign-in is kept).

Your password is **never saved**. It is exchanged for a 365-day Rich Presence
token, which is all that gets written to disk (mode `0600`). That token only
allows reading your own playback state, and you can revoke it any time from
**Account → Active Sessions** ("Discord RPC Bridge") without changing your
password. With **two-factor authentication** on, you're asked for your
6-digit code.

Options: `--bridge-url URL` (self-hosted bridge), `--identifier EMAIL_OR_USERNAME`,
`--token TOKEN` (skip the password; see below), `--config-only` (sign in and write
`config.json`, register nothing), `--force` (Linux: shadow a unit that NixOS,
home-manager, Guix or a package already provides; without it `install` refuses
and suggests `--config-only`).

**After updating the binary**, run `install` again (or restart the service): the
running process keeps executing the old one until it restarts.

### Manual / any platform

Sign in without installing a service, then run the daemon in the foreground:

```sh
lumisound-discord-rpc login   # writes config.json (mode 0600) for you
lumisound-discord-rpc
```

### Still prefer a token?

Generate one from **Account → Discord Rich Presence → Generate Rich Presence
Token**, then `lumisound-discord-rpc install --token <rpc_token>`, or write
it yourself — note the config path differs on Windows:

```sh
# Linux / macOS
mkdir -p ~/.config/lumisound-discord-rpc
echo '{"access_token": "<rpc_token>"}' > ~/.config/lumisound-discord-rpc/config.json
```

```powershell
# Windows: %APPDATA%\lumisound-discord-rpc\config.json
```

Set `LUMISOUND_RPC_CONFIG` to use a different path entirely.

If you're self-hosting the ios-bridge instead of using the hosted one, add
`"bridge_url": "https://your-bridge-host.example.com"` to `config.json`, or
pass `--bridge-url https://...` to `install`.

## How it works

The daemon polls `GET /user/playback-state` on the bridge. Whenever
Lumisound's iOS app reports a track via the existing playback-state sync,
this daemon picks it up and calls `SET_ACTIVITY` over Discord's local IPC,
showing:

- **Details**: track title
- **State**: `by <artist>`
- **Timestamps**: elapsed/remaining bar based on `position_seconds` /
  `duration_seconds`

If playback is paused or the last update is older than 2 minutes (app
backgrounded/closed), the Rich Presence is cleared.

If you would rather keep a paused track on screen (without the elapsed timer),
set `"show_when_paused": true` in your config.
