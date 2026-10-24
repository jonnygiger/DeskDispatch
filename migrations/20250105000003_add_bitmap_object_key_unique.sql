-- The bitmap upload commit path is idempotent and upserts on
-- object_storage_key, so the key must be unique.
CREATE UNIQUE INDEX IF NOT EXISTS bitmaps_object_storage_key_key
    ON bitmaps (object_storage_key);
