-- Devices and pictures from before they had change numbers (0003) get the family's current one, so a hub asking for
-- every change since the start (a new travel hub) gets them too.
UPDATE devices SET rev = COALESCE((SELECT rev FROM family WHERE id = 1), 0) WHERE rev = 0;
UPDATE media SET rev = COALESCE((SELECT rev FROM family WHERE id = 1), 0) WHERE rev = 0;
