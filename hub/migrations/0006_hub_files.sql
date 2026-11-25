-- Pictures and videos between a family's hubs (docs/multi_hub_plan.md, phase 4). A record can come from another hub
-- before its file does, and a travel hub keeps only some of the files.

-- Whether this hub has the file.
ALTER TABLE media ADD COLUMN have_file INTEGER NOT NULL DEFAULT 1;
-- The file came from another hub, which has it too, so it may be deleted here when space runs short.
ALTER TABLE media ADD COLUMN copied INTEGER NOT NULL DEFAULT 0;
-- For copied files, which go first when space runs short (the oldest): when the picture was taken for copies made in
-- the background, or when it was opened, for files fetched because someone opened them.
ALTER TABLE media ADD COLUMN kept_at INTEGER NOT NULL DEFAULT 0;
UPDATE media SET kept_at = created_at;

-- Files this hub sent to a paired hub, or found it already had, so they aren't offered again.
CREATE TABLE peer_files (
    hub_id TEXT NOT NULL,
    media_id TEXT NOT NULL,
    PRIMARY KEY (hub_id, media_id)
);
