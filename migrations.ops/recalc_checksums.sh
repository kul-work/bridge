#!/bin/bash
# Recalculate sqlx migration checksums from actual .sql files and update _sqlx_migrations.
# Usage:
#   bash recalc_checksums.sh                          # auto-detect from ../migrations
#   bash recalc_checksums.sh <migrations_dir> <db_user> <db_name>
# Example:
#   bash recalc_checksums.sh c:/share/tyde/bridge/migrations bridge_admin appgen

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Auto-detect: no args → use sibling ../migrations directory
if [[ $# -eq 0 ]]; then
    MIG_DIR="$SCRIPT_DIR/../migrations"
    # Try to auto-detect DB credentials from sibling .env file
    PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
    if [[ -f "$PROJECT_DIR/.env" ]]; then
        DB_URL=$(grep -oP '(?<=DATABASE_URL=postgresql://)[^@]+@[^/]+/\w+' "$PROJECT_DIR/.env" 2>/dev/null || true)
        if [[ -n "$DB_URL" ]]; then
            DB_USER=$(echo "$DB_URL" | grep -oP '^\w+' || echo "postgres")
            DB_NAME=$(echo "$DB_URL" | grep -oP '\w+$' || echo "appgen")
        else
            DB_USER="postgres"
            DB_NAME="appgen"
        fi
    else
        DB_USER="postgres"
        DB_NAME="appgen"
    fi
    echo "Auto-detected: migrations=$MIG_DIR db_user=$DB_USER db_name=$DB_NAME"
else
    MIG_DIR="${1:?Usage: $0 [migrations_dir] [db_user] [db_name]}"
    DB_USER="${2:-postgres}"
    DB_NAME="${3:-appgen}"
fi

DB_HOST="${DB_HOST:-localhost}"
DB_PORT="${DB_PORT:-5432}"
TMP_SQL="$(mktemp)"

echo "-- Auto-generated checksum reseed" > "$TMP_SQL"

for f in "$MIG_DIR"/[0-9]*.sql; do
    [[ -f "$f" ]] || continue
    filename=$(basename "$f")
    version=$(echo "$filename" | grep -oP '^\d+' | head -1)
    description=$(echo "$filename" | sed 's/^[0-9]*_//; s/\.sql$//')
    # sqlx uses SHA384 of file content
    checksum_hex=$(sha384sum "$f" | awk '{print $1}')
    echo "UPDATE _sqlx_migrations SET checksum = decode('${checksum_hex}', 'hex'), description = '${description}' WHERE version = ${version};" >> "$TMP_SQL"
done

echo ""
echo "Generated SQL:"
cat "$TMP_SQL"
echo ""

cmd //c "set PGPASSWORD=postgres&& psql -U ${DB_USER} -h ${DB_HOST} -p ${DB_PORT} -d ${DB_NAME} -f $(cygpath -w "$TMP_SQL" 2>/dev/null || echo "$TMP_SQL")"

rm "$TMP_SQL"
echo "Done."
