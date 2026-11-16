# rust/bridge — to do

Full history and decisions: [`RUST-PORT.md`](../../RUST-PORT.md).

## Done (2026-11-16) — downloads after a reinstall, cloud trash, delisted tracks
- Single downloads (`/api/download`, `/api/download/relay`) are skipped only when the asking device's
  own `existing_ids` lists the track. The stored inventory (last install's snapshot) no longer vetoes
  them: after a reinstall every download came back 204 "already in your library" while the phone had
  nothing. Playlist resolves still trim with the inventory.
- `DELETE /user/music/*`: moved to `<root>/.trash/<user>/<YYYYMMDD>/<rel>` with the metadata row
  (`.meta.json`) and locked art (`.art.jpg`) beside it; day folders older than 30 days purged by the
  daily janitor (`music::purge_trash`). Previously erased on the spot.
- Delisted YouTube tracks ("Video unavailable", e.g. unlisted "- Topic" art tracks): one retry with the
  matching YouTube Music release (same title ignoring asides, length within 3 s / 3 %, else same
  artist). The file keeps the requested `LUMISOUND_ID`.
- LMS3 cloud migration skips files deleted since the folder was listed (was "write: No such file").

## Incident 2026-11-16 — cloud library delete-all
- A fresh install (v2.0.0.56, empty library) ran cloud "Delete All" (confirmed) and "Clear all backups";
  3,501 files were erased before the trash existed. Metadata survives in the nightly DB dump, the old
  inventory in a snapshot table, 556 originals in `.lms3-backup/originals/`; the rest re-download by
  source id. Restore only on the owner's word.

## Done (2026-11-16) — admin performance
- `GET /admin/api/perf?hours=` (1..720): hourly CPU / power / hitch / fps / peak memory / underruns from
  `perf_window` + `perf_heartbeat`, the 15 screens with the most hitching, per-version summary. The admin
  page charts it (inline SVG, no libraries) and escapes every table cell.
- The "Jobs completed (24h)" card counted status `completed`, which downloads never use (`success`), so
  it always read 0.

## Open (2026-11-16)
- Admin restore from the trash (move back + reinsert `.meta.json`), and a trash view in admin.html.
- Alternate-release fallback for SoundCloud/Bandcamp ids (YouTube only now).

## Done (2026-11-15) — LMS3 cloud, YouTube Premium, YouTube Music
- `premium.rs`: is an account's cookie file YouTube Premium? yt-dlp `-J` on YouTube Music tracks per
  client (web_music, web_creator, tv, web+default) until one lists 774/141 or ≥200 kbps; stored in
  `ios_ytdlp_premium` (per cookie hash, re-checked daily, re-probed on cookie put/delete/preference).
  `GET /user/ytdlp-cookies/premium[?refresh=true]`.
- Downloads: format `lumi` (best stream as is, `--audio-format best`), Premium accounts get `774` first
  (also for opus/best/mp3/flac/wav) through the client that served it; the chosen format is tagged
  `LUMISOUND_QUALITY=yt:<itag>` (the app's LMS3 meta keeps it).
- `ytmusic.rs`: `/api/search?source=ytmusic` straight from music.youtube.com's WEB_REMIX search (songs
  filter): every artist, album, length, 544 px square art; falls back to YouTube search.
- `lms3_cloud.rs`: cloud `.lms` → LMS3 in place (gzip too), proven identical, originals moved to
  `<root>/.lms3-backup/originals/<user>/<rel>`; then Premium accounts' YouTube tracks re-fetched as 774 and
  replaced (old copy in `.lms3-backup/upgrades/`), 150/user/pass, 20 s apart. Every 6 h; `LMS3_CLOUD=off`
  disables. `GET/POST /admin/api/lms3` (status / run now).
- `/user/music/stream`: LMS3 goes out as a cached LMSLOCK1 wrap of its export (current apps unlock it as
  before; 3 h cache) unless `lms3=true`; low/medium transcodes read the export.
- `storage::readable_copy` exports LMS3 (gzip or not) for ffprobe/ffmpeg.

## Open (LMS3 / Premium)
- Verify the 774 path with a real Premium cookie file (none on the server now: global cookies probe
  at 130 kbps). If no client lists 774, try `youtube:player_client=web_music;formats=...` per yt-dlp docs.
- Delete `<root>/.lms3-backup` once the converted cloud library has been played for a while (~22 GB).
- Streaming (not downloads) still uses the m4a stream; Premium streaming could use 774 via the engine.

## Done (2026-11-15) — repeating announcements
- `ios_announcements.repeat_hours` (NULL = once): shown again every N hours while the user is still
  eligible; `/seen` upserts `seen_at`. Admin create accepts `repeat_hours` (1..720).
- Live: the Aria "please update" notice (c071fa78…) targets below 2.0.0.52, repeats hourly, ends
  2026-12-15 23:59 local (2026-12-16 05:59 UTC).

## Done (2026-11-15) — security pass (see ~/Projects/SECURITY-AUDIT.md)
- Shared-key routes (`ApiKey`: search, stream, download*, track, resolve, playlist, radio, lyrics,
  trending/suggestions) now need a live signed-in session (`X-Account-Token`, or a user JWT as the
  bearer). The app key ships inside the public IPAs/APK, so alone it no longer authorizes anything.
- `/api/stream/proxy`: session (header, session-table checked) or stream ticket; key alone → 401.
- `/ws/live`: token must be a live session (logout/revocation ends live events).
- `/api/library/server`, `/stream`, `/artwork`: session or `?ticket=` (`SessionOrTicket`).
- Per-IP limits: `/bug-report` 10/h (429), `/internal/logs` 1800/h, `/api/log-event` 3600/h (over cap → 204, dropped).
  `PUT /api/lyrics` 60/h per account. Limiter: `AppState::rate_allow`.
- ADMIN_TOKEN compared in constant time (`auth::ct_eq`).
- Open: old app builds that call trending/suggestions without the session header get 401 until updated
  (iOS 2.0.x with `makeRequest` sending X-Account-Token fixes it; Android/tvOS already send it).

## Done (2026-11-12)
- `USER_MUSIC_EXTRA_DIRS`: user libraries can live on more drives (web-stack mounts the terabyte store at
  `/usermusic-terabyte`). A user's folder stays on the drive it's on; new users go to the emptiest drive.
- To do: moving an existing user's library between drives is manual (stop bridge, move folder, start).

## Done (2026-11-07)
- `POST /user/skip-history` + table `ios_user_skip_history` (schema.sql): cross-device skip sync.
- `/user/music`: per-file filesystem work moved off the async workers.

## Open
- Incremental `/user/music/metadata?since=` (low priority: table is tiny today).
- Prune zero-count skip rows older than ~1 year.

## 2026-11-09
- [x] routes/radio.rs (Lumisound Radio), routes/haptics.rs (Haptics Brain), lyricsgen.rs (Aria Lyrics), realness.rs.
- [ ] Aria Lyrics: cache Demucs vocals per track hash (re-runs skip separation).

## 2026-11-11 (Rich Presence)
- [x] `GET /api/rp-art/yt/{id}.jpg`: YouTube thumbnail shaped for Discord's square slot (16:9 kept whole over a
      blurred fill), cached on disk under ytdlp_cache_dir/rp-art-yt.
- [x] `/user/rp-artwork-upload` shapes on upload; older unshaped files are shaped on first read.
- [x] `ios_playback_state.engine` column (schema.sql); playback handoff clears stale artwork/engine.
- [ ] Prune rp-art-yt / rp-artwork caches (unbounded today, ~40 KB per track).

## 2026-11-12: age-restricted downloads (403)
- YouTube now asks the cookie account to age-verify; `web_embedded` is UNPLAYABLE and the fallbacks
  (`web_creator`, `mweb`) hand out streams that 403 ("unable to download video data"). Cookies, PO token
  (bgutil 2.0.2) and yt-dlp 2026.10.26 were all fine.
- Fixed in config: `YTDLP_FALLBACK_CLIENT=tv` (web-stack docker-compose.yml). The `tv` client still serves
  these with the same cookies; output is .m4a. Verified on 4 previously failing videos.
- Done: a failed attempt whose output says age-restricted / age-verification switches the remaining attempts
  to `YTDLP_FALLBACK_CLIENT` (`ytdlp::needs_age_fallback`); "only available to Music Premium" is a permanent
  failure with its own message (no retries). Tested: Premium video stops after attempt 1. The age gate is
  intermittent: at 05:59 UTC every previously failing video downloaded on attempt 1, so the switch hasn't
  been seen live yet — look for "Age-restricted (...): retrying with the tv client" in the logs.
- Image build: the demucs model host's TLS cert expired (2026-11-12). The Dockerfile now copies the
  checkpoint from rust/models/demucs/hub/checkpoints/955717e8-8726e21a.th (git-ignored, 84 MB, sha256 starts
  8726e21a as its name says). On a fresh machine, extract it from a previous bridge image:
  `docker cp $(docker create <old image>):/app/cache/demucs/hub/checkpoints/955717e8-8726e21a.th rust/models/demucs/hub/checkpoints/`

## 2026-11-12: username changes
- Limit is now 3 per rolling 30 days (`MAX_USERNAME_CHANGES_PER_30_DAYS`, routes/account.rs).
- schema.sql: trigger `ios_users_username_changed` writes ios_username_change_log on every rename and
  rewrites each column registered in `username_sync_targets` (empty today: no table copies usernames).
  `resolve_username(name)` = current name, else a former name nobody holds now; used by friend requests
  and playlist collaborator adds. Login deliberately still needs the current name or email.
- Operator account renamed HeavenlyXenusVR -> Voxelise (Nyxframe user 8 HollowHeavenXR -> Voxelise too).

## 2026-11-14: merge-mode sync push
- `POST /user/sync` with `"merge": true` (routes/library.rs `merge_library`): favorites upserted,
  `removed_favorites` deleted; pushed playlists upserted, `removed_playlists` deleted, others untouched; a
  pushed playlist's rows become the pushed list followed by rows the device has not accounted for (`seen`,
  identities from `row_identity`, same function in core `syncmerge.rs`). Without the flag the push replaces
  the account as before (old app versions).
- Favorite and playlist endpoints now announce `sync_changed` on `/ws/live` (Android edits reach the iPhone at once).
- Playlist upserts never touch another account's playlist id (both push paths).

## Done (2026-11-15)
- Song drops & vibes: `ios_social_drops`, /api/social/drops (GET received+sent+unseen, POST send), /{id}/seen,
  /{id}/react, DELETE. Friends-only, block-aware, 40/hour, vibe dedupe within 10 min, notifications.
- `ios_announcements.below_version`: announcement only for users whose latest iOS telemetry version is lower.
  First use: "please update" notice, below 2.0.0.48, 14 days (2026-11-15).
