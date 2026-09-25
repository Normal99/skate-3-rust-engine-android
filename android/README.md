# Android (arm64) build

Experimental phone port aimed at the Samsung Galaxy S20 family (Snapdragon 865
or Exynos 990, Vulkan 1.1, Android 11 or newer). The engine is the same Rust
code as the Windows game, built as `libskate3rust.so` and hosted by
`GameActivity`.

## Install

1. Download `skate3rust-android-arm64.apk` from the **Android experimental**
   prerelease (or the `skate3rust-android-arm64` artifact of the
   *Android APK* workflow run) and install it. Allow "Install unknown apps"
   for your browser or file manager when asked.
2. **Game data is not included and cannot be converted on the phone.** Run the
   Windows release once so its setup converts your Skate 3 ISO, then copy the
   whole `data` folder that sits next to `skate3rust.exe` to the phone:
   - **Recommended:** create `Skate3Rust` in the phone's internal storage, copy
     `data` into it (`/sdcard/Skate3Rust/data`), then open the app and tap
     *Allow access*. This folder survives reinstalling or updating the APK.
   - Or, over USB from the PC, copy `data` into
     `Internal storage/Android/data/com.sk8engine.skate3rust/files/`
     (open the app once first so the folder exists). Android deletes this
     folder if the app is uninstalled.
3. Open **Skate 3 Rust**, choose options and press **Start**.

## Controls

A Bluetooth or USB controller (Xbox, DualSense/DualShock, most Android pads)
is mapped to the Xbox layout the engine expects. Without a controller an
on-screen pad is drawn: sticks, A/B/X/Y, D-pad, LB/RB, LT/RT, L3/R3,
View and Menu. The phone's Back gesture opens the game menu.

## Hitting 60 FPS

The launcher defaults are chosen for a steady 60 FPS attempt on an S20+:

- **Render resolution 720p.** The game surface is fixed to 720 lines and the
  display hardware upscales it. Native 1080p/1440p costs 2-4x the GPU work.
- **60 Hz display mode** with vsync, so frames are evenly paced.
- **MSAA off and GPU occlusion culling off** by default on Android (both can be
  turned back on in the in-game menu, together with the internal render
  scale; 67-75 % is the next step down if a map is still too heavy).

The desktop renderer was not designed for phones, so 60 FPS is a target, not
a guarantee; large maps and dense areas may run lower. Samsung Game Booster
("Game performance: Performance mode") also helps sustained clocks.

## Troubleshooting

The launcher shows the end of `logs/latest.log` from the last run. The same
lines go to logcat under the tag `skate3rust`
(`adb logcat -s skate3rust`).

## Building locally

Requires the Android SDK (platform 35), NDK r26 or newer, JDK 17, Gradle 8.14
and `rustup target add aarch64-linux-android`. Then, with `NDK` pointing to
the NDK:

```sh
BIN=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin   # darwin-x86_64 / windows-x86_64 elsewhere
export CC_aarch64_linux_android=$BIN/aarch64-linux-android30-clang
export CXX_aarch64_linux_android=$BIN/aarch64-linux-android30-clang++
export AR_aarch64_linux_android=$BIN/llvm-ar
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$BIN/aarch64-linux-android30-clang
cargo build --locked --target aarch64-linux-android --profile android -p skate-android
mkdir -p android/app/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/android/libskate3rust.so android/app/src/main/jniLibs/arm64-v8a/
cp $NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/aarch64-linux-android/libc++_shared.so android/app/src/main/jniLibs/arm64-v8a/
cd android && gradle assembleRelease
```

`.github/workflows/android.yml` does exactly this. To sign CI builds with your
own key (so updates install over each other), add the repository secrets
`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`
and `ANDROID_KEY_PASSWORD`.

## What differs from Windows

- `crates/skate-android` compiles `crates/skate-game/src/main.rs` as a
  `cdylib`; `crates/skate-game/src/android.rs` is the Android entry point.
- Controller input comes from `SkateActivity`/`Gamepads.java` instead of
  XInput, in the same raw XInput units.
- No ISO setup, updater or crash-report window on the phone. Steam multiplayer
  is unavailable (its relay is a Windows helper).
