SET search_path TO pay, public;

DROP FUNCTION IF EXISTS pay.list_app_summaries_bootstrap();

CREATE FUNCTION pay.list_app_summaries_bootstrap()
RETURNS TABLE(
    id UUID,
    slug TEXT,
    display_name TEXT,
    app_url TEXT,
    notes TEXT
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT apps.id, apps.slug, apps.display_name, apps.app_url, apps.notes
    FROM pay.apps
    ORDER BY display_name
$$;

CREATE OR REPLACE FUNCTION pay.update_app_notes_bootstrap(
    p_app_id UUID,
    p_notes TEXT
)
RETURNS TABLE(
    id UUID,
    notes TEXT
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
VOLATILE
AS $$
    UPDATE pay.apps
    SET notes = NULLIF(BTRIM(p_notes), '')
    WHERE apps.id = p_app_id
    RETURNING apps.id, apps.notes
$$;

GRANT EXECUTE ON FUNCTION pay.list_app_summaries_bootstrap() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.update_app_notes_bootstrap(UUID, TEXT) TO bridge_app;
