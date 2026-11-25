-- Groundwork for a family with several hubs that keep each other in step (docs/multi_hub_plan.md).

-- This hub. `hub_id` is made the first time the hub starts, and made again if its data turns up somewhere else
-- (`origin`, the data folder and the computer's name), so a copy of the data isn't taken for the hub it came from.
CREATE TABLE hub (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    hub_id TEXT NOT NULL,
    origin TEXT NOT NULL
);

-- The family, the same on all of its hubs.
ALTER TABLE family ADD COLUMN family_id TEXT;
UPDATE family SET family_id = lower(hex(randomblob(16))) WHERE family_id IS NULL;

-- When a member's name, colour or picture last changed, so two hubs' edits can be settled by the newest.
ALTER TABLE members ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
UPDATE members SET updated_at = created_at;

-- When a reaction last changed. A reaction taken away is kept with an empty `emoji`, so another hub's older copy can't
-- bring it back.
ALTER TABLE reactions ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;

-- Change numbers for devices (joined, removed) and pictures, so another hub can ask what changed.
ALTER TABLE devices ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
ALTER TABLE media ADD COLUMN rev INTEGER NOT NULL DEFAULT 0;
