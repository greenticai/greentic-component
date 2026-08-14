#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "usage: $0 <target> <version> <tgz|zip>" >&2
    exit 2
fi

target=$1
version=$2
format=$3
case "$format" in
    tgz|zip) ;;
    *)
        echo "unsupported archive format: $format" >&2
        exit 2
        ;;
esac

bins=(greentic-component component-doctor component-hash component-inspect)
suffix=""
if [[ "$target" == *-windows-* ]]; then
    suffix=".exe"
fi

source_dir=${RELEASE_BIN_DIR:-target/$target/release}
stem="greentic-component-v${version}-${target}"
stage_dir="dist/$stem"
mkdir -p "$stage_dir"

for bin in "${bins[@]}"; do
    source_path="$source_dir/$bin$suffix"
    if [[ ! -f "$source_path" ]]; then
        echo "required release binary not found: $source_path" >&2
        exit 1
    fi
    cp "$source_path" "$stage_dir/$bin$suffix"
done
cp README.md "$stage_dir/README.md"

case "$format" in
    tgz)
        archive="dist/$stem.tgz"
        tar -C dist -czf "$archive" "$stem"
        ;;
    zip)
        archive="dist/$stem.zip"
        tar -C dist -a -cf "$archive" "$stem"
        ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
    (cd dist && sha256sum "$(basename "$archive")" > "$(basename "$archive").sha256")
else
    (cd dist && shasum -a 256 "$(basename "$archive")" > "$(basename "$archive").sha256")
fi

echo "$archive"
