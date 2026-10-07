-- Migration 20250101000008_worker_credentials_hardening.sql

-- 1. Add registration token hash and expiry columns
ALTER TABLE task_worker_pcs
    ADD COLUMN IF NOT EXISTS registration_token_hash BYTEA UNIQUE,
    ADD COLUMN IF NOT EXISTS registration_token_expires_at TIMESTAMPTZ;

-- Migrate any existing plaintext registration_token to registration_token_hash using native sha256
UPDATE task_worker_pcs
SET registration_token_hash = sha256(convert_to(registration_token, 'UTF8')),
    registration_token_expires_at = now() + INTERVAL '24 hours'
WHERE registration_token IS NOT NULL AND registration_token_hash IS NULL;

ALTER TABLE task_worker_pcs DROP COLUMN IF EXISTS registration_token;

-- 2. Allow api_key_hash to be NULL instead of empty bytea
ALTER TABLE task_worker_pcs ALTER COLUMN api_key_hash DROP NOT NULL;

UPDATE task_worker_pcs
SET api_key_hash = NULL
WHERE octet_length(api_key_hash) = 0;

CREATE UNIQUE INDEX IF NOT EXISTS idx_task_worker_pcs_api_key_hash
    ON task_worker_pcs (api_key_hash)
    WHERE api_key_hash IS NOT NULL;

-- 3. Add previous API key hash and expiry columns for rotation overlap
ALTER TABLE task_worker_pcs
    ADD COLUMN IF NOT EXISTS previous_api_key_hash BYTEA,
    ADD COLUMN IF NOT EXISTS previous_api_key_expires_at TIMESTAMPTZ;
