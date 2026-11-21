# slint-todo-widget-example

The Slint todo example with an Android home screen widget, written almost entirely in Rust.

The widget UI (`ui/widget.slint`) is rendered offscreen with Slint's software renderer, pushed into the
widget as a bitmap through `RemoteViews`, and each row toggles its todo when tapped. The app and the
widget share their todos through `files/todos.json`.

This uses unmodified Slint (pinned in `Cargo.toml`); it needs no changes to Slint itself.

## How it works

- `src/widget.rs`: renders `TodoWidget` to RGBA on a short-lived thread (Slint's platform is per thread, so it
  does not clash with the app's Android platform), builds the `Bitmap` and `RemoteViews` over JNI, and handles
  the toggle broadcast.
- `android/java/.../TodoWidgetProvider.java`: a 30-line shim. Android needs a Java class for the
  `<receiver>`; it only forwards to the native functions.
- `android/AndroidManifest.xml`, `android/res/`: the manifest, the widget info and the layout. The layout lays
  transparent tap targets over the rows drawn by Slint, so keep its geometry in sync with `ui/widget.slint`.
- `build.sh`: builds the APK with `cargo ndk`, `javac`, `d8`, `aapt2`, `zipalign` and `apksigner`, no Gradle.
  `cargo-apk` can't be used because it can't declare a receiver or package a dex.

## Building

Requirements: Rust with the `aarch64-linux-android` (and/or `x86_64-linux-android`) target, `cargo-ndk`,
a JDK, the Android SDK (platform 34, build-tools 34) and **NDK r28 or newer**. Slint's Skia prebuilts need a
libc++ symbol older NDKs lack.

```sh
export ANDROID_HOME=/path/to/android-sdk
export ANDROID_NDK_HOME=/path/to/android-ndk-r30
./build.sh --release --install     # [--arch arm64|x86_64] [--install SERIAL]
```

Open the app once, then add the "Todo Widget" from the launcher's widget picker. The widget refreshes when the
app goes to the background, and tapping a row toggles that todo.

## Limitations

- The widget is a bitmap: no scrolling, and only the first five todos are shown.
- The widget and the app sync through a file, when the app pauses or resumes.
