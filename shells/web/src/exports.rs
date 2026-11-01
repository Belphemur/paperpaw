//! What the page's script calls: plain functions of the WebAssembly
//! module, numbers and pointers into its memory, no generated glue
//! (`www/paperpaw-web.js` wraps them). Strings and files are copied in
//! through memory the script asks for ([`pp_alloc`]); texts come back
//! through the result buffer ([`pp_result_ptr`], [`pp_result_len`]).
//!
//! The module imports three functions from `paperpaw`: `random(ptr,
//! len)`, which fills memory with random bytes and returns 1, `log(ptr,
//! len)`, which shows a text (a panic's message), and `now()`, the
//! page's `performance.now()` in milliseconds (the panel's animations,
//! the time plugins take).
//!
//! A panic stops a call in its middle (a trap): the module's memory stays
//! as it was, its state held. Every call after that says so ([`STOPPED`],
//! a size of 0, no picture, -2 from [`pp_advance`]) instead of panicking
//! again; the page makes a fresh instance.

use std::cell::RefCell;

use paperpaw_platform::PointerPhase;

use crate::emulator::Emulator;
use crate::network::Fixture;

#[link(wasm_import_module = "paperpaw")]
unsafe extern "C" {
    #[link_name = "random"]
    fn host_random(ptr: *mut u8, len: usize) -> u32;
    #[link_name = "log"]
    fn host_log(ptr: *const u8, len: usize);
    #[link_name = "now"]
    fn host_now() -> f64;
}

/// The page's clock in microseconds.
fn clock() -> u64 {
    // SAFETY: the import takes nothing and returns a number.
    (unsafe { host_now() } * 1000.0) as u64
}

#[derive(Debug, Default)]
struct State {
    emulator: Option<Emulator>,
    /// The last text asked for (an error, JSON).
    result: Vec<u8>,
    /// The last settled picture asked for ([`pp_shown`]).
    shown: Vec<u8>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Random bytes from the browser (`crypto.getRandomValues`).
fn random(buf: &mut [u8]) -> bool {
    // SAFETY: the page's `random` writes at most `len` bytes at `ptr`,
    // which is `buf`.
    unsafe { host_random(buf.as_mut_ptr(), buf.len()) == 1 }
}

fn log(text: &str) {
    // SAFETY: the page's `log` only reads `len` bytes at `ptr`.
    unsafe { host_log(text.as_ptr(), text.len()) }
}

/// Bytes the script put into memory.
///
/// # Safety
///
/// `ptr` and `len` are memory the script wrote, from [`pp_alloc`] (or
/// `len` 0).
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        return &[];
    }
    // SAFETY: as the caller promises.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// A text the script put into memory; empty if it is not UTF-8.
///
/// # Safety
///
/// As [`bytes`].
unsafe fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    // SAFETY: as the caller promises.
    std::str::from_utf8(unsafe { bytes(ptr, len) }).unwrap_or_default()
}

/// What a call returns once a panic stopped the module (see above).
pub const STOPPED: u32 = 2;

/// Runs `f` on the state; `None` where a panic left it held. A refusal
/// of the heap budget before is forgotten: [`pp_refused`] tells those of
/// this call.
fn state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    paperpaw_emulator::budget::clear_last_refused();
    STATE.with(|state| state.try_borrow_mut().ok().map(|mut held| f(&mut held)))
}

/// Runs `f` on the emulator, if there is one; 0 if it went well, else 1
/// with the error in the result buffer ([`STOPPED`] after a panic).
fn with(f: impl FnOnce(&mut Emulator) -> Result<(), String>) -> u32 {
    state(|state| {
        let State {
            emulator, result, ..
        } = state;
        let outcome = match emulator {
            Some(emulator) => f(emulator),
            None => Err("no device yet (pp_new)".to_string()),
        };
        match outcome {
            Ok(()) => 0,
            Err(error) => {
                *result = error.into_bytes();
                1
            }
        }
    })
    .unwrap_or(STOPPED)
}

/// Puts `text` into the result buffer.
fn answer(text: String) {
    let _ = state(|state| state.result = text.into_bytes());
}

/// The emulator's `f`, `none` where there is none or the module stopped.
fn read<R>(none: R, f: impl FnOnce(&mut Emulator) -> R) -> R {
    state(|state| state.emulator.as_mut().map(f))
        .flatten()
        .unwrap_or(none)
}

/// Memory for the script to write `len` bytes into; given back with
/// [`pp_free`].
#[unsafe(no_mangle)]
pub extern "C" fn pp_alloc(len: usize) -> *mut u8 {
    let mut buffer = vec![0u8; len].into_boxed_slice();
    let ptr = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    ptr
}

/// Gives back memory of [`pp_alloc`].
///
/// # Safety
///
/// `ptr` and `len` are as [`pp_alloc`] gave them, once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_free(ptr: *mut u8, len: usize) {
    if len == 0 {
        return;
    }
    // SAFETY: as the caller promises, a boxed slice of `len` bytes.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

/// Where the result buffer's text is.
#[unsafe(no_mangle)]
pub extern "C" fn pp_result_ptr() -> *const u8 {
    state(|state| state.result.as_ptr()).unwrap_or(std::ptr::null())
}

/// How long the result buffer's text is.
#[unsafe(no_mangle)]
pub extern "C" fn pp_result_len() -> usize {
    state(|state| state.result.len()).unwrap_or(0)
}

/// A device of the profile named (see [`crate::emulator::PROFILES`]),
/// its texts in the language named, dark if `dark` is 1; it replaces
/// the one before. 0, else 1 with the error in the result buffer.
///
/// # Safety
///
/// The texts are memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_new(
    profile_ptr: *const u8,
    profile_len: usize,
    language_ptr: *const u8,
    language_len: usize,
    dark: u32,
) -> u32 {
    std::panic::set_hook(Box::new(|info| {
        // A panic is no lack of memory: a refusal the device took its
        // way around before it is not why the call stopped (running out
        // aborts without a panic).
        paperpaw_emulator::budget::clear_last_refused();
        log(&info.to_string());
    }));
    paperpaw_emulator::host::set_random(random);
    paperpaw_emulator::host::set_clock(clock);
    // SAFETY: as the caller promises.
    let (profile, language) = unsafe {
        (
            text(profile_ptr, profile_len),
            text(language_ptr, language_len),
        )
    };
    // The device before goes first: one device's memory at a time.
    if state(|state| state.emulator = None).is_none() {
        return STOPPED;
    }
    let made = Emulator::new(profile, language, dark == 1);
    state(|state| match made {
        Ok(emulator) => {
            state.emulator = Some(emulator);
            0
        }
        Err(error) => {
            state.result = error.into_bytes();
            1
        }
    })
    .unwrap_or(STOPPED)
}

/// Puts a file of a data plugin on the card (`plugins/<plugin>/<path>`).
///
/// # Safety
///
/// The texts and the file are memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_put_file(
    plugin_ptr: *const u8,
    plugin_len: usize,
    path_ptr: *const u8,
    path_len: usize,
    data_ptr: *const u8,
    data_len: usize,
) -> u32 {
    // SAFETY: as the caller promises.
    let (plugin, path, data) = unsafe {
        (
            text(plugin_ptr, plugin_len),
            text(path_ptr, path_len),
            bytes(data_ptr, data_len),
        )
    };
    with(|emulator| emulator.put_file(plugin, path, data))
}

/// Takes a data plugin off the card.
///
/// # Safety
///
/// The text is memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_remove_plugin(plugin_ptr: *const u8, plugin_len: usize) -> u32 {
    // SAFETY: as the caller promises.
    let plugin = unsafe { text(plugin_ptr, plugin_len) };
    with(|emulator| {
        emulator.remove_plugin(plugin);
        Ok(())
    })
}

/// Starts the device, or starts it again with what is on the card now.
#[unsafe(no_mangle)]
pub extern "C" fn pp_start() -> u32 {
    with(Emulator::start)
}

/// A button pressed (`pressed` 1) or let go at `now_ms` (as
/// [`pp_advance`]); 1 if the device has none of that name.
///
/// # Safety
///
/// The name is memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_button(
    name_ptr: *const u8,
    name_len: usize,
    pressed: u32,
    now_ms: f64,
) -> u32 {
    // SAFETY: as the caller promises.
    let name = unsafe { text(name_ptr, name_len) };
    with(|emulator| {
        if emulator.button(name, pressed == 1, now_ms.max(0.0) as u64) {
            Ok(())
        } else {
            Err(format!("no button `{name}`"))
        }
    })
}

/// A finger on the screen: `phase` 0 down, 1 moved, 2 lifted, 3
/// cancelled, at `(x, y)` in the screen's pixels, at `now_ms` (as
/// [`pp_advance`]).
#[unsafe(no_mangle)]
pub extern "C" fn pp_pointer(phase: u32, x: i32, y: i32, now_ms: f64) -> u32 {
    let phase = match phase {
        0 => PointerPhase::Down,
        1 => PointerPhase::Move,
        2 => PointerPhase::Up,
        _ => PointerPhase::Cancel,
    };
    with(|emulator| {
        emulator.pointer(phase, x, y, now_ms.max(0.0) as u64);
        Ok(())
    })
}

/// Lets time pass to `now_ms` (`performance.now()` since the page
/// started); milliseconds until something is due, -1 if nothing is, -2
/// once the module stopped.
#[unsafe(no_mangle)]
pub extern "C" fn pp_advance(now_ms: f64) -> f64 {
    let mut next = -1.0;
    let status = with(|emulator| {
        if let Some(ms) = emulator.advance(now_ms.max(0.0) as u64) {
            next = ms as f64;
        }
        Ok(())
    });
    if status == STOPPED { -2.0 } else { next }
}

/// The screen made as large as a canvas of `width`×`height` pixels, as a
/// page that follows its window wants it; every open screen is made anew.
#[unsafe(no_mangle)]
pub extern "C" fn pp_resize(width: u32, height: u32) -> u32 {
    with(|emulator| {
        emulator.resize(width, height);
        Ok(())
    })
}

/// The screen's width in pixels.
#[unsafe(no_mangle)]
pub extern "C" fn pp_width() -> u32 {
    read(0, |e| u32::from(e.size().0))
}

/// The screen's height in pixels.
#[unsafe(no_mangle)]
pub extern "C" fn pp_height() -> u32 {
    read(0, |e| u32::from(e.size().1))
}

/// What the panel shows, RGBA, width × height × 4 bytes; valid until
/// the next call.
#[unsafe(no_mangle)]
pub extern "C" fn pp_frame() -> *const u8 {
    read(std::ptr::null(), |e| e.frame().as_ptr())
}

/// What the panel shows once a full refresh has flashed, RGBA, width ×
/// height × 4 bytes: for screenshots. Valid until the next call.
#[unsafe(no_mangle)]
pub extern "C" fn pp_shown() -> *const u8 {
    state(|state| {
        let shown = state.emulator.as_ref().map(Emulator::shown);
        state.shown = shown.unwrap_or_default();
        state.shown.as_ptr()
    })
    .unwrap_or(std::ptr::null())
}

/// What another device, night mode or the sleep screen shows (see
/// [`Emulator::snapshot`]): the profile named (empty: this one's), `dark`
/// 0 off, 1 on, 2 as now, `asleep` 1 for the sleep screen. Its pixels
/// at [`pp_shown`]'s place (call [`pp_snapshot_pixels`]), its size as
/// `<width>x<height>` in the result buffer.
///
/// # Safety
///
/// The name is memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_snapshot(
    profile_ptr: *const u8,
    profile_len: usize,
    dark: u32,
    asleep: u32,
) -> u32 {
    // SAFETY: as the caller promises.
    let profile = unsafe { text(profile_ptr, profile_len) };
    let profile = (!profile.is_empty()).then_some(profile);
    let dark = match dark {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    let mut made = None;
    let status = with(|emulator| {
        made = Some(emulator.snapshot(profile, dark, asleep == 1)?);
        Ok(())
    });
    if let Some((width, height, pixels)) = made {
        let _ = state(|state| {
            state.shown = pixels;
            state.result = format!("{width}x{height}").into_bytes();
        });
    }
    status
}

/// Where the pixels of the last [`pp_snapshot`] are.
#[unsafe(no_mangle)]
pub extern "C" fn pp_snapshot_pixels() -> *const u8 {
    state(|state| state.shown.as_ptr()).unwrap_or(std::ptr::null())
}

/// The size of an allocation the device's heap budget refused during the
/// last call, 0 if none was: asked after a trap, whether the device ran
/// out of memory there (the X4 would have stopped too). It reads no
/// state, so it answers after a panic as well.
#[unsafe(no_mangle)]
pub extern "C" fn pp_refused() -> u32 {
    u32::try_from(paperpaw_emulator::budget::last_refused()).unwrap_or(u32::MAX)
}

/// 1 while the panel still changes by itself (a full refresh flashing).
#[unsafe(no_mangle)]
pub extern "C" fn pp_animating() -> u32 {
    read(0, |e| u32::from(e.animating()))
}

/// PaperPaw's version into the result buffer (see
/// [`crate::emulator::version`]).
#[unsafe(no_mangle)]
pub extern "C" fn pp_version() -> u32 {
    answer(crate::emulator::version());
    0
}

/// The device as JSON into the result buffer (see
/// [`Emulator::device_json`]).
#[unsafe(no_mangle)]
pub extern "C" fn pp_device() -> u32 {
    let mut json = String::new();
    let failed = with(|emulator| {
        json = emulator.device_json();
        Ok(())
    });
    if failed == 0 {
        answer(json);
    }
    failed
}

/// Lets go of the recorded answers (see [`crate::network`]).
#[unsafe(no_mangle)]
pub extern "C" fn pp_fixtures_clear() -> u32 {
    with(|emulator| {
        emulator.set_fixtures(Vec::new());
        Ok(())
    })
}

/// Adds a recorded answer: the method (empty: any) and the address
/// pattern (`*` for any text) it answers, its status, headers (`name:
/// value` lines), body and delay (ms of the device's time).
///
/// # Safety
///
/// The texts and the body are memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_fixture_add(
    method_ptr: *const u8,
    method_len: usize,
    url_ptr: *const u8,
    url_len: usize,
    status: u32,
    headers_ptr: *const u8,
    headers_len: usize,
    body_ptr: *const u8,
    body_len: usize,
    delay_ms: f64,
) -> u32 {
    // SAFETY: as the caller promises.
    let (method, url, headers, body) = unsafe {
        (
            text(method_ptr, method_len),
            text(url_ptr, url_len),
            text(headers_ptr, headers_len),
            bytes(body_ptr, body_len),
        )
    };
    let fixture = Fixture {
        method: (!method.is_empty()).then(|| method.to_string()),
        url: url.to_string(),
        status: u16::try_from(status).unwrap_or(0),
        headers: headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
            .collect(),
        body: std::rc::Rc::from(body),
        delay_ms: delay_ms.max(0.0) as u64,
    };
    with(|emulator| {
        emulator.add_fixture(fixture);
        Ok(())
    })
}

/// Lets the page fetch what no recorded answer answers (`real` 1), or
/// not (0).
#[unsafe(no_mangle)]
pub extern "C" fn pp_network(real: u32) -> u32 {
    with(|emulator| {
        emulator.set_real_network(real == 1);
        Ok(())
    })
}

/// The requests plugins made since asked last, as JSON into the result
/// buffer (see [`Emulator::requests`]).
#[unsafe(no_mangle)]
pub extern "C" fn pp_requests() -> u32 {
    let mut json = String::new();
    let failed = with(|emulator| {
        json = emulator.requests();
        Ok(())
    });
    if failed == 0 {
        answer(json);
    }
    failed
}

/// The requests for the page to fetch since asked last, as JSON into the
/// result buffer (see [`Emulator::outbox`]).
#[unsafe(no_mangle)]
pub extern "C" fn pp_outbox() -> u32 {
    let mut json = String::new();
    let failed = with(|emulator| {
        json = emulator.outbox();
        Ok(())
    });
    if failed == 0 {
        answer(json);
    }
    failed
}

/// The page's answer to the request `id` it fetched: its status (0:
/// failed, the reason as the body), headers (`name: value` lines), body
/// and the address it came from after redirects (empty: the one asked),
/// at `now_ms` (as [`pp_advance`]).
///
/// # Safety
///
/// The texts and the body are memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_fetched(
    id: u32,
    status: u32,
    headers_ptr: *const u8,
    headers_len: usize,
    body_ptr: *const u8,
    body_len: usize,
    url_ptr: *const u8,
    url_len: usize,
    now_ms: f64,
) -> u32 {
    // SAFETY: as the caller promises.
    let (headers, body, url) = unsafe {
        (
            text(headers_ptr, headers_len),
            bytes(body_ptr, body_len),
            text(url_ptr, url_len),
        )
    };
    let status = u16::try_from(status).unwrap_or(0);
    let now = now_ms.max(0.0) as u64;
    with(|emulator| emulator.fetched(id, status, headers, body.to_vec(), url, now))
}

/// Carries out the remote control's command (an x4-remote line, see
/// [`Emulator::command`]) at `now_ms` (as [`pp_advance`]); its answer as
/// JSON into the result buffer.
///
/// # Safety
///
/// The line is memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_command(line_ptr: *const u8, line_len: usize, now_ms: f64) -> u32 {
    // SAFETY: as the caller promises.
    let line = unsafe { text(line_ptr, line_len) };
    let mut json = String::new();
    let failed = with(|emulator| {
        json = emulator.command(line, now_ms.max(0.0) as u64)?;
        Ok(())
    });
    if failed == 0 {
        answer(json);
    }
    failed
}

/// Whether the screen shown is `text` (its id) or shows it (see
/// [`Emulator::shows`]): 1 if so, 0 if not, [`STOPPED`].
///
/// # Safety
///
/// The text is memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_shows(text_ptr: *const u8, text_len: usize) -> u32 {
    // SAFETY: as the caller promises.
    let wanted = unsafe { text(text_ptr, text_len) };
    let mut shown = false;
    let status = with(|emulator| {
        shown = emulator.shows(wanted);
        Ok(())
    });
    if status == STOPPED {
        return STOPPED;
    }
    u32::from(shown)
}

/// Checks the data plugins and the plugins with code named with
/// `paperpaw-dev` (see [`Emulator::check`]): the commands and the code
/// plugins' ids, each separated by commas (no commands: all for data
/// plugins); the report as JSON into the result buffer.
///
/// # Safety
///
/// The texts are memory the script wrote ([`pp_alloc`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pp_check(
    commands_ptr: *const u8,
    commands_len: usize,
    code_ptr: *const u8,
    code_len: usize,
) -> u32 {
    let list = |text: &str| -> Vec<String> {
        text.split(',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(String::from)
            .collect()
    };
    // SAFETY: as the caller promises.
    let (commands, code) = unsafe {
        (
            list(text(commands_ptr, commands_len)),
            list(text(code_ptr, code_len)),
        )
    };
    let commands: Vec<&str> = commands.iter().map(String::as_str).collect();
    let code: Vec<&str> = code.iter().map(String::as_str).collect();
    let mut json = String::new();
    let failed = with(|emulator| {
        json = emulator.check(&commands, &code)?;
        Ok(())
    });
    if failed == 0 {
        answer(json);
    }
    failed
}
