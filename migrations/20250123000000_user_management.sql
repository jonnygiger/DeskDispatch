-- Migration for User Management & Forced Password Change

ALTER TABLE users ADD COLUMN IF NOT EXISTS must_change_password BOOLEAN NOT NULL DEFAULT false;

-- Force password change for default admin/bootstrap account if present
UPDATE users SET must_change_password = true WHERE username = 'admin';
