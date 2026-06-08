SET search_path TO pay, public;

-- Bridge runtime role privileges for objects created by migrations.
-- Role/schema creation remains in docs/db-install-roles-rls.sql; this migration
-- keeps the pay schema objects usable by the runtime role after deploys.

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pay TO bridge_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pay TO bridge_app;
GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA pay TO bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
GRANT USAGE, SELECT ON SEQUENCES TO bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
GRANT EXECUTE ON FUNCTIONS TO bridge_app;
