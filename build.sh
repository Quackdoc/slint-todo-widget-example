#!/usr/bin/env bash
# Build the todo-widget APK with cargo-ndk and the Android SDK tools (no Gradle).
# Usage: ./build.sh [--arch arm64|x86_64] [--release] [--install [SERIAL]]
set -euo pipefail

ARCH=arm64; RELEASE=0; INSTALL=0; SERIAL=""
while [ $# -gt 0 ]; do
	case "$1" in
		--arch) ARCH="${2:-}"; shift 2 ;;
		--release) RELEASE=1; shift ;;
		--install)
			INSTALL=1; shift
			if [ $# -gt 0 ] && [[ "$1" != --* ]]; then SERIAL="$1"; shift; fi ;;
		-h|--help) sed -n 2,3p "$0"; exit 0 ;;
		*) echo "unknown argument: $1" >&2; exit 2 ;;
	esac
done
case "$ARCH" in
	arm64|arm64-v8a|aarch64) ABI=arm64-v8a ;;
	x86_64) ABI=x86_64 ;;
	*) echo "bad --arch" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export ANDROID_HOME="${ANDROID_HOME:-/opt/android-sdk}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-/opt/android-ndk/android-ndk-r30}"
PLATFORM_JAR="${ANDROID_JAR:-$ANDROID_HOME/platforms/android-34/android.jar}"
export ANDROID_JAR="$PLATFORM_JAR"
BT="$ANDROID_HOME/build-tools/${ANDROID_BUILD_TOOLS_VERSION:-34.0.0}"
export ANDROID_BUILD_TOOLS_VERSION="${ANDROID_BUILD_TOOLS_VERSION:-34.0.0}"
KEYSTORE="${KEYSTORE:-$HOME/.android/debug.keystore}"

VARIANT=debug; CARGO_FLAGS=(); [ "$RELEASE" = 1 ] && { VARIANT=release; CARGO_FLAGS=(--release); }
OUT="$ROOT/build/$VARIANT"; rm -rf "$OUT"; mkdir -p "$OUT/classes" "$OUT/res" "$OUT/lib/$ABI"

echo "==> cargo ndk ($ABI, $VARIANT)"
(cd "$ROOT" && cargo ndk -t "$ABI" -P 26 -o "$OUT/jniLibs" build --lib "${CARGO_FLAGS[@]}")
cp "$OUT/jniLibs/$ABI/libtodo_widget_lib.so" "$OUT/lib/$ABI/"

echo "==> javac + d8"
javac -source 8 -target 8 -encoding UTF-8 -classpath "$PLATFORM_JAR" -d "$OUT/classes" \
	"$ROOT"/android/java/dev/slint/todowidget/*.java 2>&1 | grep -v "^warning\|^Note" || true
"$BT/d8" --lib "$PLATFORM_JAR" --min-api 26 --output "$OUT" $(find "$OUT/classes" -name '*.class')

echo "==> aapt2"
"$BT/aapt2" compile --dir "$ROOT/android/res" -o "$OUT/res.zip"
"$BT/aapt2" link -o "$OUT/base.apk" -I "$PLATFORM_JAR" --manifest "$ROOT/android/AndroidManifest.xml" \
	--min-sdk-version 26 --target-sdk-version 34 "$OUT/res.zip"

echo "==> package"
(cd "$OUT" && cp base.apk unsigned.apk && zip -q -u unsigned.apk classes.dex && \
	zip -q -r -u -0 unsigned.apk "lib/$ABI/libtodo_widget_lib.so")
"$BT/zipalign" -f -p 4 "$OUT/unsigned.apk" "$OUT/aligned.apk"
[ -f "$KEYSTORE" ] || keytool -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android \
	-alias androiddebugkey -dname "CN=Android Debug,O=Android,C=US" -keyalg RSA -keysize 2048 -validity 10000
"$BT/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android --key-pass pass:android \
	--out "$OUT/todo-widget.apk" "$OUT/aligned.apk"
echo "==> $OUT/todo-widget.apk"

if [ "$INSTALL" = 1 ]; then
	ADB=(adb); [ -n "$SERIAL" ] && ADB=(adb -s "$SERIAL")
	"${ADB[@]}" install -r "$OUT/todo-widget.apk"
fi
