-- The family's other hubs this hub keeps in step with (docs/multi_hub_plan.md). A travel hub calls its home hub
-- (`url`), keeps the secret they share (it has to send it) and how far it has got each way; the home hub keeps only
-- the secret's hash, and when it last heard from the travel hub.
CREATE TABLE peers (
    hub_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT,
    secret TEXT,
    secret_hash TEXT NOT NULL,
    -- The other hub's change number this hub has taken in everything up to.
    pulled_rev INTEGER NOT NULL DEFAULT 0,
    -- This hub's change number the other hub has taken in everything up to.
    pushed_rev INTEGER NOT NULL DEFAULT 0,
    last_synced INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

-- One-time codes for pairing another hub, made by a parent.
CREATE TABLE hub_codes (
    code TEXT PRIMARY KEY,
    expires_at INTEGER NOT NULL,
    used INTEGER NOT NULL DEFAULT 0
);
