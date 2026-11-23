-- Downloads that failed only because YouTube walled the cookies (bot check)
-- or rate-limited the session. The bridge tells the app to retry them once
-- that cause has cleared (live event `downloads_retry` + GET /api/download/retry).
CREATE TABLE IF NOT EXISTS ios_download_retry (
    user_id VARCHAR(36) NOT NULL REFERENCES ios_users(id) ON DELETE CASCADE,
    source VARCHAR(20) NOT NULL,
    source_id VARCHAR(280) NOT NULL,
    title TEXT,
    artist TEXT,
    format VARCHAR(20),
    reason VARCHAR(20) NOT NULL,
    failures INTEGER NOT NULL DEFAULT 1,
    failed_at TIMESTAMP NOT NULL DEFAULT (NOW() AT TIME ZONE 'UTC'),
    ready_at TIMESTAMP,
    PRIMARY KEY (user_id, source, source_id)
);
CREATE INDEX IF NOT EXISTS ios_download_retry_pending ON ios_download_retry (ready_at, failed_at);
