-- The family. There is one row, made when the first parent sets the hub up. `rev` is the change counter: every
-- change to something devices sync takes the next number, so a device asks for everything after the last one it saw.
CREATE TABLE family (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    name TEXT NOT NULL,
    rev INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE members (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('parent', 'kid')),
    color TEXT NOT NULL,
    avatar TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    rev INTEGER NOT NULL
);

-- Only the SHA-256 of each device's token is kept.
CREATE TABLE devices (
    id TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id),
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE enroll_codes (
    code TEXT PRIMARY KEY,
    member_id TEXT NOT NULL REFERENCES members(id),
    expires_at INTEGER NOT NULL,
    used INTEGER NOT NULL DEFAULT 0
);

-- `direct_key` is the two member IDs of a direct chat, sorted and joined, so there's one chat per pair.
CREATE TABLE chats (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('family', 'direct')),
    direct_key TEXT UNIQUE,
    created_at INTEGER NOT NULL,
    rev INTEGER NOT NULL
);

CREATE TABLE chat_members (
    chat_id TEXT NOT NULL REFERENCES chats(id),
    member_id TEXT NOT NULL REFERENCES members(id),
    read_up_to TEXT,
    PRIMARY KEY (chat_id, member_id)
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL REFERENCES chats(id),
    sender_id TEXT NOT NULL REFERENCES members(id),
    body_json TEXT NOT NULL,
    link_json TEXT,
    created_at INTEGER NOT NULL,
    rev INTEGER NOT NULL
);
CREATE INDEX messages_by_chat ON messages (chat_id, created_at, id);
CREATE INDEX messages_by_rev ON messages (rev);

CREATE TABLE reactions (
    message_id TEXT NOT NULL REFERENCES messages(id),
    member_id TEXT NOT NULL REFERENCES members(id),
    emoji TEXT NOT NULL,
    PRIMARY KEY (message_id, member_id)
);

-- Pictures. The bytes are in `media/<id>` next to the database.
CREATE TABLE media (
    id TEXT PRIMARY KEY,
    uploader_id TEXT,
    content_type TEXT NOT NULL,
    size INTEGER NOT NULL,
    created_at INTEGER NOT NULL
);

-- A note's full JSON (kit_protocol::notes::Note). `shared` and `deleted` are copied out for queries.
CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES members(id),
    json TEXT NOT NULL,
    shared INTEGER NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    rev INTEGER NOT NULL
);
CREATE INDEX notes_by_rev ON notes (rev);
