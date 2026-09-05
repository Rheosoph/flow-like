#!/usr/bin/env bash
# =============================================================================
# Flow-Like Kubernetes - Database Migration
# =============================================================================
# This script runs database migrations using Prisma against a PostgreSQL database
# Can be run locally or as a Kubernetes Job
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
API_PKG_DIR="$SCRIPT_DIR/../../../../packages/api"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Load .env file if it exists
if [[ -f "$SCRIPT_DIR/../.env" ]]; then
    log_info "Loading environment from .env file..."
    set -a
    source "$SCRIPT_DIR/../.env"
    set +a
fi

# Check if we have a database URL
if [[ -z "${DATABASE_URL:-}" ]]; then
    # Build from components
    POSTGRES_USER="${POSTGRES_USER:-flowlike}"
    POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-flowlike_dev}"
    POSTGRES_HOST="${POSTGRES_HOST:-localhost}"
    POSTGRES_PORT="${POSTGRES_PORT:-5432}"
    POSTGRES_DB="${POSTGRES_DB:-flowlike}"
    export DATABASE_URL="postgresql://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${POSTGRES_DB}"
fi

log_info "Database connection configured"

# Check for required tools
check_tools() {
    local missing=()

    if ! command -v bun &>/dev/null && ! command -v npx &>/dev/null; then
        missing+=("bun or npx")
    fi

    if [[ ${#missing[@]} -gt 0 ]]; then
        log_error "Missing required tools: ${missing[*]}"
        log_info "Please install the missing tools or use the Docker-based migration"
        exit 1
    fi
}

# Run migrations using the packages/api Prisma setup
run_migrations() {
    log_info "Running database migrations..."

    cd "$API_PKG_DIR"

    # Install dependencies if needed
    if [[ ! -d "node_modules" ]]; then
        log_info "Installing dependencies..."
        if command -v bun &>/dev/null; then
            bun install
        else
            npm install
        fi
    fi

    # Create PostgreSQL mirror of the schema (convert from CockroachDB)
    log_info "Creating PostgreSQL schema mirror..."
    if [[ -f "scripts/make-postgres-prisma-mirror.sh" ]]; then
        bash scripts/make-postgres-prisma-mirror.sh
    else
        log_warn "make-postgres-prisma-mirror.sh not found, using schema directly"
    fi

    # Determine which schema to use
    local schema_path="prisma/schema"
    if [[ -d "prisma-postgres-mirror/schema" ]]; then
        schema_path="prisma-postgres-mirror/schema"
    fi

    # Column type changes prisma db push emits without the USING clause they
    # need on an existing database (enum -> TEXT, array -> JSONB). Guarded and
    # idempotent; a no-op on a fresh database. The runner is TypeScript and
    # relies on import.meta.main, so without bun it needs Node >= 24.2.
    log_info "Applying pre-push SQL..."
    if command -v bun &>/dev/null; then
        bun prisma/pre-push.ts
    elif node --input-type=module -e 'process.exit("main" in import.meta ? 0 : 1)' 2>/dev/null; then
        node prisma/pre-push.ts
    else
        log_error "prisma/pre-push.ts needs bun or Node >= 24.2 (type stripping and import.meta.main)"
        exit 1
    fi

    # Push schema to database
    log_info "Pushing schema to database..."
    if command -v bun &>/dev/null; then
        bunx prisma db push --schema="$schema_path" --accept-data-loss
    else
        npx prisma db push --schema="$schema_path" --accept-data-loss
    fi

    log_info "Database migrations completed successfully!"

    # Cleanup
    if [[ -d "prisma-postgres-mirror" ]]; then
        log_info "Cleaning up temporary files..."
        rm -rf prisma-postgres-mirror
    fi
}

# Run migrations via Docker (useful when local tools aren't available)
run_migrations_docker() {
    log_info "Running database migrations via Docker..."

    cd "$SCRIPT_DIR/.."

    docker compose run --rm db-migrate

    log_info "Database migrations completed successfully!"
}

# Main
main() {
    local use_docker=false

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --docker)
                use_docker=true
                shift
                ;;
            --help|-h)
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --docker    Run migrations using Docker instead of local tools"
                echo "  --help      Show this help message"
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    if [[ "$use_docker" == "true" ]]; then
        run_migrations_docker
    else
        check_tools
        run_migrations
    fi
}

main "$@"
