#!/usr/bin/env bash
set -euo pipefail

# Reproducible, non-destructive Rust compile-time measurements.
#
# Every invocation gets a fresh target directory. A warm measurement is taken
# from that directory after a priming build, and an incremental measurement
# temporarily advances one source file's mtime before restoring it on exit.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

TARGET_ROOT="${FLOW_LIKE_TIMING_ROOT:-$ROOT_DIR/target/compile-times}"
REPORT_ROOT=""
RUN_ID=""
PHASE="all"
DRY_RUN=0
USE_RUSTC_WRAPPER=0
CARGO_CONFIG=""
JOBS=""
PROFILE=""
CARGO_SUBCOMMAND="check"
INCREMENTAL_MODE="profile"
INCREMENTAL_SOURCE_OVERRIDE=""

SCENARIOS=()
EXTRA_CARGO_ARGS=()
CARGO_COMMAND=()
TOUCHED_SOURCE=""
SOURCE_TIMESTAMP=""
SOURCE_CONTENT_HASH=""

usage() {
    cat <<'EOF'
Usage: tools/compile-times.sh [OPTIONS] [SCENARIO ...] [-- CARGO_ARGS ...]

Measure cold, warm, and incremental Cargo times without cleaning or
reusing the normal workspace target directory. With no scenario, `core` is
measured. Use `--all-scenarios` for the project baseline set.

Scenarios:
  core              Shared flow/editor surface (`flow-like`, no DB runtime)
  core-runtime      Full application/runtime core
  runtime           Execution core without editor services
  editor            FlowScript and copilot services
  desktop           Tauri desktop application (`flow-like-desktop`)
  desktop-std-string  Desktop after a string node edit
  desktop-std-ui     Desktop after an A2UI node edit
  desktop-data-github Desktop after a GitHub integration edit
  backend-executor  Kubernetes executor (`k8s-executor`)
  catalog-portable  Portable catalog metadata bundle
  catalog-server    Remote-only headless catalog execution bundle
  catalog-server-local-ml
                    Same server bundle with local ONNX inference
  backend-local     One representative backend (`local-api`)

Options:
  --command COMMAND          check (default) or build, including codegen/linking
  --incremental MODE         profile (default), 0, or 1; overrides ambient env
  --phase PHASE               cold, warm, incremental, or all (default: all)
  --all-scenarios             Measure every scenario listed above
  --target-root PATH          Build root (default: target/compile-times)
  --report-root PATH          Report root (default: TARGET_ROOT/reports)
  --run-id ID                 Stable output name (default: UTC timestamp + PID)
  --profile PROFILE           Pass `--profile PROFILE` to Cargo
  --jobs N                    Pass `--jobs N` to Cargo
  --cargo-config PATH         Pass `--config PATH` to Cargo
  --incremental-source PATH   Source to touch (only with one scenario)
  --use-rustc-wrapper         Preserve RUSTC_WRAPPER and RUSTC_WORKSPACE_WRAPPER
                              (disabled by default so a remote compiler cache
                              cannot turn a cold run warm)
  --dry-run                   Print the plan without building or touching files
  --list                      List scenarios
  -h, --help                  Show this help

Examples:
  tools/compile-times.sh core
  tools/compile-times.sh --all-scenarios
  tools/compile-times.sh --command build --phase warm desktop
  tools/compile-times.sh --command build desktop-std-string
  tools/compile-times.sh --command build --profile ci --incremental 0 backend-executor
  tools/compile-times.sh --phase incremental catalog-server
  tools/compile-times.sh --cargo-config .cargo/fast-compile.toml core

The script never runs `cargo clean` and never removes a target directory. Each
run prints the build and report paths so old benchmark artifacts can be removed
explicitly when they are no longer useful.
EOF
}

list_scenarios() {
    cat <<'EOF'
core
core-runtime
runtime
editor
desktop
desktop-std-string
desktop-std-ui
desktop-data-github
backend-executor
catalog-portable
catalog-server
catalog-server-local-ml
backend-local
EOF
}

fail() {
    echo "[compile-times] error: $*" >&2
    exit 1
}

log() {
    echo "[compile-times] $*"
}

now_nanoseconds() {
    local value
    # GNU date supports %N; BSD date prints a literal N, so fall back to
    # Time::HiRes on the pinned macOS Bash 3.2 environment.
    value="$(date +%s%N 2>/dev/null || true)"
    case "$value" in
        ""|*[!0-9]*)
            if command -v perl >/dev/null 2>&1; then
                perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1_000_000_000'
            else
                printf '%s000000000\n' "$(date +%s)"
            fi
            ;;
        *) printf '%s\n' "$value" ;;
    esac
}

restore_source_timestamp() {
    if [ -n "$TOUCHED_SOURCE" ] && [ -n "$SOURCE_TIMESTAMP" ] && [ -e "$SOURCE_TIMESTAMP" ]; then
        if [ -e "$TOUCHED_SOURCE" ] \
            && [ "$(git hash-object "$TOUCHED_SOURCE" 2>/dev/null || true)" = "$SOURCE_CONTENT_HASH" ]; then
            touch -r "$SOURCE_TIMESTAMP" "$TOUCHED_SOURCE"
            log "restored timestamp: ${TOUCHED_SOURCE#$ROOT_DIR/}"
        elif [ -e "$TOUCHED_SOURCE" ]; then
            # A developer saved the benchmark source while Cargo was running.
            # Keep those contents and a fresh mtime rather than backdating the
            # edit and letting a later Cargo invocation consider it stale.
            touch "$TOUCHED_SOURCE"
            log "warning: source changed during benchmark; left its new timestamp intact: ${TOUCHED_SOURCE#$ROOT_DIR/}"
        else
            log "warning: touched source was removed during benchmark: ${TOUCHED_SOURCE#$ROOT_DIR/}"
        fi
        rm -f "$SOURCE_TIMESTAMP"
    fi
    TOUCHED_SOURCE=""
    SOURCE_TIMESTAMP=""
    SOURCE_CONTENT_HASH=""
}

trap restore_source_timestamp EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

while [ "$#" -gt 0 ]; do
    case "$1" in
        --command)
            [ "$#" -ge 2 ] || fail "--command requires a value"
            CARGO_SUBCOMMAND="$2"
            shift 2
            ;;
        --incremental)
            [ "$#" -ge 2 ] || fail "--incremental requires a value"
            INCREMENTAL_MODE="$2"
            shift 2
            ;;
        --phase)
            [ "$#" -ge 2 ] || fail "--phase requires a value"
            PHASE="$2"
            shift 2
            ;;
        --all-scenarios)
            SCENARIOS=(core core-runtime runtime editor desktop desktop-std-string desktop-std-ui desktop-data-github backend-executor catalog-portable catalog-server catalog-server-local-ml backend-local)
            shift
            ;;
        --target-root)
            [ "$#" -ge 2 ] || fail "--target-root requires a path"
            TARGET_ROOT="$2"
            shift 2
            ;;
        --report-root)
            [ "$#" -ge 2 ] || fail "--report-root requires a path"
            REPORT_ROOT="$2"
            shift 2
            ;;
        --run-id)
            [ "$#" -ge 2 ] || fail "--run-id requires a value"
            RUN_ID="$2"
            shift 2
            ;;
        --profile)
            [ "$#" -ge 2 ] || fail "--profile requires a value"
            PROFILE="$2"
            shift 2
            ;;
        --jobs)
            [ "$#" -ge 2 ] || fail "--jobs requires a value"
            JOBS="$2"
            shift 2
            ;;
        --cargo-config)
            [ "$#" -ge 2 ] || fail "--cargo-config requires a path"
            CARGO_CONFIG="$2"
            shift 2
            ;;
        --incremental-source)
            [ "$#" -ge 2 ] || fail "--incremental-source requires a path"
            INCREMENTAL_SOURCE_OVERRIDE="$2"
            shift 2
            ;;
        --use-rustc-wrapper)
            USE_RUSTC_WRAPPER=1
            shift
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --list)
            list_scenarios
            exit 0
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            while [ "$#" -gt 0 ]; do
                EXTRA_CARGO_ARGS+=("$1")
                shift
            done
            ;;
        -*)
            fail "unknown option: $1"
            ;;
        *)
            SCENARIOS+=("$1")
            shift
            ;;
    esac
done

case "$CARGO_SUBCOMMAND" in
    check|build) ;;
    *) fail "invalid command '$CARGO_SUBCOMMAND' (expected check or build)" ;;
esac
case "$INCREMENTAL_MODE" in
    profile) unset CARGO_INCREMENTAL ;;
    0|1) export CARGO_INCREMENTAL="$INCREMENTAL_MODE" ;;
    *) fail "invalid incremental mode '$INCREMENTAL_MODE' (expected profile, 0, or 1)" ;;
esac

case "$PHASE" in
    cold|warm|incremental|all) ;;
    *) fail "invalid phase '$PHASE' (expected cold, warm, incremental, or all)" ;;
esac

if [ "${#SCENARIOS[@]}" -eq 0 ]; then
    SCENARIOS=(core)
fi

# Reusing a scenario target within one run would make its second "cold" phase warm and append
# ambiguous rows to the same report. Reject duplicates, including mixing `--all-scenarios` with
# an explicitly repeated scenario.
for ((i = 0; i < ${#SCENARIOS[@]}; i++)); do
    for ((j = i + 1; j < ${#SCENARIOS[@]}; j++)); do
        if [ "${SCENARIOS[$i]}" = "${SCENARIOS[$j]}" ]; then
            fail "scenario '${SCENARIOS[$i]}' was selected more than once"
        fi
    done
done

if [ -n "$INCREMENTAL_SOURCE_OVERRIDE" ] && [ "${#SCENARIOS[@]}" -ne 1 ]; then
    fail "--incremental-source can only be used with one scenario"
fi

if [ -z "$RUN_ID" ]; then
    RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
fi
case "$RUN_ID" in
    *[!A-Za-z0-9._-]*) fail "run id may only contain letters, numbers, '.', '_', and '-'" ;;
esac

if [ -z "$REPORT_ROOT" ]; then
    REPORT_ROOT="$TARGET_ROOT/reports"
fi

RUN_BUILD_ROOT="$TARGET_ROOT/builds/$RUN_ID"
RUN_REPORT_ROOT="$REPORT_ROOT/$RUN_ID"

configure_scenario() {
    local scenario="$1"
    SCENARIO_CARGO_ARGS=()
    case "$scenario" in
        core)
            SCENARIO_CARGO_ARGS=(--package flow-like --no-default-features --features flow-metadata)
            SCENARIO_SOURCE="$ROOT_DIR/packages/core/runtime/src/lib.rs"
            ;;
        core-runtime)
            SCENARIO_CARGO_ARGS=(--package flow-like --no-default-features --features app-runtime)
            SCENARIO_SOURCE="$ROOT_DIR/packages/core/runtime/src/lib.rs"
            ;;
        runtime)
            SCENARIO_CARGO_ARGS=(--package flow-like-runtime --no-default-features --features flow-metadata)
            SCENARIO_SOURCE="$ROOT_DIR/packages/core/runtime/src/lib.rs"
            ;;
        editor)
            SCENARIO_CARGO_ARGS=(--package flow-like-editor)
            SCENARIO_SOURCE="$ROOT_DIR/packages/core/editor/src/lib.rs"
            ;;
        desktop)
            SCENARIO_CARGO_ARGS=(--package flow-like-desktop)
            # The host library is intentionally cfg(mobile) and empty on desktop.
            # Touch the implementation included by the desktop binary so this
            # scenario measures a real application edit.
            SCENARIO_SOURCE="$ROOT_DIR/apps/desktop/src-tauri/src/application.rs"
            ;;
        desktop-std-string)
            SCENARIO_CARGO_ARGS=(--package flow-like-desktop)
            SCENARIO_SOURCE="$ROOT_DIR/packages/catalog/std-text/src/utils/string/trim.rs"
            ;;
        desktop-std-ui)
            SCENARIO_CARGO_ARGS=(--package flow-like-desktop)
            SCENARIO_SOURCE="$ROOT_DIR/packages/catalog/std-ui/src/a2ui/elements/create_element.rs"
            ;;
        desktop-data-github)
            SCENARIO_CARGO_ARGS=(--package flow-like-desktop)
            SCENARIO_SOURCE="$ROOT_DIR/packages/catalog/data/github/src/data/github/get_repo.rs"
            ;;
        backend-executor)
            SCENARIO_CARGO_ARGS=(--package k8s-executor)
            SCENARIO_SOURCE="$ROOT_DIR/apps/backend/kubernetes/executor/src/main.rs"
            ;;
        catalog-portable)
            SCENARIO_CARGO_ARGS=(--package flow-like-catalog --no-default-features --features portable-metadata)
            SCENARIO_SOURCE="$ROOT_DIR/packages/catalog/src/lib.rs"
            ;;
        catalog-server)
            SCENARIO_CARGO_ARGS=(--package flow-like-catalog --no-default-features --features server)
            SCENARIO_SOURCE="$ROOT_DIR/packages/catalog/src/lib.rs"
            ;;
        catalog-server-local-ml)
            SCENARIO_CARGO_ARGS=(--package flow-like-catalog --no-default-features --features server-local-ml)
            SCENARIO_SOURCE="$ROOT_DIR/packages/catalog/src/lib.rs"
            ;;
        backend-local)
            SCENARIO_CARGO_ARGS=(--package local-api)
            SCENARIO_SOURCE="$ROOT_DIR/apps/backend/local/api/src/main.rs"
            ;;
        *)
            fail "unknown scenario '$scenario' (run --list to see valid names)"
            ;;
    esac

    if [ -n "$INCREMENTAL_SOURCE_OVERRIDE" ]; then
        case "$INCREMENTAL_SOURCE_OVERRIDE" in
            /*) SCENARIO_SOURCE="$INCREMENTAL_SOURCE_OVERRIDE" ;;
            *) SCENARIO_SOURCE="$ROOT_DIR/$INCREMENTAL_SOURCE_OVERRIDE" ;;
        esac
    fi
    [ -f "$SCENARIO_SOURCE" ] || fail "incremental source not found: $SCENARIO_SOURCE"
}

compose_cargo_command() {
    CARGO_COMMAND=(cargo)
    if [ -n "$CARGO_CONFIG" ]; then
        CARGO_COMMAND+=(--config "$CARGO_CONFIG")
    fi
    CARGO_COMMAND+=("$CARGO_SUBCOMMAND" --locked --timings)
    CARGO_COMMAND+=("${SCENARIO_CARGO_ARGS[@]}")
    if [ -n "$PROFILE" ]; then
        CARGO_COMMAND+=(--profile "$PROFILE")
    fi
    if [ -n "$JOBS" ]; then
        CARGO_COMMAND+=(--jobs "$JOBS")
    fi
    if [ "${#EXTRA_CARGO_ARGS[@]}" -gt 0 ]; then
        CARGO_COMMAND+=("${EXTRA_CARGO_ARGS[@]}")
    fi
}

print_command() {
    local target_dir="$1"
    if [ "$INCREMENTAL_MODE" = profile ]; then
        printf '  env -u CARGO_INCREMENTAL '
    else
        printf '  CARGO_INCREMENTAL=%q ' "$INCREMENTAL_MODE"
    fi
    printf 'CARGO_TARGET_DIR=%q ' "$target_dir"
    if [ "$USE_RUSTC_WRAPPER" -eq 0 ]; then
        printf 'RUSTC_WRAPPER= RUSTC_WORKSPACE_WRAPPER= '
    fi
    printf '%q ' "${CARGO_COMMAND[@]}"
    printf '\n'
}

archive_timing_report() {
    local target_dir="$1"
    local report_dir="$2"
    local phase="$3"
    local source_report="$target_dir/cargo-timings/cargo-timing.html"
    if [ -f "$source_report" ]; then
        cp "$source_report" "$report_dir/$phase.html"
    else
        log "warning: Cargo did not produce $source_report"
    fi
}

run_measured_phase() {
    local scenario="$1"
    local phase="$2"
    local target_dir="$3"
    local report_dir="$4"
    local started finished elapsed_ns elapsed_ms elapsed status

    log "$scenario / $phase"
    print_command "$target_dir"
    if [ "$DRY_RUN" -eq 1 ]; then
        return 0
    fi

    mkdir -p "$target_dir" "$report_dir"
    started="$(now_nanoseconds)"
    set +e
    if [ "$USE_RUSTC_WRAPPER" -eq 1 ]; then
        CARGO_TARGET_DIR="$target_dir" \
            "${CARGO_COMMAND[@]}" 2>&1 | tee "$report_dir/$phase.log"
    else
        CARGO_TARGET_DIR="$target_dir" RUSTC_WRAPPER= \
            RUSTC_WORKSPACE_WRAPPER= \
            "${CARGO_COMMAND[@]}" 2>&1 | tee "$report_dir/$phase.log"
    fi
    status="${PIPESTATUS[0]}"
    set -e
    finished="$(now_nanoseconds)"
    elapsed_ns=$((finished - started))
    elapsed_ms=$((elapsed_ns / 1000000))
    printf -v elapsed '%d.%03d' "$((elapsed_ms / 1000))" "$((elapsed_ms % 1000))"

    archive_timing_report "$target_dir" "$report_dir" "$phase"
    printf '%s\t%s\t%s\t%s\n' "$scenario" "$phase" "$elapsed" "$status" \
        >> "$RUN_REPORT_ROOT/summary.tsv"
    log "$scenario / $phase: ${elapsed}s (exit $status)"
    return "$status"
}

run_prime() {
    local scenario="$1"
    local target_dir="$2"
    local report_dir="$3"
    log "$scenario / prime (not included in summary)"
    print_command "$target_dir"
    if [ "$DRY_RUN" -eq 1 ]; then
        return 0
    fi

    mkdir -p "$target_dir" "$report_dir"
    if [ "$USE_RUSTC_WRAPPER" -eq 1 ]; then
        CARGO_TARGET_DIR="$target_dir" \
            "${CARGO_COMMAND[@]}" > "$report_dir/prime.log" 2>&1
    else
        CARGO_TARGET_DIR="$target_dir" RUSTC_WRAPPER= \
            RUSTC_WORKSPACE_WRAPPER= \
            "${CARGO_COMMAND[@]}" > "$report_dir/prime.log" 2>&1
    fi
}

touch_incremental_source() {
    local source="$1"
    if [ "$DRY_RUN" -eq 1 ]; then
        log "would temporarily touch and restore: ${source#$ROOT_DIR/}"
        return 0
    fi

    SOURCE_TIMESTAMP="$(mktemp "${TMPDIR:-/tmp}/flow-like-source-time.XXXXXX")"
    touch -r "$source" "$SOURCE_TIMESTAMP"
    SOURCE_CONTENT_HASH="$(git hash-object "$source")"
    TOUCHED_SOURCE="$source"
    touch "$source"
    log "temporarily touched: ${source#$ROOT_DIR/}"
}

if [ "$DRY_RUN" -eq 0 ]; then
    command -v cargo >/dev/null 2>&1 || fail "cargo is not installed"
    command -v rustc >/dev/null 2>&1 || fail "rustc is not installed"
    [ ! -e "$RUN_BUILD_ROOT" ] || fail "build directory already exists: $RUN_BUILD_ROOT"
    [ ! -e "$RUN_REPORT_ROOT" ] || fail "report directory already exists: $RUN_REPORT_ROOT"
    mkdir -p "$RUN_REPORT_ROOT"
    {
        echo "run_id=$RUN_ID"
        echo "workspace=$ROOT_DIR"
        echo "phase=$PHASE"
        echo "command=$CARGO_SUBCOMMAND"
        echo "incremental=$INCREMENTAL_MODE"
        echo "scenarios=${SCENARIOS[*]}"
        echo "jobs=${JOBS:-auto}"
        echo "profile=${PROFILE:-dev}"
        echo "cargo_config=${CARGO_CONFIG:-<workspace-default>}"
        echo "extra_cargo_args=${EXTRA_CARGO_ARGS[*]:-<none>}"
        echo "utc_started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        echo "uname=$(uname -a)"
        echo "cargo=$(cargo --version)"
        echo "rustc=$(rustc --version)"
        echo "host=$(rustc -vV | sed -n 's/^host: //p')"
        echo "git_commit=$(git -C "$ROOT_DIR" rev-parse HEAD 2>/dev/null || echo unknown)"
        echo "git_dirty_entries=$(git -C "$ROOT_DIR" status --porcelain=v1 2>/dev/null | wc -l | tr -d ' ')"
        echo "rustflags=${RUSTFLAGS:-}"
        echo "rustdocflags=${RUSTDOCFLAGS:-}"
        if [ "$USE_RUSTC_WRAPPER" -eq 1 ]; then
            echo "rustc_wrapper=${RUSTC_WRAPPER:-}"
            echo "rustc_workspace_wrapper=${RUSTC_WORKSPACE_WRAPPER:-}"
        else
            echo "rustc_wrapper=<disabled>"
            echo "rustc_workspace_wrapper=<disabled>"
        fi
    } > "$RUN_REPORT_ROOT/environment.txt"
    printf 'scenario\tphase\telapsed_seconds\texit_code\n' > "$RUN_REPORT_ROOT/summary.tsv"
fi

log "build root: $RUN_BUILD_ROOT"
log "report root: $RUN_REPORT_ROOT"

for scenario in "${SCENARIOS[@]}"; do
    configure_scenario "$scenario"
    compose_cargo_command
    scenario_target="$RUN_BUILD_ROOT/$scenario"
    scenario_report="$RUN_REPORT_ROOT/$scenario"

    case "$PHASE" in
        cold)
            run_measured_phase "$scenario" cold "$scenario_target" "$scenario_report"
            ;;
        warm)
            run_prime "$scenario" "$scenario_target" "$scenario_report"
            run_measured_phase "$scenario" warm "$scenario_target" "$scenario_report"
            ;;
        incremental)
            run_prime "$scenario" "$scenario_target" "$scenario_report"
            touch_incremental_source "$SCENARIO_SOURCE"
            run_measured_phase "$scenario" incremental "$scenario_target" "$scenario_report"
            restore_source_timestamp
            ;;
        all)
            run_measured_phase "$scenario" cold "$scenario_target" "$scenario_report"
            run_measured_phase "$scenario" warm "$scenario_target" "$scenario_report"
            touch_incremental_source "$SCENARIO_SOURCE"
            run_measured_phase "$scenario" incremental "$scenario_target" "$scenario_report"
            restore_source_timestamp
            ;;
    esac
done

if [ "$DRY_RUN" -eq 0 ]; then
    log "summary: $RUN_REPORT_ROOT/summary.tsv"
fi
