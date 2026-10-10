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

# Build a real zip of dist/<stem> at dist/<stem>.zip.
#
# `tar -a -cf foo.zip` does NOT do this under GNU tar: -a picks a COMPRESSION
# filter from the suffix (gz/bz2/xz/zst) and knows no zip CONTAINER, so an
# unmatched suffix silently yields an uncompressed tar wearing a .zip name.
# bsdtar would have produced a zip — but the Windows release job runs this
# script under Git Bash, whose tar is MSYS2 GNU tar, not the bsdtar in
# System32. That is how every *-pc-windows-msvc.zip this repo has ever
# published came to be a POSIX tar archive.
#
# Downstream, cargo-binstall fails to extract, falls back to a source build,
# and that build collides with an already-installed binary — so
# `gtc install --tenant` has been broken on Windows for every user, not just
# CI. See greentic-e2e's nightly, red 2026-08-15 through 2026-08-31.
make_zip() {
    local dir=$1 stem=$2 out=$3

    if command -v zip >/dev/null 2>&1; then
        ( cd "$dir" && zip -qr "$out" "$stem" )
    elif command -v 7z >/dev/null 2>&1; then
        ( cd "$dir" && 7z a -tzip -bso0 -bsp0 "$out" "$stem" >/dev/null )
    elif command -v python3 >/dev/null 2>&1; then
        ( cd "$dir" && python3 -c "
import shutil, sys
shutil.make_archive(sys.argv[1].removesuffix('.zip'), 'zip', '.', sys.argv[2])
" "$out" "$stem" )
    elif tar --version 2>/dev/null | head -1 | grep -qi bsdtar; then
        # bsdtar genuinely writes a zip container for -a --format=zip.
        ( cd "$dir" && tar --format=zip -cf "$out" "$stem" )
    else
        echo "no tool available to build a zip (tried zip, 7z, python3, bsdtar)" >&2
        return 1
    fi
}

# Refuse to publish an archive whose bytes do not match its extension.
#
# This is the check whose absence let the bug above ship: the packaging step
# succeeded, the upload succeeded, and nothing looked at what was actually in
# the file until a downstream consumer tried to open it three weeks later.
assert_archive_format() {
    local path=$1 want=$2
    local magic
    magic=$(od -An -tx1 -N4 "$path" | tr -d ' \n')

    case "$want" in
        zip)
            # PK\x03\x04 (normal) or PK\x05\x06 (empty archive).
            case "$magic" in
                504b0304*|504b0506*) ;;
                *)
                    echo "$path is not a zip (magic: ${magic:-empty})" >&2
                    echo "  $(command -v file >/dev/null 2>&1 && file -b "$path")" >&2
                    return 1
                    ;;
            esac
            ;;
        tgz)
            case "$magic" in
                1f8b*) ;;
                *)
                    echo "$path is not gzip (magic: ${magic:-empty})" >&2
                    echo "  $(command -v file >/dev/null 2>&1 && file -b "$path")" >&2
                    return 1
                    ;;
            esac
            ;;
    esac
}

case "$format" in
    tgz)
        archive="dist/$stem.tgz"
        tar -C dist -czf "$archive" "$stem"
        ;;
    zip)
        archive="dist/$stem.zip"
        make_zip dist "$stem" "$stem.zip"
        ;;
esac

assert_archive_format "$archive" "$format"

if command -v sha256sum >/dev/null 2>&1; then
    (cd dist && sha256sum "$(basename "$archive")" > "$(basename "$archive").sha256")
else
    (cd dist && shasum -a 256 "$(basename "$archive")" > "$(basename "$archive").sha256")
fi

echo "$archive"
