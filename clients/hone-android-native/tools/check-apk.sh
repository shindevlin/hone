#!/bin/bash
# check-apk.sh APK : what phones may get. Fails unless
#  - the APK is not debuggable (Android 16 warns on debuggable apps without 16 KB support),
#  - every .so is stored uncompressed at a 16 KB boundary (zipalign -c -P 16),
#  - every 64-bit .so has ELF LOAD segments aligned to >= 16 KB (2**14).
set -u
APK=$1
SDK=${ANDROID_HOME:-$HOME/Android/Sdk}
BT=$(ls -d "$SDK"/build-tools/* | sort -V | tail -1)
RE=$(ls "$SDK"/ndk/*/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf 2>/dev/null | tail -1)
[ -x "$RE" ] || RE=$(command -v readelf)
fail=0
if "$BT/aapt2" dump badging "$APK" | grep -q application-debuggable; then echo "FAIL debuggable"; fail=1; else echo "ok   not debuggable"; fi
if "$BT/zipalign" -c -P 16 -v 4 "$APK" > /tmp/check-apk-zip.txt 2>&1; then echo "ok   zipalign -P 16"; else grep -v '(OK' /tmp/check-apk-zip.txt | head; echo "FAIL zipalign -P 16"; fail=1; fi
T=$(mktemp -d); unzip -q -o "$APK" 'lib/*' -d "$T" 2>/dev/null
for f in "$T"/lib/arm64-v8a/*.so "$T"/lib/x86_64/*.so; do
  [ -f "$f" ] || continue
  bad=""; for a in $("$RE" -lW "$f" | awk '/LOAD/{print $NF}'); do [ $((a)) -lt 16384 ] && bad=$a; done
  n=${f#$T/}
  if [ -n "$bad" ]; then echo "FAIL $n LOAD align $bad"; fail=1; else echo "ok   $n"; fi
done
rm -rf "$T"
[ $fail = 0 ] && echo "APK OK" || { echo "APK NOT OK"; exit 1; }
