-- Shared DB install script for Bridge (pay schema) + HouseHold schema
-- Source alignment: docs/pay-tydecode-architecture.md Section 5
-- Run as superuser/postgres in the target database (example: appgen)

-- ============================================================
-- 0) Baseline: schemas + extension
-- ============================================================
BEGIN;

CREATE SCHEMA IF NOT EXISTS pay;
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

  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'household_admin') THEN
    CREATE ROLE household_admin LOGIN BYPASSRLS PASSWORD 'CHANGE_ME';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'household_app') THEN
    CREATE ROLE household_app LOGIN PASSWORD 'CHANGE_ME';
  END IF;
END
$$;

GRANT CONNECT ON DATABASE neondb TO bridge_admin, bridge_app;
GRANT CONNECT ON DATABASE neondb TO household_admin, household_app;

GRANT USAGE, CREATE ON SCHEMA pay TO bridge_admin;
GRANT USAGE ON SCHEMA pay TO bridge_app;
GRANT USAGE, CREATE ON SCHEMA household TO household_admin, household_app;

ALTER ROLE bridge_admin IN DATABASE neondb SET search_path = pay, public;
ALTER ROLE bridge_app   IN DATABASE neondb SET search_path = pay, public;
ALTER ROLE household_admin   IN DATABASE neondb SET search_path = household, public;
ALTER ROLE household_app     IN DATABASE neondb SET search_path = household, public;

COMMIT;

-- neondb_owner needs membership in admin roles to set their default privileges.
GRANT bridge_admin TO neondb_owner;
GRANT household_admin TO neondb_owner;

-- ============================================================
-- Bridge/pay privileges (RLS handled in migrations)
-- ============================================================
BEGIN;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pay TO bridge_admin;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pay TO bridge_admin;

ALTER DEFAULT PRIVILEGES FOR ROLE bridge_admin IN SCHEMA pay
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO bridge_admin;
ALTER DEFAULT PRIVILEGES FOR ROLE bridge_admin IN SCHEMA pay
  GRANT USAGE, SELECT ON SEQUENCES TO bridge_admin;

-- bridge_app table/function privileges are intentionally narrowed by Bridge
-- migrations after all pay objects exist. Do not grant broad runtime access here.

COMMIT;



-- ============================================================
-- HouseHold privileges (RLS handled in migrations)
-- ============================================================
BEGIN;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA household TO household_admin, household_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA household TO household_admin, household_app;

ALTER DEFAULT PRIVILEGES FOR ROLE household_admin IN SCHEMA household
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO household_admin, household_app;
ALTER DEFAULT PRIVILEGES FOR ROLE household_admin IN SCHEMA household
  GRANT USAGE, SELECT ON SEQUENCES TO household_admin, household_app;

COMMIT;

-- Runtime reminders:
-- Bridge runtime connections (bridge_app) must run:
--   SET LOCAL bridge.current_app_id = '<resolved-app-uuid>';
-- HouseHold runtime connections (household_app) must run:
--   SET LOCAL request.jwt.claim.sub = '<clerk_id>';
