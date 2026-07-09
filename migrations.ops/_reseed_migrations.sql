-- Bridge: re-populate _sqlx_migrations after truncate
-- Schema is already applied; these rows just tell sqlx not to re-run them.
-- Checksum is empty bytea — sqlx will warn but won't re-execute.

INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES
(0,  'enable pgcrypto',                              NOW(), true, ''::bytea, 0),
(1,  'create apps and api keys',                     NOW(), true, ''::bytea, 0),
(2,  'create subscriptions',                         NOW(), true, ''::bytea, 0),
(3,  'create payments',                              NOW(), true, ''::bytea, 0),
(4,  'create webhooks',                              NOW(), true, ''::bytea, 0),
(5,  'create provider configs checkout and fraud',   NOW(), true, ''::bytea, 0),
(6,  'create indexes and retention view',            NOW(), true, ''::bytea, 0),
(7,  'webhook provider recovery claim',              NOW(), true, ''::bytea, 0),
(8,  'webhook delivery canonical payload',           NOW(), true, ''::bytea, 0),
(9,  'worker claims',                                NOW(), true, ''::bytea, 0),
(10, 'subscription status check',                    NOW(), true, ''::bytea, 0),
(90, 'enable row level security',                    NOW(), true, ''::bytea, 0),
(91, 'fix rls current app id cast',                  NOW(), true, ''::bytea, 0),
(92, 'enable checkout idempotency rls',              NOW(), true, ''::bytea, 0);
