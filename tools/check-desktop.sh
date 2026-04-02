#!/usr/bin/env bash
set -euo pipefail

# ─── Cross-platform cargo check for the desktop app ───
# Usage:
#   ./tools/check-desktop.sh              # all platforms
#   ./tools/check-desktop.sh macos        # macOS only (native)
#   ./tools/check-desktop.sh linux        # Linux only (Docker)
#   ./tools/check-desktop.sh windows      # Windows only (Docker + MinGW)
#   ./tools/check-desktop.sh all          # macOS + Linux + Windows

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE_NAME="flow-like-check-linux"
PACKAGE="flow-like-desktop"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()  { echo -e "${GREEN}[check]${NC} $*"; }
warn() { echo -e "${YELLOW}[check]${NC} $*"; }
fail() { echo -e "${RED}[check]${NC} $*"; }

check_macos() {
    log "── macOS (native) ──"
    cd "$ROOT_DIR"
    if cargo check -p "$PACKAGE" 2>&1; then
        log "macOS check: ${GREEN}PASSED${NC}"
        return 0
    else
        fail "macOS check: FAILED"
        return 1
    fi
}

ensure_docker_image() {
    if ! docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
        log "Building Docker image '$IMAGE_NAME' (first run only)..."
        docker build -t "$IMAGE_NAME" -f "$SCRIPT_DIR/check-linux.Dockerfile" "$ROOT_DIR"
    else
        # Rebuild if Dockerfile changed
        local dockerfile_hash
        dockerfile_hash=$(md5sum "$SCRIPT_DIR/check-linux.Dockerfile" 2>/dev/null || md5 -q "$SCRIPT_DIR/check-linux.Dockerfile")
        local label_hash
        label_hash=$(docker inspect --format='{{index .Config.Labels "dockerfile.hash"}}' "$IMAGE_NAME" 2>/dev/null || echo "")
        if [ "$dockerfile_hash" != "$label_hash" ]; then
            log "Dockerfile changed, rebuilding image..."
            docker build -t "$IMAGE_NAME" \
                --label "dockerfile.hash=$dockerfile_hash" \
                -f "$SCRIPT_DIR/check-linux.Dockerfile" "$ROOT_DIR"
        fi
    fi
}

check_linux() {
    log "── Linux (Docker) ──"

    if ! command -v docker >/dev/null 2>&1; then
        fail "Docker not found. Install Docker to run Linux checks."
        return 1
    fi
    if ! docker info >/dev/null 2>&1; then
        fail "Docker daemon not running."
        return 1
    fi

    ensure_docker_image

    log "Running cargo check in Linux container..."
    if docker run --rm \
        -v "$ROOT_DIR:/workspace:ro" \
        -e CARGO_TARGET_DIR=/tmp/cargo-target \
        -e "RUSTFLAGS=--cfg tokio_unstable" \
        "$IMAGE_NAME" \
        cargo check -p "$PACKAGE" 2>&1; then
        log "Linux check: ${GREEN}PASSED${NC}"
        return 0
    else
        fail "Linux check: FAILED"
        return 1
    fi
}

check_windows() {
    log "── Windows (cross-check via Docker + MinGW) ──"

    if ! command -v docker >/dev/null 2>&1; then
        fail "Docker not found. Install Docker to run Windows cross-checks."
        return 1
    fi
    if ! docker info >/dev/null 2>&1; then
        fail "Docker daemon not running."
        return 1
    fi

    ensure_docker_image

    log "Running cargo check --target x86_64-pc-windows-gnu in Linux container..."
    if docker run --rm \
        -v "$ROOT_DIR:/workspace" \
        -e CARGO_TARGET_DIR=/tmp/cargo-target \
        -e "RUSTFLAGS=--cfg tokio_unstable" \
        -e "ORT_LIB_LOCATION=/tmp/ort-stub" \
        "$IMAGE_NAME" \
        bash -c "mkdir -p /tmp/ort-stub && cargo check -p $PACKAGE --target x86_64-pc-windows-gnu" 2>&1; then
        log "Windows cross-check: ${GREEN}PASSED${NC}"
        return 0
    else
        fail "Windows cross-check: FAILED"
        warn "Note: Some C sys-crates may fail in cross-check but work on real Windows."
        warn "If only sys-crate build scripts fail, the Rust code itself is likely fine."
        return 1
    fi
}

cleanup_image() {
    log "Removing Docker image '$IMAGE_NAME'..."
    docker rmi "$IMAGE_NAME" 2>/dev/null || true
    log "Cleaned up."
}

usage() {
    echo "Usage: $0 [macos|linux|windows|all|clean]"
    echo ""
    echo "  macos     Run cargo check natively (macOS/current host)"
    echo "  linux     Run cargo check in Docker (Ubuntu 24.04)"
    echo "  windows   Cross-check for Windows via Docker (MinGW x86_64-pc-windows-gnu)"
    echo "  all       Run all three (default)"
    echo "  clean     Remove the Docker check image"
}

main() {
    local target="${1:-all}"
    local exit_code=0

    case "$target" in
        macos|host|native)
            check_macos || exit_code=1
            ;;
        linux|docker)
            check_linux || exit_code=1
            ;;
        windows|win)
            check_windows || exit_code=1
            ;;
        all|"")
            check_macos || exit_code=1
            check_linux || exit_code=1
            check_windows || exit_code=1
            ;;
        clean)
            cleanup_image
            ;;
        -h|--help|help)
            usage
            ;;
        *)
            fail "Unknown target: $target"
            usage
            exit 1
            ;;
    esac

    echo ""
    if [ $exit_code -eq 0 ]; then
        log "All checks ${GREEN}PASSED${NC}"
    else
        fail "Some checks FAILED"
    fi
    exit $exit_code
}

main "$@"
