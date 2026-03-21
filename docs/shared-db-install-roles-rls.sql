-- Shared DB install script for Bridge (pay schema) + Hiha (hiha schema)
-- Source alignment: docs/pay-tydecode-architecture.md Section 5
-- Run as superuser/postgres in the target database (example: tyde)

-- ============================================================
-- 0) Baseline: schemas + extension
-- ============================================================
BEGIN;

CREATE SCHEMA IF NOT EXISTS pay;
CREATE SCHEMA IF NOT EXISTS hiha;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

COMMIT;

-- ============================================================
-- 1) Roles + schema access + search_path
-- ============================================================
BEGIN;

DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'bridge_admin') THEN
    CREATE ROLE bridge_admin LOGIN BYPASSRLS PASSWORD 'CHANGE_ME';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'bridge_app') THEN
    CREATE ROLE bridge_app LOGIN PASSWORD 'CHANGE_ME';
  END IF;

  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'hiha_admin') THEN
    CREATE ROLE hiha_admin LOGIN BYPASSRLS PASSWORD 'CHANGE_ME';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'hiha_app') THEN
    CREATE ROLE hiha_app LOGIN PASSWORD 'CHANGE_ME';
  END IF;
END
$$;

-- Replace "tyde" if your DB name is different.
GRANT CONNECT ON DATABASE tyde TO bridge_admin, bridge_app, hiha_admin, hiha_app;

GRANT USAGE ON SCHEMA pay TO bridge_admin, bridge_app;
GRANT USAGE ON SCHEMA hiha TO hiha_admin, hiha_app;

ALTER ROLE bridge_admin IN DATABASE tyde SET search_path = pay, public;
ALTER ROLE bridge_app   IN DATABASE tyde SET search_path = pay, public;
ALTER ROLE hiha_admin   IN DATABASE tyde SET search_path = hiha, public;
ALTER ROLE hiha_app     IN DATABASE tyde SET search_path = hiha, public;

COMMIT;

-- ============================================================
-- 2) Bridge/pay privileges (RLS handled in migrations)
-- ============================================================
BEGIN;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pay TO bridge_admin, bridge_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pay TO bridge_admin, bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO bridge_admin, bridge_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA pay
  GRANT USAGE, SELECT ON SEQUENCES TO bridge_admin, bridge_app;

COMMIT;

-- ============================================================
-- 3) Hiha privileges (RLS handled in migrations)
-- ============================================================
BEGIN;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA hiha TO hiha_admin, hiha_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA hiha TO hiha_admin, hiha_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA hiha
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO hiha_admin, hiha_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA hiha
  GRANT USAGE, SELECT ON SEQUENCES TO hiha_admin, hiha_app;

COMMIT;

-- Runtime reminders:
-- Bridge runtime connections (bridge_app) must run:
--   SET LOCAL bridge.current_app_id = '<resolved-app-uuid>';
-- Hiha runtime connections (hiha_app) must run:
--   SET LOCAL request.jwt.claim.sub = '<clerk_id>';
-- RLS policies are installed via migrations (see migrations/11_enable_row_level_security.sql)
