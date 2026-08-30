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
RUST_CACHE_PREFIX="flow-like-check-rust-1-97-1"
CARGO_REGISTRY_VOLUME="${RUST_CACHE_PREFIX}-registry"
CARGO_GIT_VOLUME="${RUST_CACHE_PREFIX}-git"
LINUX_TARGET_VOLUME="${RUST_CACHE_PREFIX}-linux-target"
WINDOWS_TARGET_VOLUME="${RUST_CACHE_PREFIX}-windows-gnu-target"
WINDOWS_STACK_DIRECTIVE="cargo::rustc-link-arg-bin=flow-like-desktop=/STACK:8388608"

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
    if cargo check --locked -p "$PACKAGE" 2>&1; then
        log "macOS check: ${GREEN}PASSED${NC}"
        return 0
    else
        fail "macOS check: FAILED"
        return 1
    fi
}

ensure_docker_image() {
    local dockerfile_hash toolchain_hash docker_platform image_key
    if command -v md5sum >/dev/null 2>&1; then
        dockerfile_hash=$(md5sum "$SCRIPT_DIR/check-linux.Dockerfile")
        dockerfile_hash="${dockerfile_hash%% *}"
        toolchain_hash=$(md5sum "$ROOT_DIR/rust-toolchain.toml")
        toolchain_hash="${toolchain_hash%% *}"
    else
        dockerfile_hash=$(md5 -q "$SCRIPT_DIR/check-linux.Dockerfile")
        toolchain_hash=$(md5 -q "$ROOT_DIR/rust-toolchain.toml")
    fi
    docker_platform=$(docker info --format '{{.OSType}}-{{.Architecture}}' 2>/dev/null || echo unknown)
    image_key="${dockerfile_hash}-${toolchain_hash}-${docker_platform}"

    local label_hash=""
    if docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
        label_hash=$(docker inspect --format='{{index .Config.Labels "build.inputs"}}' "$IMAGE_NAME" 2>/dev/null || true)
    fi

    if [ "$image_key" != "$label_hash" ] || [ "${FLOW_LIKE_CHECK_DOCKER_PULL:-0}" = "1" ]; then
        local pull_args=()
        if [ "${FLOW_LIKE_CHECK_DOCKER_PULL:-0}" = "1" ]; then
            pull_args=(--pull)
        fi
        log "Building Docker image '$IMAGE_NAME' (first run or build-input change)..."
        docker build "${pull_args[@]}" -t "$IMAGE_NAME" \
            --label "build.inputs=$image_key" \
            -f "$SCRIPT_DIR/check-linux.Dockerfile" "$ROOT_DIR"
    fi
}

verify_windows_stack_directive() {
    local build_script="$ROOT_DIR/apps/desktop/src-tauri/build.rs"
    if ! grep -Fq "$WINDOWS_STACK_DIRECTIVE" "$build_script"; then
        fail "Required Windows MSVC stack directive is missing from apps/desktop/src-tauri/build.rs"
        return 1
    fi
    log "Required Windows MSVC /STACK:8388608 directive is present"
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
        -v "$CARGO_REGISTRY_VOLUME:/root/.cargo/registry" \
        -v "$CARGO_GIT_VOLUME:/root/.cargo/git" \
        -v "$LINUX_TARGET_VOLUME:/cargo-target" \
        -e CARGO_TARGET_DIR=/cargo-target \
        "$IMAGE_NAME" \
        cargo check --locked -p "$PACKAGE" 2>&1; then
        log "Linux check: ${GREEN}PASSED${NC}"
        return 0
    else
        fail "Linux check: FAILED"
        return 1
    fi
}

check_windows() {
    log "── Windows (cross-check via Docker + MinGW) ──"

    verify_windows_stack_directive || return 1

    if ! command -v docker >/dev/null 2>&1; then
        fail "Docker not found. Install Docker to run Windows cross-checks."
        return 1
    fi
    if ! docker info >/dev/null 2>&1; then
        fail "Docker daemon not running."
        return 1
    fi

    ensure_docker_image

    warn "MinGW cargo check validates Windows Rust cfgs only; it does not final-link the MSVC binary or exercise /STACK. The Windows release build performs that validation."
    log "Running cargo check --target x86_64-pc-windows-gnu in Linux container..."
    if docker run --rm \
        -v "$ROOT_DIR:/workspace:ro" \
        -v "$CARGO_REGISTRY_VOLUME:/root/.cargo/registry" \
        -v "$CARGO_GIT_VOLUME:/root/.cargo/git" \
        -v "$WINDOWS_TARGET_VOLUME:/cargo-target" \
        -e CARGO_TARGET_DIR=/cargo-target \
        -e "ORT_LIB_LOCATION=/tmp/ort-stub" \
        "$IMAGE_NAME" \
        bash -c "mkdir -p /tmp/ort-stub && cargo check --locked -p $PACKAGE --target x86_64-pc-windows-gnu" 2>&1; then
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
    log "Removing persistent Cargo check volumes..."
    docker volume rm \
        "$CARGO_REGISTRY_VOLUME" \
        "$CARGO_GIT_VOLUME" \
        "$LINUX_TARGET_VOLUME" \
        "$WINDOWS_TARGET_VOLUME" \
        >/dev/null 2>&1 || true
    log "Cleaned up image and Cargo caches."
}

usage() {
    echo "Usage: $0 [macos|linux|windows|all|clean]"
    echo ""
    echo "  macos     Run cargo check natively (macOS/current host)"
    echo "  linux     Run cargo check in Docker (Ubuntu 24.04)"
    echo "  windows   Cross-check for Windows via Docker (MinGW x86_64-pc-windows-gnu)"
    echo "  all       Run all three (default)"
    echo "  clean     Remove the Docker check image and persistent Cargo caches"
    echo ""
    echo "Set FLOW_LIKE_CHECK_DOCKER_PULL=1 to refresh the Ubuntu base image."
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
            return 0
            ;;
        -h|--help|help)
            usage
            return 0
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
