-- Incremental cloud metadata: GET /user/music/metadata?since=<ISO time>
-- returns rows changed after that time plus ids deleted since.
ALTER TABLE ios_user_music_metadata ADD COLUMN IF NOT EXISTS updated_at TIMESTAMP;
UPDATE ios_user_music_metadata SET updated_at = COALESCE(uploaded_at, NOW() AT TIME ZONE 'UTC') WHERE updated_at IS NULL;
ALTER TABLE ios_user_music_metadata ALTER COLUMN updated_at SET DEFAULT (NOW() AT TIME ZONE 'UTC');
CREATE INDEX IF NOT EXISTS ios_user_music_metadata_user_updated ON ios_user_music_metadata (user_id, updated_at);

CREATE TABLE IF NOT EXISTS ios_user_music_deleted (
    user_id VARCHAR(36) NOT NULL,
    id VARCHAR(64) NOT NULL,
    deleted_at TIMESTAMP NOT NULL DEFAULT (NOW() AT TIME ZONE 'UTC'),
    PRIMARY KEY (user_id, id)
);
CREATE INDEX IF NOT EXISTS ios_user_music_deleted_user_at ON ios_user_music_deleted (user_id, deleted_at);

-- A row that comes back (re-upload, trash restore) is no longer deleted.
CREATE OR REPLACE FUNCTION lumi_touch_music_metadata() RETURNS trigger LANGUAGE plpgsql AS $f$
BEGIN
  NEW.updated_at := NOW() AT TIME ZONE 'UTC';
  IF TG_OP = 'INSERT' THEN
    DELETE FROM ios_user_music_deleted WHERE user_id = NEW.user_id AND id = NEW.id;
  END IF;
  RETURN NEW;
END $f$;
DROP TRIGGER IF EXISTS ios_user_music_metadata_touch ON ios_user_music_metadata;
CREATE TRIGGER ios_user_music_metadata_touch BEFORE INSERT OR UPDATE ON ios_user_music_metadata
  FOR EACH ROW EXECUTE FUNCTION lumi_touch_music_metadata();

CREATE OR REPLACE FUNCTION lumi_tombstone_music_metadata() RETURNS trigger LANGUAGE plpgsql AS $f$
BEGIN
  INSERT INTO ios_user_music_deleted (user_id, id) VALUES (OLD.user_id, OLD.id)
    ON CONFLICT (user_id, id) DO UPDATE SET deleted_at = EXCLUDED.deleted_at;
  RETURN OLD;
END $f$;
DROP TRIGGER IF EXISTS ios_user_music_metadata_tombstone ON ios_user_music_metadata;
CREATE TRIGGER ios_user_music_metadata_tombstone AFTER DELETE ON ios_user_music_metadata
  FOR EACH ROW EXECUTE FUNCTION lumi_tombstone_music_metadata();
