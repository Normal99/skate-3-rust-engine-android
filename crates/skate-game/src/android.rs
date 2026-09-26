//! Android host (GameActivity). Owns the process entry, the phone-storage
//! locations that replace the portable executable directory, logcat output,
//! crash capture, and the controller state published by `SkateActivity.java`.
use skate_core::input::xbox::XboxState;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::{BufRead, BufReader, Write},
    os::fd::{FromRawFd, IntoRawFd},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicI32, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Shared folder that survives reinstalling the APK. Needs "All files access".
const SHARED_ROOT: &str = "/storage/emulated/0/Skate3Rust";

static APP_FILES: OnceLock<PathBuf> = OnceLock::new();
/// Append-mode descriptor of logs/latest.log for writes that must not wait
/// for the stdout/stderr pipe (panics, fatal signals).
static LOG_FD: AtomicI32 = AtomicI32::new(-1);
static PIPE_WRITE_FD: AtomicI32 = AtomicI32::new(-1);
static LOG_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
/// Load address of libskate3rust.so, for symbolising crash addresses.
static LIBRARY_BASE: AtomicUsize = AtomicUsize::new(0);

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
    install_signal_handlers();
    // SAFETY: runs before Bevy or any game thread starts; the Java side never
    // reads the native environment.
    unsafe {
        std::env::set_var("SKATE3_MODS", root.join("mods"));
        std::env::set_var("TMPDIR", files.join("tmp"));
        std::env::set_var("BEVY_ASSET_ROOT", &root);
        // The log is read as plain text in the launcher.
        std::env::set_var("NO_COLOR", "1");
        std::env::set_var("RUST_BACKTRACE", "1");
    }
    let _ = std::env::set_current_dir(&root);
    // Uncompressed RGBA map textures do not fit phone memory at authored size
    // (University: ~2,000 textures). SkateActivity passes the launcher choice.
    let reduction = std::env::var("SKATE_TEXTURE_REDUCTION").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    skate_data::skate_map::set_texture_reduction(reduction);
    let draw_distance = std::env::var("SKATE_DRAW_DISTANCE").ok().and_then(|v| v.parse().ok()).unwrap_or(150);
    crate::skate_world::DRAW_DISTANCE.store(draw_distance, Ordering::Relaxed);
    eprintln!(
        "REPORT_META stage=android_host root={} library_base=0x{:x} texture_reduction={reduction} draw_distance={draw_distance} rss_mb={} build={}",
        root.display(),
        LIBRARY_BASE.load(Ordering::Relaxed),
        resident_mb(),
        env!("SKATE_BUILD_ID")
    );
    let code = match std::panic::catch_unwind(crate::main) {
        Ok(exit) => {
            eprintln!("REPORT_META state=exit success={}", exit.is_success());
            if exit.is_success() { 0 } else { 1 }
        }
        Err(_) => {
            eprintln!("REPORT_META state=exit panic=true");
            101
        }
    };
    finish_log();
    // Bevy cannot be initialised twice in one process, and Android reuses
    // processes between launches. End it so the next launch starts clean.
    std::process::exit(code);
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
    let Ok(file) = std::fs::File::create(log) else { return };
    if let Ok(direct) = std::fs::OpenOptions::new().append(true).open(log) {
        LOG_FD.store(direct.into_raw_fd(), Ordering::Relaxed);
    }
    let mut fds = [0 as c_int; 2];
    // SAFETY: plain POSIX calls on descriptors owned by this function.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        libc::dup2(fds[1], 1);
        libc::dup2(fds[1], 2);
    }
    PIPE_WRITE_FD.store(fds[1], Ordering::Relaxed);
    let reader = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let mut file = file;
    let thread = std::thread::Builder::new().name("android-log".into()).spawn(move || {
        let tag = CString::new("skate3rust").unwrap();
        for line in BufReader::new(reader).split(b'\n').map_while(Result::ok) {
            let _ = file.write_all(&line).and_then(|_| file.write_all(b"\n"));
            let text = CString::new(line.into_iter().filter(|&b| b != 0).collect::<Vec<_>>()).unwrap_or_default();
            // SAFETY: both strings are NUL-terminated and outlive the call.
            unsafe { __android_log_write(4, tag.as_ptr(), text.as_ptr()) };
        }
    });
    if let Ok(thread) = thread {
        *LOG_THREAD.lock().unwrap() = Some(thread);
    }
}

/// Drain everything already written to stdout/stderr into the log before
/// the process ends (process::exit would otherwise drop the last lines).
fn finish_log() {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: replaces our own descriptors; the pipe reader then sees EOF.
    unsafe {
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
        if null >= 0 {
            libc::dup2(null, 1);
            libc::dup2(null, 2);
            libc::close(null);
        }
        let pipe = PIPE_WRITE_FD.swap(-1, Ordering::Relaxed);
        if pipe >= 0 {
            libc::close(pipe);
        }
    }
    if let Some(thread) = LOG_THREAD.lock().ok().and_then(|mut t| t.take()) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Write straight to the log file, bypassing the pipe and its reader thread.
fn write_direct(bytes: &[u8]) {
    let fd = LOG_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        // SAFETY: write(2) is async-signal-safe; short writes only lose text.
        unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
    }
}

/// Runs after crash_report installs its hook (which prints the backtrace
/// through stderr); the message itself is written synchronously first.
pub(crate) fn chain_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        write_direct(format!("REPORT_PANIC_DIRECT thread={} {info}\n", thread.name().unwrap_or("unnamed")).as_bytes());
        previous(info);
        let _ = std::io::stderr().flush();
    }));
}

const FATAL_SIGNALS: [c_int; 6] =
    [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE, libc::SIGABRT, libc::SIGTRAP];
static PREVIOUS_ACTIONS: Mutex<Vec<(c_int, libc::sigaction)>> = Mutex::new(Vec::new());

/// Record native crashes (GPU driver faults, aborts) in the log, then hand
/// the signal to the previous handler so Android still reports the crash.
fn install_signal_handlers() {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr only reads loader state for an address in this library.
    if unsafe { libc::dladdr(android_main as *const c_void, &mut info) } != 0 {
        LIBRARY_BASE.store(info.dli_fbase as usize, Ordering::Relaxed);
    }
    let mut previous = PREVIOUS_ACTIONS.lock().unwrap();
    for signal in FATAL_SIGNALS {
        // SAFETY: a zeroed sigaction with an empty mask is a valid template.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_fatal_signal as *const () as usize;
            action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
            libc::sigemptyset(&mut action.sa_mask);
            let mut old: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signal, &action, &mut old) == 0 {
                previous.push((signal, old));
            }
        }
    }
}

/// Fixed-size, allocation-free text buffer for the signal handler.
struct SignalText {
    bytes: [u8; 512],
    len: usize,
}
impl SignalText {
    fn new() -> Self {
        Self { bytes: [0; 512], len: 0 }
    }
    fn push(&mut self, text: &[u8]) {
        let take = text.len().min(self.bytes.len() - self.len);
        self.bytes[self.len..self.len + take].copy_from_slice(&text[..take]);
        self.len += take;
    }
    fn hex(&mut self, value: usize) {
        self.push(b"0x");
        let mut digits = [0u8; 16];
        let mut count = 0;
        let mut rest = value;
        loop {
            digits[count] = b"0123456789abcdef"[rest & 15];
            count += 1;
            rest >>= 4;
            if rest == 0 {
                break;
            }
        }
        for index in (0..count).rev() {
            self.push(&digits[index..index + 1]);
        }
    }
    fn number(&mut self, value: i64) {
        if value < 0 {
            self.push(b"-");
        }
        let mut digits = [0u8; 20];
        let mut count = 0;
        let mut rest = value.unsigned_abs();
        loop {
            digits[count] = b'0' + (rest % 10) as u8;
            count += 1;
            rest /= 10;
            if rest == 0 {
                break;
            }
        }
        for index in (0..count).rev() {
            self.push(&digits[index..index + 1]);
        }
    }
    fn flush(&mut self) {
        self.push(b"\n");
        write_direct(&self.bytes[..self.len]);
        self.len = 0;
    }
}

unsafe extern "C" {
    fn _Unwind_Backtrace(
        trace: extern "C" fn(*mut c_void, *mut c_void) -> c_int,
        argument: *mut c_void,
    ) -> c_int;
    fn _Unwind_GetIP(context: *mut c_void) -> usize;
}

struct Frames {
    pcs: [usize; 48],
    count: usize,
}

extern "C" fn collect_frame(context: *mut c_void, argument: *mut c_void) -> c_int {
    // SAFETY: argument is the Frames passed below; context comes from libunwind.
    let frames = unsafe { &mut *argument.cast::<Frames>() };
    if frames.count == frames.pcs.len() {
        return 5; // _URC_END_OF_STACK
    }
    frames.pcs[frames.count] = unsafe { _Unwind_GetIP(context) };
    frames.count += 1;
    0 // _URC_NO_REASON
}

extern "C" fn on_fatal_signal(signal: c_int, info: *mut libc::siginfo_t, _context: *mut c_void) {
    let mut text = SignalText::new();
    text.push(b"REPORT_NATIVE signal=");
    text.number(signal.into());
    // SAFETY: the kernel passes a valid siginfo_t for SA_SIGINFO handlers.
    let (code, address) = unsafe { ((*info).si_code, (*info).si_addr() as usize) };
    text.push(b" code=");
    text.number(code.into());
    text.push(b" fault_address=");
    text.hex(address);
    text.push(b" tid=");
    text.number(unsafe { libc::gettid() }.into());
    text.push(b" thread=");
    // SAFETY: open/read/close are async-signal-safe.
    unsafe {
        let fd = libc::open(c"/proc/thread-self/comm".as_ptr(), libc::O_RDONLY);
        if fd >= 0 {
            let mut name = [0u8; 32];
            let read = libc::read(fd, name.as_mut_ptr().cast(), name.len());
            libc::close(fd);
            if read > 0 {
                let name = &name[..read as usize];
                text.push(name.strip_suffix(b"\n").unwrap_or(name));
            }
        }
    }
    text.push(b" library_base=");
    text.hex(LIBRARY_BASE.load(Ordering::Relaxed));
    text.flush();
    // Best effort: unwinding and dladdr are not formally async-signal-safe,
    // but the process is already crashing and this names the faulting code.
    let mut frames = Frames { pcs: [0; 48], count: 0 };
    unsafe { _Unwind_Backtrace(collect_frame, (&mut frames as *mut Frames).cast()) };
    for &pc in &frames.pcs[..frames.count] {
        text.push(b"REPORT_NATIVE   pc=");
        text.hex(pc);
        let mut dl: libc::Dl_info = unsafe { std::mem::zeroed() };
        if unsafe { libc::dladdr(pc as *const c_void, &mut dl) } != 0 {
            text.push(b" ");
            if !dl.dli_fname.is_null() {
                let name = unsafe { CStr::from_ptr(dl.dli_fname) }.to_bytes();
                text.push(name.rsplit(|&b| b == b'/').next().unwrap_or(name));
            }
            text.push(b"+");
            text.hex(pc.wrapping_sub(dl.dli_fbase as usize));
            if !dl.dli_sname.is_null() {
                text.push(b" ");
                text.push(unsafe { CStr::from_ptr(dl.dli_sname) }.to_bytes());
            }
        }
        text.flush();
    }
    // Restore the previous handler (debuggerd/Rust). A hardware fault then
    // re-faults on return; a raised signal is re-sent explicitly.
    if let Ok(previous) = PREVIOUS_ACTIONS.try_lock() {
        if let Some((_, old)) = previous.iter().find(|(s, _)| *s == signal) {
            unsafe { libc::sigaction(signal, old, std::ptr::null_mut()) };
        }
    }
    if code <= 0 {
        unsafe { libc::raise(signal) };
    }
}

/// Resident memory of this process in MB (from /proc/self/statm).
fn resident_mb() -> u64 {
    std::fs::read_to_string("/proc/self/statm").ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map_or(0, |pages| pages * 4096 / (1024 * 1024))
}

/// Start of the main-world frame, for splitting game logic from rendering.
#[derive(bevy::prelude::Resource)]
pub(crate) struct FrameStart(Instant);

pub(crate) fn install_diagnostics(app: &mut bevy::prelude::App) {
    // Per-pass CPU and (with timestamp queries) GPU times, summarised below.
    app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin)
        .insert_resource(FrameStart(Instant::now()))
        .add_systems(bevy::prelude::First, |mut start: bevy::prelude::ResMut<FrameStart>| start.0 = Instant::now())
        .add_systems(bevy::prelude::Last, frame_report);
}

#[derive(Default)]
pub(crate) struct FrameWindow {
    frames: u64,
    window: u64,
    main_seconds: f64,
    started: Option<Instant>,
}

/// Logs frame rate, memory, main-world (game logic) time and the slowest
/// render passes every 5 seconds, visible in the launcher's log view.
fn frame_report(
    mut state: bevy::prelude::Local<FrameWindow>,
    start: bevy::prelude::Res<FrameStart>,
    diagnostics: bevy::prelude::Res<bevy::diagnostic::DiagnosticsStore>,
    windows: bevy::prelude::Query<&bevy::window::Window>,
    menu: Option<bevy::prelude::Res<crate::graphics_menu::Menu>>,
) {
    let now = Instant::now();
    let started = *state.started.get_or_insert(now);
    state.frames += 1;
    state.window += 1;
    state.main_seconds += now.duration_since(start.0).as_secs_f64();
    if state.frames <= 3 {
        eprintln!("REPORT_META frame={} rss_mb={}", state.frames, resident_mb());
    }
    if state.frames == 3 || state.window == 1 {
        // Confirms the launcher's fixed surface size and the menu render scale.
        let sizes: Vec<String> = windows.iter().map(|w| format!("{}x{}", w.physical_width(), w.physical_height())).collect();
        eprintln!("REPORT_META window={} graphics={}", sizes.join(","),
            menu.as_ref().map_or_else(String::new, |m| m.diagnostic_settings()));
    }
    let elapsed = now.duration_since(started);
    if elapsed < Duration::from_secs(5) {
        return;
    }
    let mut passes: Vec<(f64, String)> = diagnostics
        .iter()
        .filter_map(|d| {
            let path = d.path().as_str();
            let kind = if path.ends_with("/elapsed_gpu") { "gpu" } else if path.ends_with("/elapsed_cpu") { "cpu" } else { return None };
            let name = path.strip_prefix("render/")?.rsplit_once('/')?.0;
            Some((d.average()?, format!("{name}:{kind}")))
        })
        .collect();
    passes.sort_by(|a, b| b.0.total_cmp(&a.0));
    let top: Vec<String> = passes.iter().take(6).map(|(ms, name)| format!("{name}={ms:.1}ms")).collect();
    // Main-world CPU per system/schedule, averaged per frame, slowest first.
    let mut systems = crate::profiling::take_system_times();
    systems.sort_by(|a, b| b.1.cmp(&a.1));
    let frames = state.window.max(1) as f64;
    let systems: Vec<String> = systems.iter().take(14).map(|(label, total, runs)| {
        let short = label.rsplit("::").next().unwrap_or(label);
        format!("{short}={:.1}ms/{:.1}x", total.as_secs_f64() * 1000. / frames, *runs as f64 / frames)
    }).collect();
    eprintln!(
        "REPORT_META fps={:.1} frames={} rss_mb={} main_ms={:.1} passes=[{}] systems=[{}]",
        state.window as f64 / elapsed.as_secs_f64(),
        state.frames,
        resident_mb(),
        state.main_seconds * 1000. / state.window as f64,
        top.join(" "),
        systems.join(" ")
    );
    state.window = 0;
    state.main_seconds = 0.;
    state.started = Some(now);
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
