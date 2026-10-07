-- Scheduler Improvements Migration: Add overlap policy and max queue age expiry columns to schedules
ALTER TABLE schedules
    ADD COLUMN overlap_policy TEXT NOT NULL DEFAULT 'allow' CHECK (overlap_policy IN ('allow', 'skip', 'queue')),
    ADD COLUMN max_queue_age_secs INTEGER;
