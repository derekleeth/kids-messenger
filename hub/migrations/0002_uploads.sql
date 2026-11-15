-- Big uploads (full-size photos and videos) in progress. The bytes so far are in `uploads/<id>` next to the database.
CREATE TABLE uploads (
    id TEXT PRIMARY KEY,
    uploader_id TEXT NOT NULL,
    size INTEGER NOT NULL,
    received INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
