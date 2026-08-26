#!/usr/bin/env bash
set -euo pipefail

# Compile feature bundles independently and enforce dependency-light crate
# boundaries. Separate Cargo invocations are intentional: a workspace-wide
# check can hide missing feature declarations through feature unification.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

MODE="all"
TARGET_DIR="$ROOT_DIR/target/feature-boundaries"
CARGO_CONFIG=""
OFFLINE=0
DRY_RUN=0
DISABLE_RUSTC_WRAPPER=0
BOUNDARIES=()
CHECK_ARGS=()
FORBIDDEN=()
REQUIRED=("")
TREE_FILE=""
DEPENDENCY_ONLY=0

usage() {
    cat <<'EOF'
Usage: tools/check-feature-boundaries.sh [OPTIONS] [BOUNDARY ...]

Check dependency-light crates and product feature bundles in isolated Cargo
invocations. With no boundary names, every boundary is checked.

Options:
  --mode MODE              all, check, or deps (default: all)
                           check = cargo check only; deps = assertions only
  --target-dir PATH        Dedicated check target directory
  --cargo-config PATH      Pass `--config PATH` to Cargo
  --offline                Pass `--offline` to Cargo
  --no-rustc-wrapper       Disable Rust compiler wrappers for this run
  --dry-run                Print checks and assertions without running Cargo
  --list                   List available boundaries
  -h, --help               Show this help

Examples:
  tools/check-feature-boundaries.sh storage-contracts wasm-schema
  tools/check-feature-boundaries.sh --mode deps
  tools/check-feature-boundaries.sh --offline catalog-server

`cargo tree` assertions use normal and build dependencies across all targets,
so unrelated dev-dependencies cannot create false positives and target-only
regressions cannot hide behind the current host.
EOF
}

list_boundaries() {
    cat <<'EOF'
core-contracts
dev-default
core-flow
types-contracts
types-proto
storage-contracts
storage-files
model-protocol
wasm-schema
wasm-schema-nodes
wasm-host
api-entity
api-runtime
catalog-portable
catalog-server
catalog-server-local-ml
EOF
}

fail() {
    echo "[feature-boundaries] error: $*" >&2
    exit 1
}

log() {
    echo "[feature-boundaries] $*"
}

cleanup() {
    if [ -n "$TREE_FILE" ] && [ -f "$TREE_FILE" ]; then
        rm -f "$TREE_FILE"
    fi
}
trap cleanup EXIT

while [ "$#" -gt 0 ]; do
    case "$1" in
        --mode)
            [ "$#" -ge 2 ] || fail "--mode requires a value"
            MODE="$2"
            shift 2
            ;;
        --target-dir)
            [ "$#" -ge 2 ] || fail "--target-dir requires a path"
            TARGET_DIR="$2"
            shift 2
            ;;
        --cargo-config)
            [ "$#" -ge 2 ] || fail "--cargo-config requires a path"
            CARGO_CONFIG="$2"
            shift 2
            ;;
        --offline)
            OFFLINE=1
            shift
            ;;
        --no-rustc-wrapper)
            DISABLE_RUSTC_WRAPPER=1
            shift
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --list)
            list_boundaries
            exit 0
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        -*)
            fail "unknown option: $1"
            ;;
        *)
            BOUNDARIES+=("$1")
            shift
            ;;
    esac
done

case "$MODE" in
    all|check|deps) ;;
    *) fail "invalid mode '$MODE' (expected all, check, or deps)" ;;
esac

if [ "${#BOUNDARIES[@]}" -eq 0 ]; then
    BOUNDARIES=(
        core-contracts
        dev-default
        core-flow
        types-contracts
        types-proto
        storage-contracts
        storage-files
        model-protocol
        wasm-schema
        wasm-schema-nodes
        wasm-host
        api-entity
        api-runtime
        catalog-portable
        catalog-server
        catalog-server-local-ml
    )
fi

configure_boundary() {
    local boundary="$1"
    CHECK_ARGS=()
    FORBIDDEN=()
    REQUIRED=("")
    DEPENDENCY_ONLY=0
    case "$boundary" in
        core-contracts)
            CHECK_ARGS=(--package flow-like-core-contracts --no-default-features)
            FORBIDDEN=(flow-like flow-like-storage flow-like-model-provider lancedb datafusion wasmtime tauri)
            ;;
        dev-default)
            # Keep this list in lockstep with workspace.default-members. It
            # makes bare test/clippy coverage explicit in guard output too.
            CHECK_ARGS=(
                --package flow-like-dev-check
                --package flow-like-ast
                --package flow-like-types-contracts
            )
            FORBIDDEN=(
                lancedb
                lance
                lance-core
                lance-io
                lance-file
                arrow
                arrow-array
                arrow-schema
                datafusion
                datafusion-common
                smb2
                smb2-sys
                libsmb2-sys
                ab_glyph
                imageproc
                rxing
            )
            ;;
        core-flow)
            CHECK_ARGS=(--package flow-like --no-default-features --features flow)
            FORBIDDEN=(
                lancedb
                lance
                lance-core
                lance-io
                lance-file
                arrow
                arrow-array
                arrow-schema
                datafusion
                datafusion-common
                smb2
                smb2-sys
                libsmb2-sys
            )
            ;;
        types-contracts)
            CHECK_ARGS=(--package flow-like-types-contracts --no-default-features --features cache,dispatch,maintenance)
            FORBIDDEN=(flow-like flow-like-types flow-like-types-proto flow-like-storage prost lancedb datafusion wasmtime tauri)
            ;;
        types-proto)
            CHECK_ARGS=(--package flow-like-types-proto --no-default-features)
            FORBIDDEN=(flow-like flow-like-types flow-like-storage lancedb datafusion wasmtime tauri)
            ;;
        storage-contracts)
            CHECK_ARGS=(--package flow-like-storage-contracts --no-default-features --features graph,vector)
            FORBIDDEN=(flow-like flow-like-storage lancedb lance lance-core datafusion object_store wasmtime tauri)
            ;;
        storage-files)
            # The baseline deliberately leaves SMB opt-in. Object-store cloud
            # clients belong here; Lance, Arrow, and query execution do not.
            CHECK_ARGS=(--package flow-like-storage-files --no-default-features)
            FORBIDDEN=(
                flow-like-storage
                lancedb
                lance
                lance-core
                lance-io
                lance-file
                arrow
                arrow-array
                arrow-schema
                datafusion
                datafusion-common
                smb2
                smb2-sys
                libsmb2-sys
            )
            ;;
        model-protocol)
            CHECK_ARGS=(--package flow-like-model-protocol --no-default-features)
            FORBIDDEN=(flow-like flow-like-model-provider flow-like-storage fastembed ort candle-core tokenizers wasmtime tauri)
            ;;
        wasm-schema)
            CHECK_ARGS=(--package flow-like-wasm-schema --no-default-features --features bundle,openapi)
            FORBIDDEN=(flow-like flow-like-wasm wasmtime wasmtime-wasi wit-parser wasmparser tauri)
            ;;
        wasm-schema-nodes)
            CHECK_ARGS=(--package flow-like-wasm-schema --no-default-features --features bundle,nodes,openapi)
            FORBIDDEN=(
                flow-like-wasm
                wasmtime
                wasmtime-wasi
                wasmtime-wasi-http
                cranelift-codegen
                wit-parser
                wasmparser
                lancedb
                lance
                datafusion
                tauri
            )
            REQUIRED=(flow-like)
            ;;
        wasm-host)
            # Dependency-only guard used by CI: the host legitimately compiles
            # Wasmtime, but must not regain the database stack through core.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-wasm)
            FORBIDDEN=(
                lancedb
                lance
                lance-core
                lance-io
                lance-file
                arrow
                arrow-array
                arrow-schema
                datafusion
                datafusion-common
                smb2
                smb2-sys
                libsmb2-sys
            )
            ;;
        api-entity)
            CHECK_ARGS=(--package flow-like-api-entity --no-default-features)
            FORBIDDEN=(flow-like flow-like-api flow-like-storage flow-like-catalog axum tower)
            ;;
        api-runtime)
            # The API needs database/catalog runtime code, but registry DTOs
            # must not pull the Wasmtime execution host into the server binary.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-api)
            FORBIDDEN=(
                flow-like-wasm
                wasmtime
                wasmtime-wasi
                wasmtime-wasi-http
                cranelift-codegen
            )
            REQUIRED=(flow-like-wasm-schema)
            ;;
        catalog-portable)
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features portable-metadata)
            FORBIDDEN=(
                flow-like-catalog-automation
                lancedb
                lance
                lance-core
                lance-io
                lance-file
                arrow
                arrow-array
                arrow-schema
                datafusion
                datafusion-common
                smb2
                smb2-sys
                libsmb2-sys
                enigo
                rdev
                xcap
                tauri
            )
            ;;
        catalog-server)
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features server)
            FORBIDDEN=(
                flow-like-catalog-automation
                enigo
                rdev
                xcap
                tauri
                ort
                ort-sys
                fastembed
                face_id
                tract-core
                tract-onnx
                tract-tflite
            )
            ;;
        catalog-server-local-ml)
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features server-local-ml)
            FORBIDDEN=(flow-like-catalog-automation enigo rdev xcap tauri)
            REQUIRED=(ort ort-sys fastembed face_id tract-tflite)
            ;;
        *)
            fail "unknown boundary '$boundary' (run --list to see valid names)"
            ;;
    esac
}

base_cargo_command() {
    BASE_COMMAND=(cargo)
    if [ -n "$CARGO_CONFIG" ]; then
        BASE_COMMAND+=(--config "$CARGO_CONFIG")
    fi
}

append_common_flags() {
    COMMAND+=(--locked)
    if [ "$OFFLINE" -eq 1 ]; then
        COMMAND+=(--offline)
    fi
}

print_command() {
    printf '  CARGO_TARGET_DIR=%q ' "$TARGET_DIR"
    if [ "$DISABLE_RUSTC_WRAPPER" -eq 1 ]; then
        printf 'RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER= '
    fi
    printf '%q ' "${COMMAND[@]}"
    printf '\n'
}

execute_command() {
    if [ "$DISABLE_RUSTC_WRAPPER" -eq 1 ]; then
        CARGO_TARGET_DIR="$TARGET_DIR" RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER= "${COMMAND[@]}"
    else
        CARGO_TARGET_DIR="$TARGET_DIR" "${COMMAND[@]}"
    fi
}

run_compile_check() {
    local boundary="$1"
    base_cargo_command
    COMMAND=("${BASE_COMMAND[@]}" check)
    append_common_flags
    COMMAND+=("${CHECK_ARGS[@]}" --lib)
    log "$boundary: isolated cargo check"
    print_command
    if [ "$DRY_RUN" -eq 0 ]; then
        execute_command
    fi
}

run_dependency_assertions() {
    local boundary="$1"
    local forbidden required violations=0
    base_cargo_command
    COMMAND=("${BASE_COMMAND[@]}" tree)
    append_common_flags
    # Include every target's conditional dependencies. A host-only tree can
    # otherwise miss a forbidden Windows/macOS/mobile edge entirely.
    COMMAND+=("${CHECK_ARGS[@]}" --target all --edges normal,build --prefix none --format "{p}")
    log "$boundary: dependency boundary (${#FORBIDDEN[@]} forbidden packages)"
    print_command

    if [ "$DRY_RUN" -eq 1 ]; then
        printf '  must not contain:'
        printf ' %s' "${FORBIDDEN[@]}"
        printf '\n'
        if [ -n "${REQUIRED[0]}" ]; then
            printf '  must contain:'
            printf ' %s' "${REQUIRED[@]}"
            printf '\n'
        fi
        return 0
    fi

    TREE_FILE="$(mktemp "${TMPDIR:-/tmp}/flow-like-dependency-tree.XXXXXX")"
    execute_command > "$TREE_FILE"
    for forbidden in "${FORBIDDEN[@]}"; do
        if awk -v package="$forbidden" '$1 == package { found = 1 } END { exit found ? 0 : 1 }' "$TREE_FILE"; then
            echo "[feature-boundaries] forbidden dependency in $boundary: $forbidden" >&2
            awk -v package="$forbidden" '$1 == package { print "  " $0 }' "$TREE_FILE" >&2
            violations=1
        fi
    done
    for required in "${REQUIRED[@]}"; do
        [ -n "$required" ] || continue
        if ! awk -v package="$required" '$1 == package { found = 1 } END { exit found ? 0 : 1 }' "$TREE_FILE"; then
            echo "[feature-boundaries] required dependency missing from $boundary: $required" >&2
            violations=1
        fi
    done
    rm -f "$TREE_FILE"
    TREE_FILE=""
    [ "$violations" -eq 0 ] || return 1
    log "$boundary: dependency boundary passed"
}

if [ "$DRY_RUN" -eq 0 ]; then
    command -v cargo >/dev/null 2>&1 || fail "cargo is not installed"
    mkdir -p "$TARGET_DIR"
fi

for boundary in "${BOUNDARIES[@]}"; do
    configure_boundary "$boundary"
    case "$MODE" in
        check)
            if [ "$DEPENDENCY_ONLY" -eq 1 ]; then
                log "$boundary: dependency-only boundary skipped in check mode"
            else
                run_compile_check "$boundary"
            fi
            ;;
        deps)
            run_dependency_assertions "$boundary"
            ;;
        all)
            if [ "$DEPENDENCY_ONLY" -eq 0 ]; then
                run_compile_check "$boundary"
            fi
            run_dependency_assertions "$boundary"
            ;;
    esac
done

log "all requested feature boundaries passed"
