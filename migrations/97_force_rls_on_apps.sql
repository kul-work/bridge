-- Ensure app metadata is tenant-scoped even for table owners.
ALTER TABLE pay.apps FORCE ROW LEVEL SECURITY;
