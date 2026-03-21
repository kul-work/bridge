-- Shared DB install script for Bridge (pay schema) + Apps (HiHa, HouseHold schema)
-- Source alignment: docs/pay-tydecode-architecture.md Section 5
-- Run as superuser/postgres in the target database (example: appgen)

-- ============================================================
-- 0) Baseline: schemas + extension
-- ============================================================
BEGIN;

CREATE SCHEMA IF NOT EXISTS pay;
CREATE SCHEMA IF NOT EXISTS hiha;
CREATE SCHEMA IF NOT EXISTS household;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

COMMIT;

-- ============================================================
-- Roles + schema access + search_path
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

  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'household_admin') THEN
    CREATE ROLE household_admin LOGIN BYPASSRLS PASSWORD 'CHANGE_ME';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'household_app') THEN
    CREATE ROLE household_app LOGIN PASSWORD 'CHANGE_ME';
  END IF;
END
$$;

-- Replace "appgen" if your DB name is different.
GRANT CONNECT ON DATABASE appgen TO bridge_admin, bridge_app;
GRANT CONNECT ON DATABASE appgen TO hiha_admin, hiha_app, household_admin, household_app;

GRANT USAGE ON SCHEMA pay TO bridge_admin, bridge_app;
GRANT USAGE ON SCHEMA hiha TO hiha_admin, hiha_app;
GRANT USAGE ON SCHEMA household TO household_admin, household_app;

ALTER ROLE bridge_admin IN DATABASE appgen SET search_path = pay, public;
ALTER ROLE bridge_app   IN DATABASE appgen SET search_path = pay, public;
ALTER ROLE hiha_admin   IN DATABASE appgen SET search_path = hiha, public;
ALTER ROLE hiha_app     IN DATABASE appgen SET search_path = hiha, public;
ALTER ROLE household_admin   IN DATABASE appgen SET search_path = household, public;
ALTER ROLE household_app     IN DATABASE appgen SET search_path = household, public;

COMMIT;

-- ============================================================
-- Bridge/pay privileges (RLS handled in migrations)
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
-- HiHa privileges (RLS handled in migrations)
-- ============================================================
BEGIN;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA hiha TO hiha_admin, hiha_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA hiha TO hiha_admin, hiha_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA hiha
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO hiha_admin, hiha_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA hiha
  GRANT USAGE, SELECT ON SEQUENCES TO hiha_admin, hiha_app;

COMMIT;



-- ============================================================
-- HouseHold privileges (RLS handled in migrations)
-- ============================================================
BEGIN;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA household TO household_admin, household_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA household TO household_admin, household_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA household
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO household_admin, household_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA household
  GRANT USAGE, SELECT ON SEQUENCES TO household_admin, household_app;

COMMIT;

-- Runtime reminders:
-- Bridge runtime connections (bridge_app) must run:
--   SET LOCAL bridge.current_app_id = '<resolved-app-uuid>';
-- HiHa runtime connections (hiha_app) must run:
--   SET LOCAL request.jwt.claim.sub = '<clerk_id>';
-- HouseHold runtime connections (household_app) must run:
--   SET LOCAL request.jwt.claim.sub = '<clerk_id>';
