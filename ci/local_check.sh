#!/usr/bin/env bash
# Usage:
#   LOCAL_CHECK_ONLINE=1 LOCAL_CHECK_STRICT=1 LOCAL_CHECK_VERBOSE=1 ci/local_check.sh
# Defaults: online, non-strict, quiet.

if [ -z "${BASH_VERSION:-}" ]; then
    exec bash "$0" "$@"
fi

set -euo pipefail
if [ -z "${RUSTFLAGS+x}" ]; then
    export RUSTFLAGS=""
fi

ROOT_DIR=$(cd -- "$(dirname "$0")/.." && pwd)
cd "$ROOT_DIR"

TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT_DIR/target}

LOCAL_CHECK_ONLINE=${LOCAL_CHECK_ONLINE:-1}
LOCAL_CHECK_STRICT=${LOCAL_CHECK_STRICT:-0}
LOCAL_CHECK_VERBOSE=${LOCAL_CHECK_VERBOSE:-0}
LOCAL_CHECK_SKIP_FMT=${LOCAL_CHECK_SKIP_FMT:-0}
LOCAL_CHECK_SKIP_CLIPPY=${LOCAL_CHECK_SKIP_CLIPPY:-0}
LOCAL_CHECK_SKIP_BUILD=${LOCAL_CHECK_SKIP_BUILD:-0}
LOCAL_CHECK_SKIP_TEST=${LOCAL_CHECK_SKIP_TEST:-0}
LOCAL_CHECK_SKIP_BUILD_ALL=${LOCAL_CHECK_SKIP_BUILD_ALL:-0}
LOCAL_CHECK_SKIP_TEST_ALL=${LOCAL_CHECK_SKIP_TEST_ALL:-0}
LOCAL_CHECK_SKIP_BINS=${LOCAL_CHECK_SKIP_BINS:-0}
LOCAL_CHECK_SKIP_SCHEMA=${LOCAL_CHECK_SKIP_SCHEMA:-0}
LOCAL_CHECK_SKIP_PACKAGE=${LOCAL_CHECK_SKIP_PACKAGE:-0}
LOCAL_CHECK_SKIP_PUBLISH=${LOCAL_CHECK_SKIP_PUBLISH:-0}
RUST_VERSION=${RUST_VERSION:-1.91.0}
SMOKE_NAME=${SMOKE_NAME:-local-check}
TREE_DIR=${LOCAL_CHECK_TREE_DIR:-$TARGET_DIR/local-check}
SMOKE_TARGET_DIR=$TARGET_DIR/smoke

# Publish mode defaults:
#   - CI environment => publish
#   - local runs      => dry-run
if [ -z "${LOCAL_CHECK_PUBLISH_MODE:-}" ]; then
    if [ -n "${CI:-}" ]; then
        LOCAL_CHECK_PUBLISH_MODE="publish"
    else
        LOCAL_CHECK_PUBLISH_MODE="dry-run"
    fi
fi

# Ignore publish dry-run failures when bumping versions locally.
LOCAL_VERSION_UPGRADE=${LOCAL_VERSION_UPGRADE:-0}
mkdir -p "$TREE_DIR" "$SMOKE_TARGET_DIR"

if [ "$LOCAL_CHECK_VERBOSE" = "1" ]; then
    set -x
fi

need() {
    if command -v "$1" >/dev/null 2>&1; then
        return 0
    fi
    echo "[miss] $1"
    return 1
}

step() {
    echo ""
    echo "▶ $*"
}

FAILED=0
FAILED_STEPS=()
LAST_RUN_FAILED=0

record_failure() {
    FAILED=1
    FAILED_STEPS+=("$1")
}

run_cmd() {
    local desc=$1
    shift
    step "$desc"
    LAST_RUN_FAILED=0
    if ! "$@"; then
        echo "[fail] $desc"
        record_failure "$desc"
        LAST_RUN_FAILED=1
    fi
}

run_bin_cmd() {
    local desc=$1
    local bin_path=$2
    shift 2
    step "$desc"
    if [ ! -x "$bin_path" ]; then
        echo "[fail] $desc ($bin_path missing)"
        record_failure "$desc ($bin_path missing)"
        return 1
    fi
    if ! "$bin_path" "$@"; then
        echo "[fail] $desc"
        record_failure "$desc"
    fi
}

run_bin_cmd_expect_fail() {
    local desc=$1
    local bin_path=$2
    shift 2
    step "$desc"
    if [ ! -x "$bin_path" ]; then
        echo "[fail] $desc ($bin_path missing)"
        record_failure "$desc ($bin_path missing)"
        return 1
    fi
    local out_file
    out_file=$(mktemp)
    if "$bin_path" "$@" >"$out_file" 2>&1; then
        echo "[fail] $desc (unexpected success)"
        record_failure "$desc (unexpected success)"
        if [ -s "$out_file" ]; then
            cat "$out_file"
        fi
        rm -f "$out_file"
        return
    fi
    rm -f "$out_file"
}

run_or_skip() {
    local desc=$1
    shift
    if "$@"; then
        return 0
    fi
    if [ "$LOCAL_CHECK_STRICT" = "1" ]; then
        echo "[fail] $desc"
        record_failure "$desc"
    else
        echo "[skip] $desc"
    fi
}

skip_step() {
    local desc=$1
    local reason=$2
    if [ "$LOCAL_CHECK_STRICT" = "1" ]; then
        echo "[fail] $desc ($reason)"
        record_failure "$desc ($reason)"
    else
        echo "[skip] $desc ($reason)"
    fi
}

skip_flagged() {
    local desc=$1
    local reason=$2
    echo "[skip] $desc ($reason)"
}

hard_need() {
    if ! need "$1"; then
        echo "Error: required tool '$1' is missing" >&2
        exit 1
    fi
}

ensure_rust_target() {
    local target=$1
    if rustup target list --installed | grep -Fxq "$target"; then
        return 0
    fi
    step "Installing Rust target $target"
    rustup target add "$target"
}

hard_need rustc
hard_need cargo
hard_need rustup

if ! rustup toolchain list | awk '{print $1}' | grep -Fxq "$RUST_VERSION"; then
    step "Installing Rust toolchain $RUST_VERSION"
    rustup toolchain install "$RUST_VERSION" --profile minimal
fi
export RUSTUP_TOOLCHAIN="$RUST_VERSION"

ensure_rust_target wasm32-wasip2
ensure_rust_target x86_64-unknown-linux-gnu
ensure_rust_component() {
    local component=$1
    if rustup component list --installed | cut -d' ' -f1 | grep -Fxq "$component"; then
        return 0
    fi
    step "Installing Rust component $component"
    rustup component add "$component"
}
ensure_rust_component rustfmt
ensure_rust_component clippy

step "Tool versions"
echo "expected rust toolchain: $RUST_VERSION"
rustc --version
cargo --version
need jq && jq --version || true
need curl && curl --version || true

CRATES_IO_AVAILABLE=0
CRATES_IO_REASON="LOCAL_CHECK_ONLINE=0"
if [ "$LOCAL_CHECK_ONLINE" = "1" ]; then
    CRATES_IO_REASON="crates.io unreachable"
    if command -v curl >/dev/null 2>&1; then
        if curl -sSf --max-time 5 https://index.crates.io/config.json >/dev/null 2>&1; then
            CRATES_IO_AVAILABLE=1
            CRATES_IO_REASON=""
        else
            echo "[warn] crates.io unreachable; online-only steps will be skipped"
        fi
    else
        CRATES_IO_REASON="curl missing"
        echo "[warn] curl is missing; crates.io-dependent steps will be skipped"
    fi
fi

schema_check() {
    if [ "$LOCAL_CHECK_ONLINE" != "1" ]; then
        echo "[skip] schema drift check (offline)"
        return 0
    fi
    if ! need curl; then
        echo "[skip] schema drift check (curl missing)"
        return 0
    fi
    if ! need jq; then
        echo "[skip] schema drift check (jq missing)"
        return 0
    fi
    step "Schema drift check"
    local remote=/tmp/local-check-schema.json
    local schema_url="https://greenticai-org.github.io/greentic-component/schemas/v1/component.manifest.schema.json"
    if [[ "$schema_url" == *"greenticai.github.io"* ]]; then
        echo "[fail] schema drift check (legacy host configured: $schema_url)"
        record_failure "schema drift check (legacy host configured)"
        return 0
    fi
    local attempts=0
    local success=0
    while [ $attempts -lt 3 ]; do
        if curl -sSf --max-time 5 -o "$remote" "$schema_url"; then
            success=1
            break
        fi
        attempts=$((attempts + 1))
        echo "[warn] schema download attempt $attempts failed: $schema_url"
        sleep 1
    done
    if [ $success -ne 1 ]; then
        echo "[warn] schema URL not reachable: $schema_url"
        if [ "$LOCAL_CHECK_STRICT" = "1" ]; then
            echo "[fail] schema drift check (remote unavailable)"
            record_failure "schema drift check (remote unavailable)"
        else
            echo "[skip] schema drift check (remote unavailable)"
        fi
        return 0
    fi
    local remote_id
    local local_id
    remote_id=$(jq -r '."$id"' "$remote")
    local_id=$(jq -r '."$id"' crates/greentic-component/schemas/v1/component.manifest.schema.json)
    if [ "$remote_id" != "$local_id" ]; then
        echo "Schema ID mismatch remote=$remote_id local=$local_id"
        if [ "$LOCAL_CHECK_STRICT" = "1" ]; then
            record_failure "schema drift check (schema ID mismatch remote=$remote_id local=$local_id)"
        fi
    else
        echo "Schema IDs match: $remote_id"
    fi
}

canonical_wit_dup_check() {
    run_cmd "canonical WIT duplication guard" bash ci/check_no_duplicate_canonical_wit.sh .
}

canonical_bindings_import_guard() {
    run_cmd "canonical bindings import guard" bash ci/check_no_bindings_imports.sh .
}

build_release_bin() {
    local bin=$1
    local features=$2
    local args=(cargo build --locked --release -p greentic-component --bin "$bin")
    if [ -n "$features" ]; then
        args+=(--features "$features")
    fi
    run_cmd "cargo build release -p $bin" "${args[@]}"
}

if [ "$LOCAL_CHECK_ONLINE" = "1" ] && [ "$CRATES_IO_AVAILABLE" = "1" ]; then
    run_cmd "cargo fetch (linux target)" \
        cargo fetch --locked --target x86_64-unknown-linux-gnu
else
    skip_step "cargo fetch (linux target)" "${CRATES_IO_REASON:-crates.io unreachable}"
fi

if [ "$LOCAL_CHECK_SKIP_FMT" = "1" ]; then
    skip_flagged "cargo fmt" "LOCAL_CHECK_SKIP_FMT=1"
else
    run_cmd "cargo fmt" cargo fmt --all --check
fi
if [ "$LOCAL_CHECK_SKIP_CLIPPY" = "1" ]; then
    skip_flagged "cargo clippy" "LOCAL_CHECK_SKIP_CLIPPY=1"
else
    run_cmd "cargo clippy" cargo clippy --locked --workspace --all-targets -- -D warnings
fi
canonical_wit_dup_check
canonical_bindings_import_guard
if [ "$LOCAL_CHECK_SKIP_BUILD" = "1" ]; then
    skip_flagged "cargo build --workspace --locked" "LOCAL_CHECK_SKIP_BUILD=1"
else
    run_cmd "cargo build --workspace --locked" cargo build --workspace --locked
fi
if [ "$LOCAL_CHECK_SKIP_BUILD_ALL" = "1" ]; then
    skip_flagged "cargo build --workspace --all-features --locked" "LOCAL_CHECK_SKIP_BUILD_ALL=1"
else
    run_cmd "cargo build --workspace --all-features --locked" cargo build --workspace --all-features --locked
fi
if [ "$LOCAL_CHECK_SKIP_TEST_ALL" = "1" ]; then
    skip_flagged "cargo test --workspace --all-features --locked" "LOCAL_CHECK_SKIP_TEST_ALL=1"
else
    run_cmd "cargo test --workspace --all-features --locked" cargo test --workspace --all-features --locked -- --nocapture
fi
if [ "$LOCAL_CHECK_SKIP_SCHEMA" = "1" ]; then
    skip_flagged "schema drift check" "LOCAL_CHECK_SKIP_SCHEMA=1"
else
    schema_check
fi

bins_ready=0
if [ "$LOCAL_CHECK_SKIP_BINS" = "1" ]; then
    skip_flagged "release bins" "LOCAL_CHECK_SKIP_BINS=1"
else
    build_release_bin component-inspect "cli,prepare"
    build_release_bin component-doctor "cli,prepare"
    build_release_bin component-hash "cli"
    build_release_bin greentic-component "cli"

    readonly BIN_COMPONENT_INSPECT=$TARGET_DIR/release/component-inspect
    readonly BIN_COMPONENT_DOCTOR=$TARGET_DIR/release/component-doctor
    readonly BIN_COMPONENT_HASH=$TARGET_DIR/release/component-hash
    readonly BIN_GREENTIC_COMPONENT=$TARGET_DIR/release/greentic-component
    bins_ready=1

    run_bin_cmd "component-inspect probe" "$BIN_COMPONENT_INSPECT" --json crates/greentic-component/tests/fixtures/manifests/valid.component.json
    export GREENTIC_SKIP_NODE_EXPORT_CHECK=1
    run_bin_cmd_expect_fail "component-doctor probe" "$BIN_COMPONENT_DOCTOR" \
        crates/greentic-component/tests/contract/fixtures/component_v0_5_0/component.wasm \
        --manifest crates/greentic-component/tests/contract/fixtures/component_v0_5_0/component.manifest.json
    unset GREENTIC_SKIP_NODE_EXPORT_CHECK
fi

run_smoke_mode() {
    local mode=$1
    step "Smoke mode: $mode"
    local smoke_parent
    local cleanup_smoke=0
    local cleanup_dir
    if [ -n "${SMOKE_DIR:-}" ]; then
        smoke_parent="$SMOKE_DIR"
    else
        smoke_parent=$(mktemp -d)
        cleanup_smoke=1
        cleanup_dir="$smoke_parent"
        trap "rm -rf '$cleanup_dir'" EXIT
    fi
    local smoke_path="$smoke_parent/$SMOKE_NAME-$mode"
    rm -rf "$smoke_path"
    export GREENTIC_DEP_MODE="$mode"
    local smoke_manifest="$smoke_path/component.manifest.json"
    local had_cargo_target=0
    local prev_cargo_target
    if [ "${CARGO_TARGET_DIR+x}" = "x" ]; then
        had_cargo_target=1
        prev_cargo_target="$CARGO_TARGET_DIR"
    fi
    export CARGO_TARGET_DIR="$SMOKE_TARGET_DIR"
    run_bin_cmd "Smoke ($mode): scaffold component" "$BIN_GREENTIC_COMPONENT" \
        new --name "$SMOKE_NAME" --org ai.greentic \
        --path "$smoke_path" --non-interactive --no-check --json
    run_bin_cmd_expect_fail "Smoke ($mode): component-doctor" "$BIN_COMPONENT_DOCTOR" "$smoke_path"
    local network_ok=0
    local network_reason="${CRATES_IO_REASON:-network unavailable}"
    if [ "$LOCAL_CHECK_ONLINE" = "1" ] && [ "$CRATES_IO_AVAILABLE" = "1" ]; then
        if curl -sSf --max-time 5 https://index.crates.io/config.json >/dev/null 2>&1; then
            local wasm_ready=1
            run_cmd "Smoke ($mode): cargo generate-lockfile" \
                bash -lc "cd '$smoke_path' && cargo generate-lockfile"
            if [ "$LAST_RUN_FAILED" -eq 1 ]; then
                wasm_ready=0
                network_reason="cargo generate-lockfile failed"
            fi
            local tree_file="$TREE_DIR/tree-$mode.txt"
            if [ $wasm_ready -eq 1 ]; then
                run_cmd "Smoke ($mode): cargo tree" \
                    bash -lc "cd '$smoke_path' && cargo tree -e no-dev --locked | tee '$tree_file' >/dev/null"
                if [ "$LAST_RUN_FAILED" -eq 1 ]; then
                    wasm_ready=0
                    network_reason="cargo tree failed"
                fi
            fi
            if [ $wasm_ready -eq 1 ]; then
                # wasm-component-ld rejects some wasm-ld flags; use WASM_RUSTFLAGS (or RUSTFLAGS fallback)
                # and strip incompatible link-args.
                local wasm_rustflags="${WASM_RUSTFLAGS:-${RUSTFLAGS:-}}"
                if [ -n "$wasm_rustflags" ]; then
                    wasm_rustflags="${wasm_rustflags//-Wl,/}"
                    wasm_rustflags="${wasm_rustflags//-C link-arg=--no-keep-memory/}"
                    wasm_rustflags="${wasm_rustflags//-C link-arg=--threads=1/}"
                fi
                run_cmd "Smoke ($mode): cargo check" \
                    env RUSTFLAGS="$wasm_rustflags" \
                    bash -lc "cd '$smoke_path' && cargo check --target wasm32-wasip2 --locked"
                if [ "$LAST_RUN_FAILED" -eq 1 ]; then
                    wasm_ready=0
                    network_reason="cargo check failed"
                fi
            fi
            if [ $wasm_ready -eq 1 ]; then
                run_cmd "Smoke ($mode): cargo build --release" \
                    env RUSTFLAGS="$wasm_rustflags" \
                    bash -lc "cd '$smoke_path' && cargo build --target wasm32-wasip2 --release --locked"
                if [ "$LAST_RUN_FAILED" -eq 1 ]; then
                    wasm_ready=0
                    network_reason="cargo wasm build failed"
                fi
            fi
            if [ $wasm_ready -eq 1 ]; then
                local crate_snake=${SMOKE_NAME//-/_}
                local built_wasm="$CARGO_TARGET_DIR/wasm32-wasip2/release/${crate_snake}.wasm"
                local wasm_rel
                wasm_rel=$(jq -r '.artifacts.component_wasm' "$smoke_manifest")
                local wasm_file="$smoke_path/$wasm_rel"
                if [ -f "$built_wasm" ]; then
                    mkdir -p "$(dirname "$wasm_file")"
                    cp "$built_wasm" "$wasm_file"
                fi
                if [ ! -f "$wasm_file" ]; then
                    wasm_ready=0
                    network_reason="wasm artifact missing ($wasm_file)"
                fi
            fi
            if [ $wasm_ready -eq 1 ]; then
                network_ok=1
            fi
        else
            network_reason="crates.io unreachable"
        fi
    else
        if [ "$LOCAL_CHECK_ONLINE" != "1" ]; then
            network_reason="LOCAL_CHECK_ONLINE=0"
        fi
    fi
    if [ $network_ok -ne 1 ]; then
        skip_step "Smoke ($mode): cargo generate-lockfile" "$network_reason"
        skip_step "Smoke ($mode): cargo tree" "$network_reason"
        skip_step "Smoke ($mode): cargo check" "$network_reason"
        skip_step "Smoke ($mode): cargo build --release" "$network_reason"
    fi
    if [ $network_ok -eq 1 ]; then
        run_bin_cmd "Smoke ($mode): component-hash" "$BIN_COMPONENT_HASH" "$smoke_manifest"
        run_bin_cmd "Smoke ($mode): component-inspect" "$BIN_COMPONENT_INSPECT" --json "$smoke_manifest"
    else
        skip_step "Smoke ($mode): update manifest hash" "${network_reason:-wasm build unavailable}"
        skip_step "Smoke ($mode): component-inspect" "${network_reason:-wasm build unavailable}"
    fi
    if [ $cleanup_smoke -eq 1 ]; then
        rm -rf "$smoke_parent"
        trap - EXIT
    fi
    if [ $had_cargo_target -eq 1 ]; then
        export CARGO_TARGET_DIR="$prev_cargo_target"
    else
        unset CARGO_TARGET_DIR
    fi
}

run_wizard_smoke() {
    step "Smoke: wizard scaffold"
    local wizard_parent
    local cleanup_wizard=0
    local cleanup_dir
    if [ -n "${SMOKE_DIR:-}" ]; then
        wizard_parent="$SMOKE_DIR"
    else
        wizard_parent=$(mktemp -d)
        cleanup_wizard=1
        cleanup_dir="$wizard_parent"
        trap "rm -rf '$cleanup_dir'" EXIT
    fi
    local wizard_root="$wizard_parent/wizard-smoke"
    rm -rf "$wizard_root"
    run_bin_cmd "wizard new" "$BIN_GREENTIC_COMPONENT" \
        wizard new wizard-smoke --out "$wizard_parent"
    if [ ! -d "$wizard_root" ]; then
        record_failure "wizard new (missing scaffold dir: $wizard_root)"
        if [ $cleanup_wizard -eq 1 ]; then
            rm -rf "$wizard_parent"
            trap - EXIT
        fi
        return
    fi
    run_bin_cmd_expect_fail "wizard doctor (unbuilt)" "$BIN_COMPONENT_DOCTOR" "$wizard_root"

    # Wizard wasm builds must produce a component-model artifact. Installing
    # cargo-component avoids falling back to plain cargo build modules.
    if ! cargo component --version >/dev/null 2>&1; then
        if [ "$LOCAL_CHECK_ONLINE" = "1" ] && [ "$CRATES_IO_AVAILABLE" = "1" ]; then
            run_cmd "wizard: install cargo-component" cargo install cargo-component --locked
        else
            skip_step "wizard: install cargo-component" "${CRATES_IO_REASON:-crates.io unreachable}"
        fi
    fi

    local wizard_manifest="$wizard_root/component.manifest.json"
    if [ -f "$wizard_manifest" ]; then
        if grep -q "component_v0_6::node" "$wizard_root/src/lib.rs"; then
            run_cmd "wizard wasm (make)" env CARGO_NET_OFFLINE=true make -C "$wizard_root" wasm
            local wizard_wasm_rel
            wizard_wasm_rel=$(jq -r '.artifacts.component_wasm // empty' "$wizard_manifest")
            if [ -n "$wizard_wasm_rel" ] && [ "$wizard_wasm_rel" != "null" ]; then
                local wizard_wasm_path="$wizard_root/$wizard_wasm_rel"
                if [ ! -f "$wizard_wasm_path" ]; then
                    local wizard_wasm_base
                    local wizard_wasm_stem
                    local wizard_wasm_dash
                    local wizard_wasm_underscore
                    wizard_wasm_base=$(basename "$wizard_wasm_rel")
                    wizard_wasm_stem=${wizard_wasm_base%.wasm}
                    wizard_wasm_dash=${wizard_wasm_stem//_/-}
                    wizard_wasm_underscore=${wizard_wasm_stem//-/_}
                    local wizard_wasm_candidate=""
                    for cand in \
                        "$wizard_root/target/wasm32-wasip2/release/${wizard_wasm_underscore}.wasm" \
                        "$wizard_root/target/wasm32-wasip2/release/${wizard_wasm_dash}.wasm" \
                        "$wizard_root/target/wasm32-wasip1/release/${wizard_wasm_underscore}.wasm" \
                        "$wizard_root/target/wasm32-wasip1/release/${wizard_wasm_dash}.wasm" \
                        "$wizard_root/dist/${wizard_wasm_underscore}__"*.wasm \
                        "$wizard_root/dist/${wizard_wasm_dash}__"*.wasm; do
                        if [ -f "$cand" ]; then
                            wizard_wasm_candidate="$cand"
                            break
                        fi
                    done
                    if [ -n "$wizard_wasm_candidate" ]; then
                        mkdir -p "$(dirname "$wizard_wasm_path")"
                        cp "$wizard_wasm_candidate" "$wizard_wasm_path"
                    fi
                fi
                if [ -f "$wizard_wasm_path" ]; then
                    run_bin_cmd "wizard update manifest hash" "$BIN_COMPONENT_HASH" "$wizard_manifest"
                    run_bin_cmd "wizard inspect (wasm)" "$BIN_COMPONENT_INSPECT" \
                        "$wizard_wasm_path" --manifest "$wizard_manifest" --json --verify
                else
                    record_failure "wizard inspect (wasm missing artifact: $wizard_wasm_path)"
                fi
            else
                record_failure "wizard inspect (manifest missing artifacts.component_wasm)"
            fi
        else
            run_bin_cmd "wizard build" "$BIN_GREENTIC_COMPONENT" build --manifest "$wizard_manifest" --no-flow
            local wizard_wasm
            wizard_wasm=$(jq -r '.artifacts.component_wasm' "$wizard_manifest")
            local wizard_wasm_path="$wizard_root/$wizard_wasm"
            run_bin_cmd "wizard doctor (built)" "$BIN_COMPONENT_DOCTOR" "$wizard_wasm_path" --manifest "$wizard_manifest"
            local wizard_describe="$wizard_root/dist/$(jq -r '.name' "$wizard_manifest")__0_6_0.describe.cbor"
            if [ -f "$wizard_describe" ]; then
                run_bin_cmd "wizard inspect describe" "$BIN_COMPONENT_INSPECT" --describe "$wizard_describe" --json --verify
            else
                record_failure "wizard inspect describe (missing describe artifact)"
            fi
        fi
    else
        if [ ! -f "$wizard_root/Cargo.toml" ]; then
            record_failure "wizard wasm (missing Cargo.toml: $wizard_root/Cargo.toml)"
            if [ $cleanup_wizard -eq 1 ]; then
                rm -rf "$wizard_parent"
                trap - EXIT
            fi
            return
        fi
        run_cmd "wizard wasm (make)" make -C "$wizard_root" wasm
        local wizard_name
        wizard_name=$(awk 'BEGIN{in_pkg=0} /^\[package\]/{in_pkg=1; next} /^\[/{in_pkg=0} in_pkg && /^name = / {gsub(/"/ , "", $3); print $3; exit}' "$wizard_root/Cargo.toml")
        local wizard_abi
        wizard_abi=$(awk 'BEGIN{in_meta=0} /^\[package.metadata.greentic\]/{in_meta=1; next} /^\[/{in_meta=0} in_meta && /^abi_version = / {gsub(/"/ , "", $3); print $3; exit}' "$wizard_root/Cargo.toml")
        if [ -z "$wizard_abi" ]; then
            wizard_abi="0.6.0"
        fi
        local wizard_abi_underscore=${wizard_abi//./_}
        local wizard_wasm_path="$wizard_root/dist/${wizard_name}__${wizard_abi_underscore}.wasm"
        run_bin_cmd "wizard doctor (built)" "$BIN_COMPONENT_DOCTOR" "$wizard_wasm_path"
        run_bin_cmd "wizard inspect (wasm)" "$BIN_COMPONENT_INSPECT" "$wizard_wasm_path" --json --verify
    fi
    if [ $cleanup_wizard -eq 1 ]; then
        rm -rf "$wizard_parent"
        trap - EXIT
    fi
}

if [ "${LOCAL_CHECK_SKIP_SMOKE:-0}" = "1" ]; then
    echo "[skip] smoke scaffold (LOCAL_CHECK_SKIP_SMOKE=1)"
elif [ "$bins_ready" -ne 1 ]; then
    echo "[skip] smoke scaffold (release bins skipped)"
else
    smoke_modes="${LOCAL_CHECK_SMOKE_MODES:-cratesio}"
    for mode in $smoke_modes; do
        run_smoke_mode "$mode"
    done
    run_wizard_smoke
fi

publish_crates=(
    greentic-component-manifest
    greentic-component-store
    greentic-component-runtime
    greentic-component
)
if [ -n "${PUBLISH_CRATES:-}" ]; then
    read -r -a publish_crates <<< "$PUBLISH_CRATES"
fi
if [ "$LOCAL_CHECK_SKIP_PACKAGE" = "1" ]; then
    skip_flagged "cargo package (locked)" "LOCAL_CHECK_SKIP_PACKAGE=1"
else
    if [ "$LOCAL_CHECK_ONLINE" = "1" ] && [ "$CRATES_IO_AVAILABLE" = "1" ]; then
        for crate in "${publish_crates[@]}"; do
            run_cmd "cargo package (locked) -p $crate" \
                cargo package --allow-dirty -p "$crate" --locked
        done
    else
        skip_step "cargo package (locked)" "${CRATES_IO_REASON:-network unavailable}"
    fi
fi
if [ "$LOCAL_CHECK_SKIP_PUBLISH" = "1" ]; then
    skip_flagged "cargo publish" "LOCAL_CHECK_SKIP_PUBLISH=1"
else
    case "$LOCAL_CHECK_PUBLISH_MODE" in
        dry-run)
            if [ "$LOCAL_CHECK_ONLINE" = "1" ] && [ "$CRATES_IO_AVAILABLE" = "1" ]; then
                for crate in "${publish_crates[@]}"; do
                    step "cargo publish --dry-run (locked) -p $crate"
                    echo ""
                    echo "▶ cargo publish --dry-run (locked) -p $crate"
                    if ! cargo publish --allow-dirty -p "$crate" --dry-run --locked; then
                        if [ "$LOCAL_VERSION_UPGRADE" = "1" ]; then
                            echo "[warn] cargo publish --dry-run (locked) -p $crate failed, ignoring due to LOCAL_VERSION_UPGRADE=1"
                        else
                            echo "[fail] cargo publish --dry-run (locked) -p $crate"
                            record_failure "cargo publish --dry-run (locked) -p $crate"
                        fi
                    fi
                done
            else
                skip_step "cargo publish --dry-run (locked)" "${CRATES_IO_REASON:-network unavailable}"
            fi
            ;;
        publish)
            if [ "$LOCAL_CHECK_ONLINE" = "1" ] && [ "$CRATES_IO_AVAILABLE" = "1" ]; then
                for crate in "${publish_crates[@]}"; do
                    step "cargo publish (locked) -p $crate"
                    echo ""
                    echo "▶ cargo publish (locked) -p $crate"
                    publish_output=$(mktemp)
                    publish_status=0
                    if ! cargo publish --allow-dirty -p "$crate" --locked 2> "$publish_output"; then
                        publish_status=$?
                    fi
                    if [ $publish_status -ne 0 ]; then
                        if grep -qi "already exists on crates.io index" "$publish_output"; then
                            echo "[warn] cargo publish skipped (version already on crates.io)"
                        else
                            cat "$publish_output"
                            echo "[fail] cargo publish (locked) -p $crate"
                            record_failure "cargo publish (locked) -p $crate"
                        fi
                    fi
                    rm -f "$publish_output"
                done
            else
                skip_step "cargo publish (locked)" "${CRATES_IO_REASON:-network unavailable}"
            fi
            ;;
    *)
        skip_step "cargo publish / dry-run" "LOCAL_CHECK_PUBLISH_MODE=$LOCAL_CHECK_PUBLISH_MODE not recognized"
        ;;
esac
fi

if [ "$FAILED" -ne 0 ]; then
    echo ""
    if [ "${#FAILED_STEPS[@]}" -ne 0 ]; then
        echo "Failed checks:"
        for step in "${FAILED_STEPS[@]}"; do
            echo "- $step"
        done
        echo ""
    fi
    echo "❌ LOCAL CHECK FAILED"
    exit 1
fi

echo ""
echo "✅ LOCAL CHECK PASSED"

