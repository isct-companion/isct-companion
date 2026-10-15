#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: scripts/data/migrate-v3-to-v4.sh /absolute/path/to/school.db" >&2
  exit 2
fi

database=$1
case "$database" in
  /*) ;;
  *) echo "database path must be absolute" >&2; exit 2 ;;
esac
if [ ! -f "$database" ]; then
  echo "database does not exist: $database" >&2
  exit 2
fi

version=$(sqlite3 "$database" 'PRAGMA user_version;')
if [ "$version" -ne 3 ]; then
  echo "expected schema 3, found $version" >&2
  exit 2
fi

backup="${database}.v3-backup-$(date -u +%Y%m%dT%H%M%SZ)"
sqlite3 "$database" ".backup '$backup'"
sqlite3 "$database" < "$(dirname "$0")/migrate-v3-to-v4.sql"
sqlite3 "$database" 'PRAGMA integrity_check; PRAGMA user_version;'
echo "backup: $backup"
