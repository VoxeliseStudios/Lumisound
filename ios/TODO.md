# ios — to do

Full history and decisions: [`../RUST-PORT.md`](../RUST-PORT.md). Swift is views + app shell only.

## Done (2026-11-15) — LMS3 in the UI and every feature
- Format tags: "LMS3 OPUS/AAC/FLAC…" from the core's cached header badge; Now Playing shows codec +
  real bitrate and a PREMIUM chip; ALAC in LMS3 counts as lossless.
- Clip maker, clip export, lyric video, AI DJ export, AcoustID identify, BPM fallback open
  `avFoundationURL` (Opus decoded to WAV once) — they couldn't read Ogg Opus before at all.
- Library conversion: ordered (library order, never random), ≤30 tracks / 150 s per 5-minute pass,
  thermal check before every track, never the playing/queued track; your own imports in Documents
  (audio-only) convert too; files in linked external folders never. Settings → Streaming & Downloads
  shows "Lumisound Audio: N of M tracks".

## Done (2026-11-15) — performance telemetry
- `Services/PerformanceTelemetryService.swift`: display-link frame probe (batched 1/s), thermal/battery/
  Low Power/foreground/screen into the core sampler; minute windows uploaded as `perf_window` when
  flagged, `perf_heartbeat` every 15 min, `perf_thermal` on serious/critical.
- `Views/PerformanceMonitorView.swift` (Settings → Help → Performance Monitor): live CPU/GPU/power/
  memory/fps/thermal, CPU by thread, last minute's window.
- MetricKit daily payload → `metrickit_daily` event (CPU/GPU time, scroll hitch rate, peak memory,
  launch/resume/hang histograms, disk writes). The 60 s Swift system-CPU logger was removed.

## Open (telemetry)
- Per-view screen names beyond the tab bar (sheets like Now Playing full screen report as their tab).
- Bridge admin page: chart perf_window / metrickit_daily per device and app version.

## Done (2026-11-15) — Lumisound Audio (LMS3) + Premium
- Download format "Lumisound Audio (Best)" (`lumi`) is the new default (explicit choices kept).
- The 5-minute library pass converts every older `.lms` (plain/LMSLOCK1/2) to LMS3 in place
  (`relockLegacyFile` → `lms_relock_in_place`), then runs `QualityUpgradeService`.
- `Services/QualityUpgradeService.swift`: Premium status (`/user/ytdlp-cookies/premium`), up to 25
  YouTube tracks per 30 min re-fetched as `lumi` and swapped in place by the core (never the playing
  track, waits for thermal headroom). Settings → YouTube Cookies → Premium Audio card (status, toggle,
  progress, Upgrade Now).
- Search: "YT Music" source tab (`source=ytmusic`).

## Open (LMS3)
- Format Info sheet: show "Lumisound Audio", codec, kbps and Premium from `lmsInfo` (v3 fields exist).
- Cloud tracks: opt in to `lms3=true` on `/user/music/stream` once the cloud playback path reads LMS3
  directly (today the bridge sends the LMSLOCK1 wrap of the export).

## Done (2026-11-15) — bridge security pass
- `StreamingService.makeRequest` sends `X-Account-Token` whenever signed in: the bridge's shared-key
  routes now require a session (the app key is public inside the IPA).
- Server-library stream/artwork URLs carry the cached stream ticket (`&ticket=`); the server-library
  search warms the ticket first.

## Done (2026-11-07)
- Library Health > Listening > Often Skipped (`Views/OftenSkippedView.swift`).
- Skip history sync on start / every 20 min / backgrounding (`Services/AccountService+SkipSync.swift`).

## Done (2026-11-08)
- `Services/MusicHapticsService.swift`: Core Haptics player for the core's haptic score; Settings feel picker.
- `Services/SilenceTrimAnalyzer.swift` / `SilenceTrimService.swift`: core intro detector, engine opens tracks at the music.
- `LumisoundTV/Sources/TVAudioSettings.swift`: TVSilenceTrim on the core detector.

## Open
- Phase H: delete superseded Swift (AVAudioEngine graph etc.).
- tvOS/watchOS: show Often Skipped / use Smart Shuffle there too.
- Verify Music Haptics feel and timing on device (AirPods vs speaker latency).

## Done (2026-11-09)
- `Services/RadioService.swift`: Lumisound Radio sessions (start/refill/feedback), mode picker on Auto-Radio long-press.
- `Services/HapticsBrainService.swift`: background observation of every track + global model sync.
- Smart Crossfade on the core Blend planner (`queueRustBlendNext`).
- `Views/Components/PanningArtwork.swift`: thumbnail slide no longer freezes.
- Aria lyrics poll deadline 15 min (server-side engine).

## Done (2026-11-11)
- Strata interface: `Views/Strata/` (StrataStore, StrataRenderer, StrataRootView, StrataNowView) on core `strata.rs`.

## Done (2026-11-12)
- Strata: Library crash fixed (rows play long lists by reference); playlists (create/rename/delete/add/remove),
  Library sorting, player Now/Lyrics/Queue tabs, Settings (8 pages), Account and Cloud composed in Rust
  (`StrataSettings.swift` adapter, `StrataLyrics.swift`).
- Interface picker: cards share the screen width equally; previews no longer set the width (page was wider than the phone).
- Sideload widgets: unsigned release IPAs are now ad-hoc signed with only the App Group (group.com.lumisound.ios) so SideStore/AltStore register it and widgets can read app data. PixelBoost and SwarmPanel IPAs have the same gap (no embedded App Group).

## Open
- Strata: verify on device; rare advanced settings (API keys, app lock, notifications) still open the classic Settings.
- tvOS has no Strata.

## Done (2026-11-11, widgets + Live Activity revamp)
- `Shared/NowPlaying/NowPlayingSnapshot.swift`: one atomic now-playing record (title, art file, timing, speed,
  tint, engine line) read by the widgets and carried as the Live Activity's ContentState.
- `Services/WidgetDataService.swift`: per-track artwork files (no more old art beside a new title), trailing
  reload throttle (bursts used to drop the last reload), per-tick resync re-anchors on drift > 1 s, duration,
  speed or play-state changes; 1 s grace after a track change while the engine loads.
- `Services/LiveActivityManager.swift`: publishes the snapshot with a stale date at track end.
- `LumisoundWidget/Sources/`: new `NowPlayingViews.swift`; widgets and Live Activity redesigned — live system
  progress bars (the widget bar used to be frozen), 16:9 art shown wide, artwork-tinted backdrop, engine/BPM
  line, speed-aware time labels, Watch Smart Stack / CarPlay (`.small`) layout.
- `engineSummary` ("Rust engine · LMS OPUS · 48 kHz") sent with playback state for Discord Rich Presence.

## Done (2026-11-11, widget fix in 2.0.0.25)
- Widgets were stuck on "Nothing Playing" after 2.0.0.24: `WidgetDataService.publish` only saved when the App
  Group *folder* was reachable, but on the sideloaded install only the shared *defaults* work (the old code
  saved to defaults unconditionally). Now always saves to defaults; artwork falls back to small JPEGs in the
  defaults (`now_playing_art.<file>`, current + previous track) when there is no folder.
- Snapshot numbers are sanitized before saving (NaN/inf made JSON encoding fail silently).

- 2.0.0.26: medium/large widgets and the Live Activity crashed (SIGTRAP, 6 MetricKit reports at 21:17 while
  browsing the widget gallery; small rendered fine). Only those used `Text(timerInterval:countsDown: true)` for
  the remaining time; replaced with `Text(endDate, style: .timer)`. BPM / time formatting guard Int conversions.

- 2.0.0.27: system media player turned off (`AudioPlayerManager.showsSystemNowPlaying = false`): no
  MPNowPlayingInfo, no remote command targets, so the Live Activity is the only Lock Screen card. Trade-off
  accepted by the user: AirPods/headphone buttons, car Bluetooth, CarPlay, Siri media commands, Control Center
  and the Watch's Now Playing no longer control Lumisound.

- Live Activity stuck on the previous track after auto-advance/crossfade (widgets were right). Each update was
  its own `Task`, so updates could land out of order, and steady playback sends no more. Now one serial sender
  (newest last) plus `LiveActivityManager.verify()` each tick: resends when the activity's actual
  `content.state` shows another track or play state.

- Button lag (Live Activity pause/like): intents returned before the Live Activity update landed, and iOS
  redraws when the intent returns, so the old state stayed until a later redraw. Intents now await
  `LiveActivityManager.flush()` (via `LumisoundPlaybackCommandRouter.settle`); play/pause and favourite reload
  widgets immediately instead of through the 1 s reload spacing.

- Stale username on Account/home/profile screens: `refreshMe()` (GET /auth/me) was never called, so the
  user saved at sign-in was shown forever. Now `refreshMeIfStale()` runs on launch and on returning to the
  app (max once a minute) and refetches the social profile when the name changed.
- New Shortcut/Siri action "Download Saved Playlists" (`DownloadSavedPlaylistsIntent`, 10th and last
  AppShortcut slot): runs the tracked-playlist auto-downloader now, ignoring the 5-minute spacing.
- Background refresh was reported "denied" in the 2026-11-12 diagnostics: BGTaskScheduler never runs then,
  so saved playlists only download while the app is open (Settings > General > Background App Refresh).

## Open (widgets)
- If iOS still shows a bare system player while audio plays, the remaining lever is the audio session's
  `.mixWithOthers` option (then Lumisound no longer pauses other apps' audio).
- 2.0.0.28: Live Activity never showed artwork (every song). Cause: `AppGroup.id` used a keychain probe for the
  team-suffixed group, and the keychain is unreadable while locked (Lock Screen rendering) → wrong group.
  Now reads the group from embedded.mobileprovision, caches it per process, keychain last. Confirm on device;
  this may also be why the app reported the App Group folder as unavailable.
- Confirm 2.0.0.26 on device: medium + large in the gallery show the song, Live Activity appears on the Lock Screen.
- Find out why `containerURL(forSecurityApplicationGroupIdentifier:)` is nil on the SideStore install while the
  shared defaults work (check Settings > About warning, and the suffixed group ID SideStore registered).
- Verify on device: widget sizes, Dynamic Island layout, Smart Stack, tinted/clear Home Screen modes.

## Done (2026-11-12, six-part Lumisound pass)
- Live Activity stuck on old tracks (crossfade, or pause → skip/rewind, worst in the background), all three
  presentations (Lock Screen, Dynamic Island, Watch/CarPlay small): `LiveActivityManager` now updates every
  live activity (ends duplicates left by crashes — the one updated wasn't always the one on screen), coalesces
  bursts, sends under a background-task assertion (a paused app was suspended before iOS got the update),
  confirms delivery by snapshot `revision` and resends, and runs its own 12 s watchdog after each change (the
  player timer stops while paused). The Live Activity view also reads the App Group snapshot and shows it when
  its revision is newer than the activity state, so the redraw after a button tap is right even if ActivityKit
  dropped the update.
- Duplicate Finder rebuilt on the Rust Duplicate Engine (`rust/core/src/dupes.rs`): exact-file / same-source /
  same-recording (LumiPrint) / likely evidence, variant guard (Live, Remix, Sped Up… are never duplicates),
  keeper scoring with reasons, "Not duplicates", safe one-tap clean-up, merge of favorites + playlist places,
  Recently Deleted + undo journal (`Views/DuplicateFilesView.swift`, `FolderDuplicatesSheet.swift`,
  `Services/LibraryManager+DuplicateResolution.swift`). Aria auto-removal and the server fpcalc check are gone
  from this screen (the engine decides on-device).
- Classic Account screen: `Views/ClassicAccountView.swift` (listener pass, sync strip, Profile / Listening /
  Connections / Security decks). Lumen and Strata keep `AccountView`'s standard body.
- LumiBoost (volume > 100 %): `applyRustVolume` keeps the master at unity and sends the boost + output route
  profile to the engine; Now Playing shows the profile and a live hold-back meter.
- LumiEQ: `EQPreset` is now an id into the engine catalogue (18 presets in 4 categories, legacy names migrate);
  Now Playing panel with the engine's real response curve, preset cards, loudness match toggle.
- LumiSpace: Spatial Audio rooms (Studio / Wide Stage / Concert Hall / Crossfeed) picker in Settings.

## Open (2026-11-12 pass)
- Verify on device: Live Activity after pause → skip in the background; crossfade with the phone locked.
- Tune LumiBoost / LumiSpace by ear on AirPods and the phone speaker (levels were set by measurement).
- Duplicate clean-up does not merge play counts (history lives server-side); the plan reports them only.
- tvOS: no LumiEQ/LumiBoost/LumiSpace UI (tvOS uses its own audio path).

## Live Activity in the background (2026-11-12, after 2.0.0.32)
- Root cause (from device telemetry): every automatic track change while backgrounded logged "activity showed
  <old>, resending <new>" — iOS silently drops local `Activity.update` calls from an app playing audio with the
  `.playback` session in the background (Apple forums 716458 / 724483; open since iOS 16). APNs push updates
  are the only supported background path and this sideloaded build has no aps-environment ("APNs registration
  failed"). So:
  - Transport intents are also `LiveActivityIntent` (updates from inside an intent are allowed).
  - staleDate = end of track + 0.75 s: the system redraws then and the view reads the newer App Group snapshot,
    so the first automatic advance/crossfade after the last update shows without opening the app.
  - Background resend loop removed (it could never succeed).
  - Settings > Playback > Lock Screen Player: Live Activity / System Player / Both (system player always follows).
- Open: a second automatic advance in a row while backgrounded still waits for the app/a button (no redraw trigger).

## Settings family redesign (2026-11-12, branch settings-redesign → main)
- `Views/Kit/LumiKit.swift`: shared kit (LKPage/LKHero, LKCard/LKSection, LKRow/LKLink/LKToggle/LKControlRow,
  LKTile/LKGrid, LKStat, LKChip/LKPill/LKSegmented, LKField, LKBanner, LKEmpty, LKPrimaryButton, LKScroll,
  LKRowLinkStyle). Colours from AppTheme, so it follows the edition, accent and Liquid Glass.
- Settings (Classic + Strata): `SettingsView+Home.swift` — control-room home (search over every setting,
  account card, quick switches, nine room tiles with live details) and pages: Sound, Library, Downloads,
  Look & Feel, Notifications, Sleep & Wake, Lock Screen & Car, Privacy & Security, Help & About.
  Lumen keeps its own settings pages (old section vars, untouched).
- Own layouts: Notifications, Subscriptions & Tracked Playlists (dashboard + smart add + channel cards +
  playlist grid), Profile editor (live header, Identity/Style/Showcase/Privacy tabs, unsaved-changes bar),
  Library Health (grade gauge + issue tiles), Storage (usage bar), Duplicate Finder, Corrupt Files, Backup &
  Restore, Backup History (timeline), Compare Playlists (Venn), Listening Stats (podium), Liquid Glass (lab),
  Appearance (four shelves), Gallery Background (source cards), Deleted Playlists (countdown rings),
  Sessions (device cards), Wake-Up Alarm (clock face), Change Password (strength meter), Delete Account,
  Listening Goal, 2FA, YouTube API Key (credential card), Cookies (health checklist), Discord Webhook
  (message mock), Channel Settings sheet.
- Converted onto kit sections: Lua Themes, Help & Guide, Bug Report, Discover, Admin, Scrobbling, Rich
  Presence, Podcasts, Achievements, Time Capsules, Constellation, Inbox, Scripted Visualizer. Swipe actions
  there became long-press context menus (no List to swipe in).
- Kept as-is (already bespoke): Interface picker, Rewind, Needle Drop; Heatmap and Hearing Health got the hero.
- Open: verify on device (dark/light, Liquid Glass on/off, Dynamic Type), Lumen settings pages still old style.

## Done in 2.0.0.35 (2026-11-13) — batch requested 2026-11-12
1. Background downloader: no run on every launch/foreground. Manual "Start" button in Settings (one pass),
   still runs on the iOS daily background task, plus an in-app timer every 15 h. Resume only an interrupted pass.
2. Find out why YouTube API key quotas run out so fast (bridge usage, per-call unit costs, repeat resolves) and
   optimize (caching, fewer list calls, ETag/If-None-Match, batching, skip unchanged playlists).
3. Audio effects remade for the Lumisound engine (custom catalogue in Rust).
4. Now Playing artwork styles and seeker styles: redesign every one with its own idea.
5. All app animations remade as custom Lumisound motion, every animation capped at 60 fps.
6. Loading (launch) screen remade.

- Batch shipped in 2.0.0.35. Quota root cause: Spotify playlist matching spent 100 units (a Data API search) per
  track, so one 100-song import used a whole day; plus 100-unit interactive searches and tracked playlists
  re-resolved every 5 min with durations refetched each time. Fixed in the bridge (deployed): see commit 9dd0a453.
- Open: verify on device — artwork styles/seekers look and feel, launch dial, LumiFX character effects by ear.

## Task list (requested 2026-11-12 22:26) — auto-continue armed for 01:25
- [x] 1. Auto EQ fully on LumiEQ (catalogue presets, eqAutoPreset, spectral matcher + analyzer correction).
- [x] 2. New app identity: "sound eclipse" icon (waveform corona round a dark disc, pulse line, spark) for iOS, in-app display, tvOS parallax layers + Top Shelf, and a new watch AppIcon catalogue. Source: ios/Branding (make_icon.py + render.sh regenerate every size). Widgets use the host app icon.
- [x] 3. README: only what users need (what it is, install via AltStore/SideStore source, features). No build internals.
- [x] 4. Fix the same track appearing twice in Favorites.
- [x] 5. Subscriptions: channels are compact ~56pt rows in one grouped card (avatar with auto-download badge, meta line, mute/check/menu icons). TrackedPlaylistDetailView rebuilt: blurred-art hero, downloaded vs missing split bar, Missing / In Library / All filter + search, lazy track card, Tracking settings card.
- [x] 6. Audit of Apple-system features: ios/SWIFT-AUDIT.md section 5. Replaced DSWaveformImage with Rust waveform peaks (RustWaveformView), library search, engine format gaps and the engine toggle; remaining Apple uses listed with reasons (system integrations, mic, TTS, video export; SharePlay is task 12).
- [x] 7. Engine format coverage: rust/media tests/format_matrix.rs encodes every importable format with ffmpeg and checks rate, channels, exact length and tones (plain + locked .lms). Fixed: ALAC in CAF (atom-wrapped cookie, media/src/alac.rs), MP1/MP2 and MKA added, gapless trimming (media/src/gapless.rs: MP4 iTunSMPB / edit list for AAC, WAV fact for ADPCM, Ogg Opus end trim). New AAC path (media/src/aac.rs, pure-Rust syom) for HE-AAC v1/v2, 5.1/7.1 folded to stereo, 960-sample and implicit-SBR streams (tests/aac_profiles.rs). Engine toggle removed: always on, system engine only as automatic fallback. Not importable (no decoder): AC-3, E-AC-3, WMA, WavPack.
       wav/ogg/webm/alac...); custom decoders where missing; then hide the "Lumisound Audio Engine" toggle.
- [x] 8. Library redesign: LibraryHeader (big title, live songs/artists/albums/hours counts over an accent glow, Shuffle All + Add Music) and LibraryTabBar as an icon strip where the chosen section opens to icon+label with a gliding highlight (all 11 sections in ~one width, auto-scrolls to the selection); unified Rust search screen replaces tab content while searching (task 11). Tabs themselves (Artists shelf/letter headers/grid modes, Albums, Songs card styles, Hub) were already redesigned earlier and keep their layouts.
- [x] 9. tvOS overhaul: playback on the Lumisound engine (TVLumisoundEngine: pull render via engineRenderInto → AVAudioSourceNode; TVTrackFiles cache, locked files decoded in place; engine gapless + crossfade; AVPlayer pair, unlock cache and stall watchdog removed); Settings → Sound and a Now Playing Sound panel (LumiEQ presets from the core, Night Mode, Room); Listen Together on TV (TVListenTogether: join by code, follows the host with the Rust TogetherSync, reactions; Account → Listen Together); new icon. Screens keep the existing TV design system (neon/glass, redesigned in earlier rounds); everything new already exists on iOS.
       must be ported to iOS too.
- [x] 10. Tests + release: rust/media (format matrix, AAC profiles incl. HE-AAC/5.1, gapless, ALAC cookie, blur) and rust/core (138 unit tests incl. search, TogetherSync, pull render, deck gapless) pass; engine_cloud + dupes::real pass on real tracks; only the old engine_quality pre-echo-at-speed failure remains. Released 2.0.0.36 → 2.0.0.40; every IPA downloaded and checked: app + widget signed with group.com.lumisound.ios, SharePlay entitlement gone, AltStore source at 2.0.0.40. tvOS compiles in CI.
- [x] 11. Library search engine in Rust (rust/core/src/search.rs, LibrarySearch): songs, artists (credits split on feat/&/,/x), albums, playlists and lyrics (Lyrics folder, LRC times kept); accent/case folding, prefix-as-you-type, OSA typo tolerance, every-word AND, phrase/exact/plays/favourite boosts, UTF-16 highlight ranges; ~7 ms on 20k songs (prefilter + parallel ranking). UI: LibrarySearchView (top result, songs, artist/album ribbons, playlists, lyric cards that play from the line, scope chips) replaces tab content while searching. LibrarySearchService reindexes on library changes and re-reads lyrics when the folder changes.
- [x] 12. Listen Together without GroupActivities: SharePlayCoordinator/ListenTogetherActivity and the group-session entitlement removed. Rooms (bridge /rooms + /ws/live) do everything: Rust TogetherSync (rust/core/src/together.rs) does NTP-style clock offset from heartbeat round trips and decides load / seek / play / pause / ±5% tempo nudge (pitch kept, AudioPlayerManager.togetherRate) — simulated follower converges within 40 ms. Track identity = together_track_key (source id or folded title|artist), resolved to the library copy or the stream. Host republishes on track / play-state / seek (1 s watch). Live reactions (new POST /rooms/{code}/react, bridge deployed), queue suggestions/votes in QueueView, Follow-the-host toggle + sync status in the room.
- [x] 13. Cloud-track tests (2026-11-13): engine_cloud on 13 real tracks (Opus, AAC M4A, FLAC; locked .lms decoded in place) — every effect, speed, pitch, spatial and gapless check passes. The spatial check was fixed to measure against the dry render with GCC-PHAT (yaw 0: 0.00 ms on all tracks) and the >3 kHz head-shadow ILD (yaw 60: −15 to −16 dB). dupes::real passes (re-encode matched, other track rejected). Full 101-track set takes ~9 h at ~18 renders per track; the subset covers every format in the library.
- [x] 14. Responsiveness: telemetry-led (MetricKit showed daily GPU time up ~20× between 2.0.0.6 and 2.0.0.35 on the same account; no hang reports). Gallery background stills now come pre-blurred from the core (gallery_blurred_still: 720 px + fast_blur, blur kept as a share of the screen) so the Ken Burns pan moves a plain image instead of re-blurring a full-screen photo every frame behind every screen. Smart Crossfade no longer holds the analyzer (2048-pt FFT + SwiftUI publish at 23 Hz) for the whole of every track: armed 20 s before the fade only. Re-upload storm seen in telemetry was a one-time first backup (recent builds upload ~1–2 files/day).
- [x] 15. Owner/admin email voxellickinetix@icloud.com: SwarmPanel .env SWARM_PANEL_SITE_OWNER_EMAIL (new first, old kept) + container recreated; Nyxframe SITE_OWNER_EMAILS list (pushed, redeployed); Lumisound admin is the operator account (id ca8a4c53…), its email set to voxellickinetix@icloud.com.
- [x] 16. Git: global identity VoxeliseStudios <voxellickinetix@icloud.com>; remotes moved to github.com/VoxeliseStudios.

## Done (2026-11-14, engine pass: boost + stalls)
- Volume boost: LumiBoost v2 in the core (see `rust/core/TODO.md`) — every step above 100 % is audible on
  loud masters now. No Swift change; the hold-back meter keeps its meaning.
- Stalls: device telemetry (2.0.0.40) shows a healthy audio thread (0 underruns, load ≤ 11 %) but main-thread
  hangs of 6–50 s, with no way to see where. `Services/MainThreadBacktrace.swift` samples the main thread's
  stack while it is stuck (1.5 s and 6 s into a hang); `main_thread_hang` events carry it as `mainStack`
  ("Image 0xoffset symbol", innermost first).
- Core-side causes removed in the same pass: per-song cache lookups no longer copy + parse whole documents,
  output start is bounded to 3 s, `slow_start` telemetry no longer counts queued decks.

## Open (2026-11-14)
- Read `mainStack` from the next `main_thread_hang` reports (table `ios_app_event_log`) and fix what it names.
- Tune LumiBoost v2 by ear on AirPods and the phone speaker.

## Done (2026-11-14, sync: removals reach every device)
- Favorites and playlists are a three-way merge now (core `syncmerge.rs`, bridge merge-mode push). A favorite
  or playlist removed on one device is removed on the others; a playlist made on Android opens with this
  library's songs (rows resolved through `TrackResolver`); playlist entries follow adds and removes both ways.
  A playlist deleted on another device lands in Recently Deleted here.
- `pullSync` computes the merge off the main thread and applies it in one write (`LibraryManager+SyncMerge`).
  `pushSync` sends `"merge": true` with what this device removed; songs not in the library right now stay in
  the pushed playlist instead of being dropped (which told the server they were removed).
- `mergeFavoritesFromServer` (union-only pull that also re-uploaded every local favorite the server lacked,
  which put back anything removed elsewhere) is gone; `pullSync` covers it.
- `toggleFavorite` removes every id that makes the song a favorite (its own and other devices'): a favorite
  made on the Apple TV / Android could not be removed here before.
- A burst of live `sync_changed` events is one pull (1.5 s debounce).

## Done (2026-11-14, hang reports that can be read)
- `main_thread_hang` now carries `probeTicks` (how many of the monitor's own 250 ms timer ticks fired during
  the stall) and `mainThreadCPUSeconds`. A "stall" during which the monitor's timer barely fired was the whole
  process being suspended, not a hang: it is logged as `process_suspended_gap` (info) instead. A real stall
  with near-zero main-thread CPU was waiting (lock, synchronous call, disk); CPU close to the stall length
  was computing. Together with `mainStack` the next reports say what and where.

## Open (2026-11-14)
- Playlist order: a reorder made elsewhere is followed when this device has not reordered (core `follow_order`); two devices reordering at once keep their own order until one syncs first.
- The first sync after this update is a union (no base yet): something removed elsewhere before the update
  can come back once.

## Stalls: what 2.0.0.42 reported (2026-11-15)
- Three `main_thread_hang` reports (8, 16 and 32 s), all real (the monitor's timer kept ticking) and all with
  0 s of main-thread CPU: the main thread was waiting, not working. Each began a few seconds after launch or
  after returning to the app, on the Classic library screen, with no scan running.
- `mainStack` is the same place every time: the run loop's update cycle → UIKit → Core Animation's
  transaction commit, blocked either in a `dispatch_sync` inside QuartzCore (waiting for another thread that is
  in the same commit path) or in a synchronous `mach_msg` to the render server. No app frame is on the stack.
- So the app's own main-thread code is not the stall; something else in the process (or the render server) is
  holding Core Animation's commit. The report could not say what, so 2.0.0.43 adds `otherStacks`: every other
  thread that is inside app code or the drawing / media frameworks, sampled 3 s into the stall.
- Next: read `otherStacks` from the next report. Symbolicate `Lumisound 0x…` frames with the release's dSYM.

## Stalls: what 2.0.0.44 reported (2026-11-15, first hour on the owner's iPhone 13 mini)
- 5 `main_thread_hang` reports, all short (1.5–2.9 s, warning level), none of the 8–32 s Core Animation
  commit hangs yet (too little time on 2.0.0.44 to call those gone). None reached 3 s, so `otherStacks` is
  still empty everywhere.
- Symbolicated with `Lumisound-v2.0.0.44-dSYMs.zip` (offset + 0x100000000 − 4, `llvm-symbolizer`):
  - 2.90 s and 1.56 s: `LumisoundTrackVaultService.runLegacyRelockPass` → `LumisoundLockFormat.isLocked(at:)`
    → core `lms_is_locked` → `lumisound_media::lock::read_header` (`open`/`read`). The relock work was moved
    off-main, but the candidate filter (`importedSongs.compactMap { … isLocked(at:) }`) still opens and
    reads the header of every converted track on the main actor, every pass. Fix: build the candidate list
    in `Task.detached` (or a core fn that takes all paths), only the tally on the main actor.
  - 1.60 s: SwiftUI text drawing in a commit while a library scan ran (`libraryScanInFlight`); no app frame.
- `disk_write_exception` (iOS "excessive disk writes"): 304 of the sampled writes are
  `CorruptFileFinderService.validateAudio` → core `validate_audio` → `readable` → `lms_playable_path` →
  `unlock_file`: the corrupt-file scan writes a decrypted copy of every `.lms` track to the cache to check it.
  Fix: validate from the locked bytes (decode through the lock reader / the new `fd:` source) with no copy.
- Playback on 2.0.0.44: no `playback_stall_recovered`, no `output_restarted`. One `stream_load_failed`:
  imported "Ocean Man.mp3" is not decodable (core: unsupported_format, duration 0; AVPlayer −11800/−17913),
  most likely a bad file, not a player bug.
- Elsewhere: an account on 1.9.0.111 logs "position frozen" 12× on 1–1.5 h tracks (old build); 2.0.0.6
  devices restart output in loops on `interruption_ended` while locked on Bluetooth (restartsLast30s up to 5).

## Done (2026-11-15, 2.0.0.45: the 2.0.0.44 telemetry fixes)
- Relock pass: the candidate filter (header read of every converted track) runs off the main actor through
  the core's `lmsLegacyUnlocked(paths:)`; only the path snapshot and the tally stay on the main actor. This was
  the 1.5–2.9 s `main_thread_hang`s in 2.0.0.44.
- Excessive disk writes (iOS `disk_write_exception`, 4.3 GB in the 2.0.0.44 report): the corrupt-file scan
  and every core analysis call (`validate_audio`, waveform, BPM, loudness, transition/spectral profile,
  `is_valid_audio`) decode `.lms` tracks in place instead of writing an unlocked copy per track.
- 2.0.0.42's 1.1 GB report was led by `library_snapshot_save`: an unchanged snapshot is no longer rewritten.

## Open (after 2.0.0.45)
- Watch for the 8–32 s Core Animation commit hangs (2.0.0.42) coming back; a report ≥ 3 s now carries
  `otherStacks`. Symbolicate with the release's dSYMs (offset + 0x100000000 − 4, `llvm-symbolizer`).
- Confirm `disk_write_exception` stops after a full corrupt-file scan on 2.0.0.45.
- Old builds in the field (1.9.0.111 "position frozen" on 1 h+ tracks, 2.0.0.6 output-restart loops on
  `interruption_ended`) are fixed in current builds; those devices need to update from the source.
- Core tests failing before this change too (not caused by it): `blend_cloud` blend_starts_on_cue_with_bass_swap
  (level hole, min_ratio), `pitch_cloud` pitch_keeps_sustained_notes_through_drum_hits; `store` background-write
  test is flaky under a parallel `cargo test`.

## Done (2026-11-15, Folders grid + folder view scroll)
- Screen recording 2026-11-14 20:49: flinging the Folders grid ran at 10–20 fps (frames 50–120 ms apart).
  Each tile was four `ArtworkThumbnail`s (UIKit `PanningArtwork` with an endless Core Animation slide for
  16:9 art, placeholder, shimmer, scrim, continuous-corner clip) in a GeometryReader under
  `.compositingGroup().shadow`, so ~48 animating, masked layers and a per-frame offscreen shadow pass.
- `Services/FolderMosaicService.swift`: each folder cover is one bitmap (collage / custom cover / single
  track, corners, badge or hairline, drop shadow baked in), rendered off-main, memory-cached (48 MB),
  concurrent requests share one render. `FoldersTab` tiles and the folder detail header (`.header` style)
  show it as a single `Image`; the grid measures its width once (`onGeometryChange`) instead of a
  GeometryReader per tile. Grid collage art is now a still, centered crop (no panning) — by design.
- `HeroArtworkBackdrop` (folder detail, Queue and 4 more headers): blurred once at 160 px off-main
  (`BackdropBlur`) instead of a live `.blur(radius: 50)` on full-size artwork every frame.

## Open (Folders)
- Check on device that a fling through the Folders grid holds 60/120 fps and that tiles fade in without
  placeholders flashing when scrolling back. The hub speed-dial folder tiles still use live thumbnails.

## Done (2026-11-15, Subscriptions grid + render-server hangs)
- Subscriptions → Playlists grid (screenshot): cards ran off the left edge and a playlist without art shrank to
  its icon. The aspect-fill 16:9 thumbnail was the sized view; now a square `Color.clear` box sizes the card and
  the thumbnail is an overlay on it.
- Telemetry after 2.0.0.45 (8 min on device): 11.9 s and 2.4 s hangs; 2.0.0.44 had 11.4 s and 3.7 s more. The
  long ones now carry `otherStacks`: main thread in Core Animation's commit, `dispatch_sync` onto a RenderBox
  queue whose thread is in a synchronous `mach_msg` to the render server; no app frames anywhere, ~0 s main CPU.
  Of every account reporting in 7 days, only the owner has the gallery background on (photos, 41 images,
  Ken Burns, Liquid Glass) and only the owner has hangs ≥ 5 s (16 of them).
- Ken Burns was a `TimelineView` committing a new transform for the full-screen image ~12×/s forever (main
  thread), with every glass control re-rendering against the moved backdrop. It is now a Core Animation
  transform animation on the image view's layer (`KenBurnsImageView` in `Utilities/AnimatedImageView.swift`):
  same framing (1.08→1.13, ±7/5 pt, 30 s legs, eased), paused/held when hot, Low Power, or tab not visible.

## Open (render-server hangs)
- If the CA-commit hangs continue on 2.0.0.47 with the gallery background on, try with it off for a day to
  confirm; next suspects are the Liquid Glass controls over the photo and the GIF live-blur path.

## Done (2026-11-15, 60 fps pass over the Library screen and its tabs)
Same causes as the Folders grid, removed everywhere they repeat per cell:
- Real Liquid Glass per cell (a live backdrop blur per card/row, re-rendered whenever the content under it
  moves): new `cellSurface(in:fallback:)` (Theme/GlassEffectCompat.swift) draws the pre-iOS 26 card look
  (fallback fill + user glass tint + hairline) as plain shapes. Used by SongGridCell, card-style SongRow,
  Album/Artist list rows, Hub Jump Back In + friends activity, search/cloud result rows (Server/Stream/User
  track rows, stream grid cell, trending rows, station cards). One-off chrome keeps real glass.
- `.shadow` on changing content (panning covers, fading artwork) = an offscreen pass per cell per frame: new
  `shadowBehind(shape:)` (Theme/ScrollPerformance.swift) casts it from a static shape behind the cover. Songs
  grid, album grid/list/Recently Added, artist grid/top shelf, hub speed dial + song carousel, mood cards,
  mini player cover, Lumen album/artist/playlist covers.
- `ShimmerOverlay` was a TimelineView re-running a SwiftUI body per loading cell (dozens during a fling): now a
  Core Animation gradient band (`ShimmerBandView`); `SwiftUIShimmerOverlay` remains for `.drawingGroup()`.
- `ArtworkThumbnail(pans:)`: covers under 100 pt (list rows) no longer pan — invisible at that size, but an
  endless layer animation per row. Grid-size and larger covers still pan.
- Hub: `HubArtworkCollage` (speed dial + top artists/decades/genres carousels) is one pre-rendered bitmap
  (`FolderMosaicService` `.collage` style); `HubParallaxCard` uses `.scrollTransition` instead of a
  GeometryReader reading the global frame (it re-ran every card's body on every frame of any scroll).
- Songs list: `ForEach(sortedSongs)` + the cached index instead of `Array(sortedSongs.enumerated())` (a
  full-library copy on every body pass).
- Library header glow: radial gradient instead of a 60 pt blur filter.

## Open (performance)
- Verify on device (Library tabs, hub, folder/album pages) that flings hold 60/120 fps; `main_thread_hitches`
  telemetry on the next build is the field check.
- Remaining per-item costs not touched: GeometryReader per grid cell (layout only), Lumen `LumenShelfCard` art
  shadow (shape unknown), `PlaylistRow` sums durations per render.

## Done (2026-11-15, social redesign + Library screens)
- Profile tab (`PublicProfileView`, building blocks in `Views/Social/ProfileStage.swift`): a stage instead of a
  column of identical cards — full-bleed banner hero with pull-to-stretch (`visualEffect`, no per-frame body),
  centred avatar/identity/status pill, action row (Edit / Send a Song / Share), stat strip (streak, best,
  badges, visits, member since), live "On air" card with Vibes, then sticky Showcase / Music / Wall tabs
  (record-sleeve pinned tracks that play, medal wall, top-artist portraits, genre chips, fingerprint, match +
  Blend, guestbook, visitors). Panels are `cellSurface`; the accent wash lost its 60 pt blur; `ProfileInfoCard`
  (editor, friend banners) is `cellSurface` too.
- Friends tab (`FriendsListView`): one hub instead of four pill tabs — big serif header with counts, requests
  banner, quick actions (Song Drops with unseen badge, Activity, All Friends, Find People), "Listening now"
  carousel with artwork + Vibe bar, latest drops, friends ordered by presence with a send-a-song button, weekly
  podium. Discover/Requests/Activity/All Friends are pushed pages (existing views).
- New social features (bridge `routes/social.rs`): Song Drops (send a friend a song + note; they play it —
  own copy if in the library, else the YouTube stream — and react with an emoji) and Vibes (emoji reaction to a
  friend's current track). `Services/SongDropsStore.swift`, `Views/Social/SongDropViews.swift`.
- Aria update notice: announcement `below_version 2.0.0.48` (14 days, download category) tells everyone on an
  older iOS build to update via AltStore/SideStore (gallery source) or GitHub releases.
- Library header: compact counts ("5.8K songs") that never truncate; stats drop from the end when tight.
- Genres: tile wall (per-genre colour, collage cover, count, play) + new `GenreDetailView`.
- Playlists: action tiles (New, Smart, Shared, Import M3U) + grid of collage covers (`CollageCover`).
- Add Music: hero + big Import from Files card with formats, Cloud card, computer transfer steps.
- Songs index rail: A–Z + "#" only (NFKC/case/width folding), no more 60-entry rail under the mini player.

## Open (social)
- Notification taps for `song_drop` / `vibe` / `song_drop_reaction` could open the Song Drops inbox.
- Drops unseen count on the Friends tab icon.

## Done (2026-11-15, speed/pitch "randomly" changing between songs)
Telemetry showed the owner's pitch at odd values (−0.45, −0.64, −0.90, −1.04, −1.79, −2.56 st) with speed 1.0, and
the synced default settings held pitch −0.6386; one track override held −2.5. Causes and fixes:
- Now Playing pitch/speed sliders were continuous (brushed while swiping between songs) and the pitch label
  rounded to whole semitones: now 0.5 st / 0.05× steps, `%+.1f st` label, reset buttons.
- `audioSettings.didSet` saves every write as the default or the current track's override, so temporary tempo
  changes became permanent: the sleep-timer wind-down and Workout Mode now use `transientRate` (never saved,
  multiplied in at the engines: Rust setSpeedPitch, AVAudioEngine timePitch, AVPlayer rate).
- Leaving a tempo/pitch effect (Slowed, Nightcore…) for a neutral one kept its speed/pitch forever:
  `AudioEffectsService.apply` restores the neutral value unless the listener changed it meanwhile.
- Listen Together: a sync nudge (±10%) stayed after a room ended without `leave`; reset whenever not following.
- `AudioSettings.withCleanTempo` resets values no control produces (off the 0.1 st / 0.01× grid) on load,
  server restore and per-track maps; the cleaned default syncs back.

## Done (2026-11-15, speed/pitch drift part 2)
- Speed/pitch always run on the Lumisound (Rust) engine's own stretcher/resampler; the
  native `AVAudioUnitTimePitch` graph is only the rare fallback (2 fallbacks/3 days, none on 2.0.0.5x).
- Remaining drift was Smart Crossfade's hard-wired beat-match: every auto-transition stretched the
  next song up to ±8 % and eased it back, and a blend that never started handed the stretched deck
  over gaplessly. Beat-match is now opt-in (`AudioSettings.beatMatchEnabled`, Settings > Sound >
  Smart Auto Crossfade > Beat-match tempo); off, the core planner never touches tempo and bar-locks
  only tempos within 2 % (≤60 ms drift). A missed blend drops the nudge (`DeckShared.nudge_off`).

## Open (audio)
- `core/tests/engine_quality.rs` fails pre-echo on drum hits at 1.25x/0.8x (-12/-16 dB vs -18 limit):
  stretcher transient smearing when speed != 1. Pre-existing; only heard with a non-1x speed.
- Phase H still pending: remove the AVAudioEngine graph once the fallbacks (output start failure,
  system-engine streams) also run on the core engine.
