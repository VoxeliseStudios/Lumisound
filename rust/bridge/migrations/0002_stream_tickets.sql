-- Stream tickets survive bridge restarts (looked up by SHA-256 on a cache miss).
CREATE TABLE IF NOT EXISTS ios_stream_tickets (
    ticket_hash CHAR(64) PRIMARY KEY,
    user_id VARCHAR(36) NOT NULL REFERENCES ios_users(id) ON DELETE CASCADE,
    expires_at TIMESTAMP NOT NULL
);
CREATE INDEX IF NOT EXISTS ios_stream_tickets_expires ON ios_stream_tickets (expires_at);
