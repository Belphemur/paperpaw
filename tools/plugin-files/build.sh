#!/usr/bin/env bash
# Builds PaperPaw's own plugins that the store offers (plugins.txt) as
# native plugin files for one chip, as the release pipeline does, packs
# each (signed with --key), and checks that it fits the firmware:
#
#   tools/plugin-files/build.sh <chip|device> <out-dir> [--key <file>]
#       [--fingerprint <hex>] [--only <id>]
#
# A file is built for a chip, not for a device (chips.py): <chip> is
# esp32c3 (the X4 and the X3) or esp32s3 (the X4 Pro and the X4 Classic),
# or a device (x4, x3, x4-pro, x4-classic) for its chip. The files are
# <out-dir>/<id>-<version>-<chip>.ppn, <version> the plugin's own (its
# crate's, which its manifest says). With --fingerprint (the firmware's,
# `paperpaw-sign fingerprint <elf>`) each file must have it. With --only,
# one plugin alone (an update of it between releases). Run from
# anywhere; paperpaw-sign is built from this repository.
#
# Needs: the X4's Rust from rust-toolchain.toml, for the ESP32-S3 espup's `esp` toolchain with its environment and the
# release's core libraries in it (tools/xtensa-std). Never set
# RUSTFLAGS: it would replace .cargo/config.toml's flags, which link the
# plugins.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"

usage() { echo "usage: $0 <esp32c3|esp32s3|x4|x3|x4-pro|x4-classic> <out-dir> [--key <file>] [--fingerprint <hex>] [--only <id>]" >&2; exit 2; }
[[ $# -ge 2 ]] || usage
chip="$(python3 "$here/chips.py" chip "$1")" || usage
out=$2
shift 2
key=()
fingerprint=
only=
while [[ $# -gt 0 ]]; do
    case "$1" in
        --key) [[ $# -ge 2 ]] || usage; key=(--key "$2"); shift 2 ;;
        --fingerprint) [[ $# -ge 2 ]] || usage; fingerprint=$2; shift 2 ;;
        --only) [[ $# -ge 2 ]] || usage; only=$2; shift 2 ;;
        *) usage ;;
    esac
done

case "$chip" in
    esp32c3) build=(cargo x4 --locked); target=riscv32imc-unknown-none-elf; chip_id=5 ;;
    esp32s3) build=(cargo +esp x4pro --locked); target=xtensa-esp32s3-none-elf; chip_id=9 ;;
    *) usage ;;
esac

ids="$(sed -e 's/#.*//' "$here/plugins.txt" | awk 'NF { print $1 }')"
if [[ -n "$only" ]]; then
    grep -qx "$only" <<< "$ids" || { echo "$only is not in plugins.txt" >&2; exit 1; }
    ids=$only
fi
# Each plugin's own version, as its crate says it: never PaperPaw's.
version_of() {
    sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$root/plugins/$1/Cargo.toml"
}
for id in $ids; do
    [[ -n "$(version_of "$id")" ]] || { echo "plugins/$id/Cargo.toml: no version of its own" >&2; exit 1; }
done

(cd "$here" && env -u RUSTFLAGS "${build[@]}")
target_dir="${CARGO_TARGET_DIR:-$here/target}"
sign() { (cd "$root" && cargo run -q --release -p paperpaw-sign -- "$@"); }

mkdir -p "$out"
out="$(cd "$out" && pwd)"
for id in $ids; do
    file="$out/$id-$(version_of "$id")-$chip.ppn"
    sign pack "$target_dir/$target/release/$id" "$file" "${key[@]}"
    shown="$(sign plugin "$file")"
    if ! grep -Eq "^chip: .*\($chip_id\)\$" <<< "$shown"; then
        echo "$file: not for chip $chip_id" >&2
        echo "$shown" >&2
        exit 1
    fi
    if [[ -n "$fingerprint" ]] && ! grep -Eq "^fingerprint: +$fingerprint\$" <<< "$shown"; then
        echo "$file does not fit the firmware (fingerprint $fingerprint)" >&2
        echo "$shown" >&2
        exit 1
    fi
    echo "$id: $(wc -c < "$file") bytes"
done
