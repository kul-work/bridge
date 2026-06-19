ALTER TABLE pay.apps
  ADD COLUMN IF NOT EXISTS notes TEXT;

COMMENT ON COLUMN pay.apps.notes IS 'Internal notes about the app.';
