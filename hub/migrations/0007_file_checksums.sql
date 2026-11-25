-- A checksum for each picture's file, so a hub copying it from another can tell it arrived whole
-- (docs/multi_hub_plan.md, phase 4).

-- SHA-256 of the file as kept (after its hidden data was removed), in hex. Filled in for older pictures when the hub
-- starts.
ALTER TABLE media ADD COLUMN sha256 TEXT;
-- When copying the file to or from another hub last came out wrong; it isn't tried again in the background for a day.
ALTER TABLE media ADD COLUMN copy_failed_at INTEGER NOT NULL DEFAULT 0;
