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
-- 2) Bridge/pay privileges + RLS policies
-- ============================================================
BEGIN;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pay TO bridge_admin, bridge_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pay TO bridge_admin, bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO bridge_admin, bridge_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA pay
  GRANT USAGE, SELECT ON SEQUENCES TO bridge_admin, bridge_app;

ALTER TABLE pay.api_keys             ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.subscriptions        ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.payments             ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_log          ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_delivery     ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.agent_credits        ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.agent_transactions   ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.agent_payment_tokens ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.fraud_prevention     ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS tenant_isolation_api_keys             ON pay.api_keys;
DROP POLICY IF EXISTS tenant_isolation_subscriptions        ON pay.subscriptions;
DROP POLICY IF EXISTS tenant_isolation_payments             ON pay.payments;
DROP POLICY IF EXISTS tenant_isolation_webhook_log          ON pay.webhook_log;
DROP POLICY IF EXISTS tenant_isolation_webhook_delivery     ON pay.webhook_delivery;
DROP POLICY IF EXISTS tenant_isolation_agent_credits        ON pay.agent_credits;
DROP POLICY IF EXISTS tenant_isolation_agent_transactions   ON pay.agent_transactions;
DROP POLICY IF EXISTS tenant_isolation_agent_payment_tokens ON pay.agent_payment_tokens;
DROP POLICY IF EXISTS tenant_isolation_fraud_prevention     ON pay.fraud_prevention;

CREATE POLICY tenant_isolation_api_keys ON pay.api_keys
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

CREATE POLICY tenant_isolation_subscriptions ON pay.subscriptions
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

CREATE POLICY tenant_isolation_payments ON pay.payments
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

CREATE POLICY tenant_isolation_webhook_log ON pay.webhook_log
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

CREATE POLICY tenant_isolation_webhook_delivery ON pay.webhook_delivery
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

CREATE POLICY tenant_isolation_agent_credits ON pay.agent_credits
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

CREATE POLICY tenant_isolation_agent_transactions ON pay.agent_transactions
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

CREATE POLICY tenant_isolation_agent_payment_tokens ON pay.agent_payment_tokens
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

CREATE POLICY tenant_isolation_fraud_prevention ON pay.fraud_prevention
  FOR ALL TO bridge_app
  USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
  WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

COMMIT;

-- ============================================================
-- 3) Hiha privileges + RLS policies
-- ============================================================
BEGIN;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA hiha TO hiha_admin, hiha_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA hiha TO hiha_admin, hiha_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA hiha
  GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO hiha_admin, hiha_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA hiha
  GRANT USAGE, SELECT ON SEQUENCES TO hiha_admin, hiha_app;

ALTER TABLE hiha.users           ENABLE ROW LEVEL SECURITY;
ALTER TABLE hiha.rate_limits     ENABLE ROW LEVEL SECURITY;
ALTER TABLE hiha.callback_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE hiha.notifications   ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS hiha_tenant_isolation_users           ON hiha.users;
DROP POLICY IF EXISTS hiha_tenant_isolation_rate_limits     ON hiha.rate_limits;
DROP POLICY IF EXISTS hiha_tenant_isolation_callback_events ON hiha.callback_events;
DROP POLICY IF EXISTS hiha_tenant_isolation_notifications   ON hiha.notifications;

CREATE POLICY hiha_tenant_isolation_users ON hiha.users
  FOR ALL TO hiha_app
  USING (clerk_id = current_setting('request.jwt.claim.sub', true))
  WITH CHECK (clerk_id = current_setting('request.jwt.claim.sub', true));

CREATE POLICY hiha_tenant_isolation_rate_limits ON hiha.rate_limits
  FOR ALL TO hiha_app
  USING (clerk_id = current_setting('request.jwt.claim.sub', true))
  WITH CHECK (clerk_id = current_setting('request.jwt.claim.sub', true));

CREATE POLICY hiha_tenant_isolation_callback_events ON hiha.callback_events
  FOR ALL TO hiha_app
  USING (clerk_id = current_setting('request.jwt.claim.sub', true))
  WITH CHECK (clerk_id = current_setting('request.jwt.claim.sub', true));

CREATE POLICY hiha_tenant_isolation_notifications ON hiha.notifications
  FOR ALL TO hiha_app
  USING (clerk_id = current_setting('request.jwt.claim.sub', true))
  WITH CHECK (clerk_id = current_setting('request.jwt.claim.sub', true));

COMMIT;

-- Runtime reminders:
-- Bridge runtime connections (bridge_app) must run:
--   SET LOCAL bridge.current_app_id = '<resolved-app-uuid>';
-- Hiha runtime connections (hiha_app) must run:
--   SET LOCAL request.jwt.claim.sub = '<clerk_id>';
