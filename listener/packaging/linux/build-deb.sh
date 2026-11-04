#!/usr/bin/env bash
# Build hone-listener_<ver>_<arch>.deb.  usage: build-deb.sh <amd64|arm64> <binary> <libtensorflowlite_c.so> <assets-dir> <out-dir>
# assets-dir holds perch_v2_8bit.tflite, class_names.json, names_en.json, NOTICE.txt
set -euo pipefail
ARCH=$1 BIN=$2 TFL=$3 ASSETS=$4 OUT=$5
HERE=$(cd "$(dirname "$0")" && pwd)
VER=$(grep -m1 '^version' "$HERE/../../Cargo.toml" | cut -d'"' -f2)
umask 022
ROOT=$(mktemp -d)
chmod 0755 "$ROOT"
trap 'rm -rf "$ROOT"' EXIT
install -d "$ROOT/DEBIAN" "$ROOT/usr/lib/hone-listener" "$ROOT/usr/bin" "$ROOT/lib/systemd/system" \
  "$ROOT/usr/share/applications" "$ROOT/usr/share/doc/hone-listener" "$ROOT/etc/default" "$ROOT/usr/share/icons/hicolor/256x256/apps"
install -m 0755 "$BIN" "$ROOT/usr/lib/hone-listener/hone-listener"
install -m 0644 "$TFL" "$ROOT/usr/lib/hone-listener/libtensorflowlite_c.so"
for f in perch_v2_8bit.tflite class_names.json names_en.json NOTICE.txt; do install -m 0644 "$ASSETS/$f" "$ROOT/usr/lib/hone-listener/$f"; done
ln -s ../lib/hone-listener/hone-listener "$ROOT/usr/bin/hone-listener"
install -m 0644 "$HERE"/hone-listener.service "$HERE"/hone-listener-update.path "$HERE"/hone-listener-update.service "$ROOT/lib/systemd/system/"
install -m 0644 "$HERE/hone-listener.desktop" "$ROOT/usr/share/applications/"
install -m 0644 "$HERE/default" "$ROOT/etc/default/hone-listener"
[ -f "$HERE/../icon-256.png" ] && install -m 0644 "$HERE/../icon-256.png" "$ROOT/usr/share/icons/hicolor/256x256/apps/hone-listener.png"
cat > "$ROOT/usr/share/doc/hone-listener/copyright" <<C
HONE listener: Usufruct License (UFL), see https://github.com/shindevlin/hone
Perch v2 model: Copyright Google LLC, Apache License 2.0; 8-bit LiteRT packaging by Arm, Apache License 2.0.
TensorFlow Lite runtime: Copyright The TensorFlow Authors, Apache License 2.0.
C
sed -e "s/@VERSION@/$VER/" -e "s/@ARCH@/$ARCH/" "$HERE/debian/control" > "$ROOT/DEBIAN/control"
echo "/etc/default/hone-listener" > "$ROOT/DEBIAN/conffiles"
install -m 0755 "$HERE"/debian/postinst "$HERE"/debian/prerm "$HERE"/debian/postrm "$ROOT/DEBIAN/"
mkdir -p "$OUT"
dpkg-deb --root-owner-group -Zxz --build "$ROOT" "$OUT/hone-listener_${VER}_${ARCH}.deb"
echo "$OUT/hone-listener_${VER}_${ARCH}.deb"
