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
FORBIDDEN=("")
REQUIRED=("")
FEATURE_PACKAGE=""
FORBIDDEN_FEATURES=("")
REQUIRED_FEATURES=("")
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
core-flow-metadata
core-flow
core-model
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
catalog-portable-execute
catalog-server
catalog-server-local-ml
catalog-execute
catalog-server-execute
catalog-executor
catalog-local-runtime
catalog-modifier-local-ml
catalog-modifier-data
executor-default
executor-server
api-remote-catalog
api-aws-catalog
api-local-ml-catalog
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
        core-flow-metadata
        core-flow
        core-model
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
        catalog-portable-execute
        catalog-server
        catalog-server-local-ml
        catalog-execute
        catalog-server-execute
        catalog-executor
        catalog-local-runtime
        catalog-modifier-local-ml
        catalog-modifier-data
        executor-default
        executor-server
        api-remote-catalog
        api-aws-catalog
        api-local-ml-catalog
    )
fi

configure_boundary() {
    local boundary="$1"
    CHECK_ARGS=()
    FORBIDDEN=("")
    REQUIRED=("")
    FEATURE_PACKAGE=""
    FORBIDDEN_FEATURES=("")
    REQUIRED_FEATURES=("")
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
        core-flow-metadata)
            CHECK_ARGS=(--package flow-like --no-default-features --features flow-metadata)
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
            FEATURE_PACKAGE="flow-like"
            REQUIRED_FEATURES=(flow-metadata)
            FORBIDDEN_FEATURES=(flow flow-runtime)
            ;;
        core-flow)
            # Dependency-only compatibility guard. The historical `flow`
            # feature must retain the database-backed execution surface.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like --no-default-features --features flow)
            REQUIRED=(lancedb lance datafusion)
            FEATURE_PACKAGE="flow-like-storage"
            REQUIRED_FEATURES=(database-runtime)
            ;;
        core-model)
            CHECK_ARGS=(--package flow-like --no-default-features --features model)
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
            )
            FEATURE_PACKAGE="flow-like"
            REQUIRED_FEATURES=(model bit hub app flow-metadata)
            FORBIDDEN_FEATURES=(flow flow-runtime)
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
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(portable-metadata)
            FORBIDDEN_FEATURES=(runtime-catalog)
            ;;
        catalog-portable-execute)
            # This execution modifier does not select a metadata bundle or a
            # local inference runtime. Capability modifiers may self-register
            # their own packages, and the aggregate must compile in isolation.
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features portable-execute)
            FORBIDDEN=(
                flow-like-catalog-automation
                flow-like-catalog-onnx
                ort
                ort-sys
                fastembed
                face_id
                tract-core
                tract-onnx
                tract-tflite
            )
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(portable-execute runtime-catalog)
            FORBIDDEN_FEATURES=(package-onnx local-ml)
            ;;
        catalog-server)
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features server)
            FORBIDDEN=(
                flow-like-catalog-automation
                flow-like-catalog-onnx
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
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(
                remote-metadata
                package-std
                package-data
                package-web
                package-media
                package-ml
                package-llm
                package-processing
                package-geo
                bigquery
                runtime-catalog
            )
            FORBIDDEN_FEATURES=(package-onnx local-ml portable-metadata)
            ;;
        catalog-server-local-ml)
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features server-local-ml)
            FORBIDDEN=(flow-like-catalog-automation enigo rdev xcap tauri)
            REQUIRED=(flow-like-catalog-onnx ort ort-sys fastembed face_id tract-tflite)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(remote-metadata package-onnx local-ml runtime-catalog)
            FORBIDDEN_FEATURES=(package-automation)
            ;;
        catalog-execute)
            # Compatibility alias. Dependency checks plus aggregate feature
            # assertions avoid recompiling the same local-ML graph solely to
            # verify that get_catalog() package gates are enabled.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features execute)
            REQUIRED=(
                flow-like-catalog-std
                flow-like-catalog-data
                flow-like-catalog-web
                flow-like-catalog-media
                flow-like-catalog-ml
                flow-like-catalog-onnx
                flow-like-catalog-llm
                flow-like-catalog-processing
                flow-like-catalog-geo
                flow-like-catalog-automation
                ort
                fastembed
            )
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(
                package-std
                package-data
                package-web
                package-media
                package-ml
                package-onnx
                package-llm
                package-processing
                package-geo
                package-automation
                local-ml
                runtime-catalog
            )
            ;;
        catalog-server-execute)
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features server-execute)
            FORBIDDEN=(flow-like-catalog-automation enigo rdev xcap tauri)
            REQUIRED=(flow-like-catalog-onnx ort fastembed)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(
                package-std
                package-data
                package-web
                package-media
                package-ml
                package-onnx
                package-llm
                package-processing
                package-geo
                local-ml
                runtime-catalog
            )
            FORBIDDEN_FEATURES=(package-automation)
            ;;
        catalog-executor)
            # The base full executor includes automation and database support,
            # but ONNX is reserved for the explicit executor-local-ml variant.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features executor)
            FORBIDDEN=(flow-like-catalog-onnx ort ort-sys fastembed face_id tract-tflite)
            REQUIRED=(flow-like-catalog-automation)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(executor remote-metadata package-automation portable-execute remote runtime-catalog)
            FORBIDDEN_FEATURES=(package-onnx local-ml portable-metadata)
            ;;
        catalog-local-runtime)
            # The local runtime name describes an in-process executor. Local Bit
            # model support remains an explicit local-runtime-ml capability.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features local-runtime)
            FORBIDDEN=(flow-like-catalog-onnx ort ort-sys fastembed face_id tract-tflite)
            REQUIRED=(flow-like-catalog-automation)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(local-runtime remote-metadata package-automation portable-execute remote runtime-catalog)
            FORBIDDEN_FEATURES=(package-onnx local-ml portable-metadata)
            ;;
        catalog-modifier-local-ml)
            # Representative self-registration check for a modifier spanning
            # several optional catalog packages.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features local-ml)
            REQUIRED=(flow-like-catalog-onnx ort fastembed)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(
                package-std
                package-data
                package-web
                package-media
                package-ml
                package-onnx
                package-llm
                package-processing
                local-ml
            )
            FORBIDDEN_FEATURES=(package-automation)
            ;;
        catalog-modifier-data)
            # Database, lake and federation modifiers all follow this same
            # package-data self-registration rule.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-catalog --no-default-features --features postgres,bigquery)
            REQUIRED=(flow-like-catalog-data gcp-bigquery-client)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(package-data postgres bigquery)
            FORBIDDEN_FEATURES=(portable-metadata all-metadata)
            ;;
        executor-default)
            # Direct users historically receive the portable metadata catalog.
            # Workspace products disable this default and select a named bundle.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-executor)
            REQUIRED=(flow-like-catalog-onnx)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(portable-metadata package-onnx)
            ;;
        executor-server)
            # Guard against an unconditional executor dependency reintroducing
            # metadata for nodes that the remote server cannot execute.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package flow-like-executor --no-default-features --features server)
            FORBIDDEN=(flow-like-catalog-onnx ort ort-sys fastembed face_id tract-tflite)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(remote-metadata package-std package-data package-web runtime-catalog)
            FORBIDDEN_FEATURES=(portable-metadata package-onnx local-ml)
            ;;
        api-remote-catalog)
            # Azure and GCP APIs publish the same catalog as their remote-only
            # executors, so clients never receive ONNX or automation nodes.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package azure-api --package gcp-api)
            FORBIDDEN=(flow-like-catalog-onnx flow-like-catalog-automation ort fastembed face_id)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(remote-metadata package-std package-data package-web runtime-catalog)
            FORBIDDEN_FEATURES=(package-onnx package-automation local-ml portable-execute)
            ;;
        api-aws-catalog)
            # AWS APIs advertise ONNX metadata to match their local-ML
            # executors, but do not link node execution or ORT dependencies.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package aws-api)
            FORBIDDEN=(flow-like-catalog-automation ort ort-sys fastembed face_id tract-tflite)
            REQUIRED=(flow-like-catalog-onnx)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(server-local-metadata remote-metadata package-onnx runtime-catalog)
            FORBIDDEN_FEATURES=(package-automation local-ml portable-execute)
            ;;
        api-local-ml-catalog)
            # Local, Kubernetes, and Docker APIs mirror executors that support
            # local ONNX and automation without linking either implementation.
            DEPENDENCY_ONLY=1
            CHECK_ARGS=(--package local-api --package k8s-api --package docker-compose-api)
            FORBIDDEN=(ort ort-sys fastembed face_id tract-tflite enigo rdev xcap)
            REQUIRED=(flow-like-catalog-onnx flow-like-catalog-automation)
            FEATURE_PACKAGE="flow-like-catalog"
            REQUIRED_FEATURES=(executor-local-metadata server-local-metadata remote-metadata package-onnx package-automation runtime-catalog)
            FORBIDDEN_FEATURES=(local-ml portable-execute)
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
    local forbidden required violations=0 forbidden_count="${#FORBIDDEN[@]}"
    [ -n "${FORBIDDEN[0]}" ] || forbidden_count=0
    base_cargo_command
    COMMAND=("${BASE_COMMAND[@]}" tree)
    append_common_flags
    # Include every target's conditional dependencies. A host-only tree can
    # otherwise miss a forbidden Windows/macOS/mobile edge entirely.
    COMMAND+=("${CHECK_ARGS[@]}" --target all --edges normal,build --prefix none --format "{p}")
    log "$boundary: dependency boundary ($forbidden_count forbidden packages)"
    print_command

    if [ "$DRY_RUN" -eq 1 ]; then
        if [ "$forbidden_count" -gt 0 ]; then
            printf '  must not contain:'
            printf ' %s' "${FORBIDDEN[@]}"
            printf '\n'
        fi
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
        [ -n "$forbidden" ] || continue
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

run_feature_assertions() {
    local boundary="$1"
    local feature root_line enabled_features violations=0
    [ -n "$FEATURE_PACKAGE" ] || return 0

    base_cargo_command
    COMMAND=("${BASE_COMMAND[@]}" tree)
    append_common_flags
    COMMAND+=(
        "${CHECK_ARGS[@]}"
        --edges features
        --invert "$FEATURE_PACKAGE"
        --prefix none
        --format "{p} {f}"
    )
    log "$boundary: aggregate feature assertions"
    print_command

    if [ "$DRY_RUN" -eq 1 ]; then
        printf '  root package: %s\n' "$FEATURE_PACKAGE"
        if [ -n "${FORBIDDEN_FEATURES[0]}" ]; then
            printf '  must not enable:'
            printf ' %s' "${FORBIDDEN_FEATURES[@]}"
            printf '\n'
        fi
        if [ -n "${REQUIRED_FEATURES[0]}" ]; then
            printf '  must enable:'
            printf ' %s' "${REQUIRED_FEATURES[@]}"
            printf '\n'
        fi
        return 0
    fi

    TREE_FILE="$(mktemp "${TMPDIR:-/tmp}/flow-like-feature-tree.XXXXXX")"
    execute_command > "$TREE_FILE"
    root_line="$(awk -v package="$FEATURE_PACKAGE" '$1 == package && $2 ~ /^v[0-9]/ { print; exit }' "$TREE_FILE")"
    [ -n "$root_line" ] || fail "could not find $FEATURE_PACKAGE in feature tree for $boundary"
    enabled_features="${root_line##* }"

    for feature in "${FORBIDDEN_FEATURES[@]}"; do
        [ -n "$feature" ] || continue
        case ",$enabled_features," in
            *",$feature,"*)
                echo "[feature-boundaries] forbidden aggregate feature in $boundary: $feature" >&2
                violations=1
                ;;
        esac
    done
    for feature in "${REQUIRED_FEATURES[@]}"; do
        [ -n "$feature" ] || continue
        case ",$enabled_features," in
            *",$feature,"*) ;;
            *)
                echo "[feature-boundaries] required aggregate feature missing from $boundary: $feature" >&2
                violations=1
                ;;
        esac
    done
    rm -f "$TREE_FILE"
    TREE_FILE=""
    [ "$violations" -eq 0 ] || return 1
    log "$boundary: aggregate feature assertions passed"
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
            run_feature_assertions "$boundary"
            ;;
        all)
            if [ "$DEPENDENCY_ONLY" -eq 0 ]; then
                run_compile_check "$boundary"
            fi
            run_dependency_assertions "$boundary"
            run_feature_assertions "$boundary"
            ;;
    esac
done

log "all requested feature boundaries passed"
