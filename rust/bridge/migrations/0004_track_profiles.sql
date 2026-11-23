-- Shared Smart Crossfade (Blend) profiles for streamed tracks, keyed by
-- source id. Measured by an app from its downloaded copy (core
-- `blend_profile`), shared so the next device skips the decode.
CREATE TABLE IF NOT EXISTS ios_track_blend_profiles (
    source VARCHAR(20) NOT NULL,
    source_id VARCHAR(280) NOT NULL,
    version INTEGER NOT NULL,
    profile JSONB NOT NULL,
    contributed_by VARCHAR(36),
    updated_at TIMESTAMP NOT NULL DEFAULT (NOW() AT TIME ZONE 'UTC'),
    PRIMARY KEY (source, source_id, version)
);
