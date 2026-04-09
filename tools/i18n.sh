#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

MODE="${1:-all}"
AUTH_MODE="${AUTH_MODE:-auto}"
LOCALE="${LOCALE:-en}"
EN_PATH="${EN_PATH:-crates/greentic-component/i18n/en.json}"

usage() {
  cat <<'EOF'
Usage: tools/i18n.sh [translate|validate|status|all]

Environment overrides:
  EN_PATH=...                     English source file path (default: i18n/en.json)
  AUTH_MODE=...                   Translator auth mode for translate (default: auto)
  LOCALE=...                      CLI locale used for translator output (default: en)

Examples:
  tools/i18n.sh all
  AUTH_MODE=api-key tools/i18n.sh translate
  EN_PATH=crates/greentic-component/i18n/en.json tools/i18n.sh validate
EOF
}

log() {
  printf '[i18n] %s\n' "$*"
}

fail() {
  printf '[i18n] error: %s\n' "$*" >&2
  exit 1
}

ensure_translator() {
  if command -v greentic-i18n-translator >/dev/null 2>&1; then
    return
  fi

  command -v cargo-binstall >/dev/null 2>&1 \
    || fail "greentic-i18n-translator not found and cargo-binstall is unavailable"

  log "installing greentic-i18n-translator via cargo-binstall"
  cargo binstall -y greentic-i18n-translator \
    || fail "failed to install greentic-i18n-translator via cargo-binstall"

  command -v greentic-i18n-translator >/dev/null 2>&1 \
    || fail "greentic-i18n-translator is still not on PATH after cargo-binstall"
}

run_translate() {
  greentic-i18n-translator \
    --locale "$LOCALE" \
    translate --langs all --en "$EN_PATH" --auth-mode "$AUTH_MODE"
}

run_validate() {
  greentic-i18n-translator \
    --locale "$LOCALE" \
    validate --langs all --en "$EN_PATH"
}

run_status() {
  greentic-i18n-translator \
    --locale "$LOCALE" \
    status --langs all --en "$EN_PATH"
}

if [[ "${MODE}" == "-h" || "${MODE}" == "--help" ]]; then
  usage
  exit 0
fi

ensure_translator

case "$MODE" in
  translate) run_translate ;;
  validate) run_validate ;;
  status) run_status ;;
  all)
    run_translate
    run_validate
    run_status
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    usage
    exit 2
    ;;
esac
