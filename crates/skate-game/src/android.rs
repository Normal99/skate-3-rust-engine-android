//! Android host (GameActivity). Owns the process entry, the phone-storage
//! locations that replace the portable executable directory, logcat output,
//! and the controller state published by `SkateActivity.java`.
use skate_core::input::xbox::XboxState;
use std::{
    ffi::{CString, c_char, c_int, c_void},
    io::{BufRead, BufReader, Write},
    os::fd::FromRawFd,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// Shared folder that survives reinstalling the APK. Needs "All files access".
const SHARED_ROOT: &str = "/storage/emulated/0/Skate3Rust";

static APP_FILES: OnceLock<PathBuf> = OnceLock::new();

#[unsafe(no_mangle)]
fn android_main(app: bevy::android::android_activity::AndroidApp) {
    let files = app
        .external_data_path()
        .or_else(|| app.internal_data_path())
        .unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let _ = APP_FILES.set(files.clone());
    let _ = bevy::android::ANDROID_APP.set(app);
    let root = data_root();
    let _ = std::fs::create_dir_all(root.join("logs"));
    let _ = std::fs::create_dir_all(root.join("mods"));
    let _ = std::fs::create_dir_all(files.join("tmp"));
    redirect_output(&root.join("logs/latest.log"));
    // SAFETY: runs before Bevy or any game thread starts; the Java side never
    // reads the native environment.
    unsafe {
        std::env::set_var("SKATE3_MODS", root.join("mods"));
        std::env::set_var("TMPDIR", files.join("tmp"));
        std::env::set_var("BEVY_ASSET_ROOT", &root);
    }
    let _ = std::env::set_current_dir(&root);
    eprintln!("REPORT_META stage=android_host root={}", root.display());
    let exit = crate::main();
    eprintln!("REPORT_META state=exit success={}", exit.is_success());
    let _ = std::io::stderr().flush();
    // Bevy cannot be initialised twice in one process, and Android reuses
    // processes between launches. End it so the next launch starts clean.
    std::process::exit(if exit.is_success() { 0 } else { 1 });
}

/// Folder holding `data/` (or `assets/`), settings, logs and mods. The shared
/// folder wins when it holds game data the app is allowed to read (without
/// "All files access", scoped storage hides non-media files there).
pub(crate) fn data_root() -> PathBuf {
    let shared = Path::new(SHARED_ROOT);
    if ["data/installation.json", "assets/private/game.json", "data/assets/private/game.json"]
        .iter()
        .any(|marker| std::fs::File::open(shared.join(marker)).is_ok())
    {
        return shared.to_owned();
    }
    APP_FILES.get().cloned().unwrap_or_else(|| PathBuf::from("."))
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(priority: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Game diagnostics use stdout/stderr, which Android discards. Mirror both to
/// logcat (tag "skate3rust") and to logs/latest.log for players without adb.
fn redirect_output(log: &Path) {
    let mut fds = [0 as c_int; 2];
    // SAFETY: plain POSIX calls on descriptors owned by this function.
    unsafe {
        if libc_pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        libc_dup2(fds[1], 1);
        libc_dup2(fds[1], 2);
    }
    let reader = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let mut file = std::fs::File::create(log).ok();
    let _ = std::thread::Builder::new().name("android-log".into()).spawn(move || {
        let tag = CString::new("skate3rust").unwrap();
        for line in BufReader::new(reader).split(b'\n').map_while(Result::ok) {
            if let Some(file) = file.as_mut() {
                let _ = file.write_all(&line).and_then(|_| file.write_all(b"\n"));
            }
            let text = CString::new(line.into_iter().filter(|&b| b != 0).collect::<Vec<_>>()).unwrap_or_default();
            // SAFETY: both strings are NUL-terminated and outlive the call.
            unsafe { __android_log_write(4, tag.as_ptr(), text.as_ptr()) };
        }
    });
}

unsafe extern "C" {
    #[link_name = "pipe"]
    fn libc_pipe(fds: *mut c_int) -> c_int;
    #[link_name = "dup2"]
    fn libc_dup2(old: c_int, new: c_int) -> c_int;
}

#[derive(Clone, Copy)]
struct Pad {
    number: u32,
    buttons: u16,
    triggers: [u8; 2],
    left: [i16; 2],
    right: [i16; 2],
}

/// Latest XInput-shaped state per slot, written by the Java activity from
/// Android gamepad events and the on-screen touch controls.
static PADS: Mutex<[Option<Pad>; 4]> = Mutex::new([None; 4]);

pub(crate) fn poll_pad(slot: usize) -> Option<(u32, XboxState)> {
    let pad = PADS.lock().ok()?.get(slot).copied().flatten()?;
    Some((
        pad.number,
        XboxState { buttons: pad.buttons, triggers: pad.triggers, left: pad.left, right: pad.right },
    ))
}

/// `static native void nativePad(int, int, int, int, int, int, int, int, int)`
/// on `com.sk8engine.skate3rust.SkateActivity`. Values are already in XInput
/// units: button bits, 0..255 triggers and -32768..32767 sticks (Y up).
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_sk8engine_skate3rust_SkateActivity_nativePad(
    _env: *mut c_void,
    _class: *mut c_void,
    slot: c_int,
    connected: c_int,
    buttons: c_int,
    left_trigger: c_int,
    right_trigger: c_int,
    left_x: c_int,
    left_y: c_int,
    right_x: c_int,
    right_y: c_int,
) {
    let Ok(mut pads) = PADS.lock() else { return };
    let Some(entry) = usize::try_from(slot).ok().and_then(|slot| pads.get_mut(slot)) else { return };
    if connected == 0 {
        *entry = None;
        return;
    }
    let axis = |value: c_int| value.clamp(-32768, 32767) as i16;
    let trigger = |value: c_int| value.clamp(0, 255) as u8;
    *entry = Some(Pad {
        number: entry.map_or(1, |pad| pad.number.wrapping_add(1)),
        buttons: buttons as u16,
        triggers: [trigger(left_trigger), trigger(right_trigger)],
        left: [axis(left_x), axis(left_y)],
        right: [axis(right_x), axis(right_y)],
    });
}
