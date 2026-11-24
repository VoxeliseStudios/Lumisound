-- Cross-device favourite identity, the same rule as core crossdevice::file_key:
-- the file name without folders, the "local:" scheme, the .lms lock suffix,
-- the audio extension or case. "local:Imported Music/A/Song.opus.lms" (iPhone),
-- "Song.opus.lms" (Apple TV / Android cloud) -> "song".
CREATE OR REPLACE FUNCTION lumi_fav_key(id text) RETURNS text LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $f$
  SELECT NULLIF(btrim(
    regexp_replace(
      regexp_replace(
        lower(regexp_replace(regexp_replace(id, '^local:', ''), '^.*[/\\]', '')),
        '\.lms$', ''),
      '\.(opus|mp3|m4a|aac|flac|wav|ogg|oga|webm|mka|mp4|aiff|aif|alac|caf|wma)$', '')
  ), '')
$f$;
CREATE INDEX IF NOT EXISTS ios_user_favorites_user_key ON ios_user_favorites (user_id, lumi_fav_key(song_id));
