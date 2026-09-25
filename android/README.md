# Android (arm64) build

Experimental phone port aimed at the Samsung Galaxy S20 family (Snapdragon 865
or Exynos 990, Vulkan 1.1, Android 11 or newer). The engine is the same Rust
code as the Windows game, built as `libskate3rust.so` and hosted by
`GameActivity`.

## Install

1. From the **Android experimental** prerelease, download
   `skate3rust-android-arm64.apk` and `skate3rust-phone-converter.zip`.
   Install the APK (allow "Install unknown apps" when asked).
2. **Game data is not included.** Convert your own Skate 3 Xbox 360 ISO once,
   either on the phone (below) or with the Windows release.

### Convert on the phone (no PC needed)

The converter is the same Python pipeline the Windows setup uses, run inside
[Termux](https://termux.dev) (install it from F-Droid or its GitHub releases;
the Play Store build is outdated). Allow about 30 GB of free space; on a phone
it can take a few hours, so keep Termux open and the phone charging.

1. Put your `.iso` and `skate3rust-phone-converter.zip` in the phone's
   **Download** folder.
2. Open Termux and run `termux-setup-storage`, then allow storage access.
3. Paste (the app's launcher has a *Copy Termux commands* button):

   ```sh
   pkg install -y python python-numpy python-pillow && python -m zipfile -e /sdcard/Download/skate3rust-phone-converter.zip ~ && python ~/skate3rust-converter/tools/phone_setup.py
   ```

   It picks up the only `.iso` in Download (or pass `--iso PATH`), converts
   in Termux's private storage, then moves the result to
   `/sdcard/Skate3Rust/data`. If Android stops it, run the last command again.
4. When it prints `Done`, open **Skate 3 Rust**, tap *Allow access* (needed
   to read `/sdcard/Skate3Rust`) and press **Start**.

The phone converter skips one step of the Windows setup: loading each
converted map in the Windows game as a final check. A map that converts but
fails to load shows its error in the launcher log instead.

### Or copy from a Windows setup

Copy the `data` folder next to `skate3rust.exe` to `/sdcard/Skate3Rust/data`
(or to `Android/data/com.sk8engine.skate3rust/files/data`, which Android
deletes on uninstall), then *Allow access* and **Start**.

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

`.github/workflows/android.yml` does exactly this. CI builds are signed with
a key generated once and kept in the Actions cache, so updates install over
each other. GitHub drops caches unused for 7 days, which changes the key (the
next update then needs an uninstall). To sign with your own key instead, add
the repository secrets
`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`
and `ANDROID_KEY_PASSWORD`.

## What differs from Windows

- `crates/skate-android` compiles `crates/skate-game/src/main.rs` as a
  `cdylib`; `crates/skate-game/src/android.rs` is the Android entry point.
- Controller input comes from `SkateActivity`/`Gamepads.java` instead of
  XInput, in the same raw XInput units.
- ISO conversion runs in Termux (`tools/phone_setup.py`, with the pure-Python
  disc reader `tools/asset_pipeline/xiso.py`) rather than inside the app.
- No updater or crash-report window on the phone. Steam multiplayer is
  unavailable (its relay is a Windows helper).
