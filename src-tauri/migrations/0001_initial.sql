-- This device's copy of what it can see, so chats and notes open offline, plus what it hasn't sent yet.

-- Settings and account: hubUrl, deviceId, me (JSON), familyName, members (JSON), rev (the last change synced).
CREATE TABLE kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE chats (
    id TEXT PRIMARY KEY,
    json TEXT NOT NULL
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    json TEXT NOT NULL
);
CREATE INDEX messages_by_chat ON messages (chat_id, created_at, id);

-- Messages written but not yet on the hub. `upload` is a picture in the media folder still to be sent first.
CREATE TABLE outbox (
    id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL,
    body_json TEXT NOT NULL,
    upload TEXT,
    created_at INTEGER NOT NULL,
    error TEXT
);

-- `dirty`: changed here and not yet saved on the hub. `deleted` with `dirty`: a deletion still to send.
CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    json TEXT NOT NULL,
    dirty INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0
);
