#!/usr/bin/env bash
set -euo pipefail

if [[ -n "${DATABASE_URL_FILE:-}" ]]; then
    [[ -z "${DATABASE_URL:-}" ]] || { echo 'Set either DATABASE_URL or DATABASE_URL_FILE' >&2; exit 1; }
    DATABASE_URL="$(<"$DATABASE_URL_FILE")"
fi

if [[ -z "${DATABASE_URL:-}" ]]; then
    # Encode bytes, including URI delimiters in generated passwords. Keep the
    # authority supplied by the operator separate from user and database names.
    uri_component() {
        local LC_ALL=C value="$1" result='' char encoded index code
        for ((index=0; index<${#value}; index++)); do
            char="${value:index:1}"
            case "$char" in
                [a-zA-Z0-9.~_-]) result+="$char" ;;
                *) printf -v code '%d' "'$char"; printf -v encoded '%%%02X' "$((code & 255))"; result+="$encoded" ;;
            esac
        done
        REPLY="$result"
    }
    if [[ -n "${POSTGRES_PASSWORD_FILE:-}" ]]; then
        [[ -z "${POSTGRES_PASSWORD:-}" ]] || { echo 'Set either POSTGRES_PASSWORD or POSTGRES_PASSWORD_FILE' >&2; exit 1; }
        POSTGRES_PASSWORD="$(<"$POSTGRES_PASSWORD_FILE")"
    fi
    : "${POSTGRES_PASSWORD:?POSTGRES_PASSWORD or DATABASE_URL is required}"
    uri_component "${POSTGRES_USER:-postgres}"; db_user="$REPLY"
    uri_component "$POSTGRES_PASSWORD"; db_password="$REPLY"
    uri_component "${POSTGRES_DB:-app}"; db_name="$REPLY"
    DATABASE_URL="postgresql://${db_user}:${db_password}@${POSTGRES_HOST:-postgres}:${POSTGRES_PORT:-5432}/${db_name}"
fi

export DATABASE_URL
unset DATABASE_URL_FILE
exec "$@"
