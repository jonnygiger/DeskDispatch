use crate::api_client::WorkerApiClient;
use crate::config::WorkerConfig;
use crate::engine::capture::capture_screen;
use chrono::Utc;
use deskdispatch_protocol::RecordingEventItem;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;

const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;

const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

const KEY_A: u16 = 30;

const KEY_LEFTCTRL: u16 = 29;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTALT: u16 = 56;
const KEY_RIGHTALT: u16 = 100;
const KEY_LEFTMETA: u16 = 125;
const KEY_RIGHTMETA: u16 = 126;

const BATCH_SIZE: usize = 25;
const FLUSH_INTERVAL: Duration = Duration::from_millis(700);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(2);
const SCREENSHOT_DEBOUNCE: Duration = Duration::from_millis(700);

#[repr(C)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct InputAbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceRole {
    Keyboard,
    PrimaryPointer,
    SecondaryPointer,
}

#[derive(Debug)]
struct RawInput {
    role: DeviceRole,
    event_type: u16,
    code: u16,
    value: i32,
    time_us: i64,
    abs: Option<InputAbsInfo>,
}

struct DeviceInfo {
    path: PathBuf,
    name: String,
    role: DeviceRole,
    abs_x: InputAbsInfo,
    abs_y: InputAbsInfo,
}

struct Modifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
}

impl Modifiers {
    fn new() -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            super_key: false,
        }
    }

    fn apply(&mut self, code: u16, pressed: bool) {
        match code {
            KEY_LEFTCTRL | KEY_RIGHTCTRL => self.ctrl = pressed,
            KEY_LEFTALT | KEY_RIGHTALT => self.alt = pressed,
            KEY_LEFTSHIFT | KEY_RIGHTSHIFT => self.shift = pressed,
            KEY_LEFTMETA | KEY_RIGHTMETA => self.super_key = pressed,
            _ => {}
        }
    }

    fn prefix(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("ctrl");
        }
        if self.alt {
            parts.push("alt");
        }
        if self.shift {
            parts.push("shift");
        }
        if self.super_key {
            parts.push("super");
        }
        parts.join("+")
    }
}

fn is_modifier(code: u16) -> bool {
    matches!(
        code,
        KEY_LEFTCTRL | KEY_RIGHTCTRL | KEY_LEFTSHIFT | KEY_RIGHTSHIFT | KEY_LEFTALT
            | KEY_RIGHTALT | KEY_LEFTMETA | KEY_RIGHTMETA
    )
}

fn key_name(code: u16) -> Option<&'static str> {
    let name = match code {
        1 => "Escape",
        2..=11 => return Some(DIGITS[(code - 2) as usize]),
        12 => "minus",
        13 => "equal",
        14 => "BackSpace",
        15 => "Tab",
        16..=25 => return Some(LETTERS_QWERTY[(code - 16) as usize]),
        26 => "bracketleft",
        27 => "bracketright",
        28 => "Return",
        30..=38 => return Some(LETTERS_HOME[(code - 30) as usize]),
        39 => "semicolon",
        40 => "apostrophe",
        41 => "grave",
        43 => "backslash",
        44..=50 => return Some(LETTERS_BOTTOM[(code - 44) as usize]),
        51 => "comma",
        52 => "period",
        53 => "slash",
        55 => "KP_Multiply",
        57 => "space",
        58 => "Caps_Lock",
        59..=68 => return Some(F_KEYS[(code - 59) as usize]),
        69 => "Num_Lock",
        70 => "Scroll_Lock",
        71..=79 => return Some(KP_DIGITS[(code - 71) as usize]),
        80 => "KP_7",
        81 => "KP_8",
        82 => "KP_0",
        83 => "KP_Decimal",
        87 => "F11",
        88 => "F12",
        96 => "KP_Enter",
        98 => "KP_Divide",
        99 => "Print",
        102 => "Home",
        103 => "Up",
        104 => "Prior",
        105 => "Left",
        106 => "Right",
        107 => "End",
        108 => "Down",
        109 => "Next",
        110 => "Insert",
        111 => "Delete",
        119 => "Pause",
        127 => "Menu",
        _ => return None,
    };
    Some(name)
}

static DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
static LETTERS_QWERTY: [&str; 10] = ["q", "w", "e", "r", "t", "y", "u", "i", "o", "p"];
static LETTERS_HOME: [&str; 9] = ["a", "s", "d", "f", "g", "h", "j", "k", "l"];
static LETTERS_BOTTOM: [&str; 7] = ["z", "x", "c", "v", "b", "n", "m"];
static F_KEYS: [&str; 10] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10",
];
static KP_DIGITS: [&str; 9] = ["KP_1", "KP_2", "KP_3", "KP_4", "KP_5", "KP_6", "KP_7", "KP_8", "KP_9"];

fn eviocgbit(event_type: u8, len: usize) -> libc::c_ulong {
    ((2u64 << 30) | ((len as u64) << 16) | (u64::from(b'E') << 8) | (0x20 + u64::from(event_type)))
        as libc::c_ulong
}

fn eviocgabs(abs: u8) -> libc::c_ulong {
    ((2u64 << 30) | (24u64 << 16) | (u64::from(b'E') << 8) | (0x40 + u64::from(abs)))
        as libc::c_ulong
}

fn test_bit(bits: &[u8], code: u16) -> bool {
    let byte = code as usize / 8;
    bits.get(byte)
        .map(|b| b & (1u8 << (code % 8)) != 0)
        .unwrap_or(false)
}

fn read_capabilities(file: &File, event_type: u16, len: usize) -> Vec<u8> {
    let mut bits = vec![0u8; len];
    // SAFETY: the kernel writes at most `len` bytes into the valid `bits` buffer.
    let ret = unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            eviocgbit(event_type as u8, len),
            bits.as_mut_ptr(),
        )
    };
    if ret < 0 { Vec::new() } else { bits }
}

fn read_abs_info(file: &File, abs: u8) -> Option<InputAbsInfo> {
    let mut info = InputAbsInfo::default();
    // SAFETY: EVIOCGABS writes a single `input_absinfo` into `info`.
    let ret = unsafe { libc::ioctl(file.as_raw_fd(), eviocgabs(abs), &mut info) };
    if ret < 0 { None } else { Some(info) }
}

fn read_device_name(file: &File) -> String {
    let mut buf = [0u8; 256];
    // SAFETY: EVIOCGNAME writes at most 255 bytes plus NUL into `buf`.
    let ret = unsafe { libc::ioctl(file.as_raw_fd(), 0x80ff4506u64 as libc::c_ulong, buf.as_mut_ptr()) };
    if ret < 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

fn default_abs_range() -> InputAbsInfo {
    InputAbsInfo {
        value: 0,
        minimum: 0,
        maximum: 32767,
        fuzz: 0,
        flat: 0,
        resolution: 0,
    }
}

fn enumerate_devices() -> Vec<DeviceInfo> {
    let mut keyboards = Vec::new();
    let mut pointers = Vec::new();

    for index in 0..32 {
        let path = PathBuf::from(format!("/dev/input/event{index}"));
        if !path.exists() {
            continue;
        }

        let file = match OpenOptions::new().read(true).open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let name = read_device_name(&file);
        let key_bits = read_capabilities(&file, EV_KEY, 96);
        let rel_bits = read_capabilities(&file, EV_REL, 8);
        let abs_bits = read_capabilities(&file, EV_ABS, 8);

        let keyboard = test_bit(&key_bits, KEY_A);
        let has_abs_xy = test_bit(&abs_bits, ABS_X) && test_bit(&abs_bits, ABS_Y);
        let has_rel_xy = test_bit(&rel_bits, REL_X) && test_bit(&rel_bits, REL_Y);
        let has_button = test_bit(&key_bits, BTN_LEFT);

        if keyboard {
            keyboards.push(DeviceInfo {
                path,
                name,
                role: DeviceRole::Keyboard,
                abs_x: default_abs_range(),
                abs_y: default_abs_range(),
            });
            continue;
        }

        if has_abs_xy || has_rel_xy {
            let abs_x = read_abs_info(&file, ABS_X as u8).unwrap_or_else(default_abs_range);
            let abs_y = read_abs_info(&file, ABS_Y as u8).unwrap_or_else(default_abs_range);
            pointers.push((
                DeviceInfo {
                    path,
                    name,
                    role: DeviceRole::SecondaryPointer,
                    abs_x,
                    abs_y,
                },
                has_abs_xy,
                has_button,
            ));
        }
    }

    let primary_index = pointers
        .iter()
        .position(|(d, abs, btn)| *abs && *btn && d.name.to_lowercase().contains("tablet"))
        .or_else(|| pointers.iter().position(|(_, abs, btn)| *abs && *btn))
        .or_else(|| pointers.iter().position(|(_, abs, _)| *abs))
        .or_else(|| pointers.iter().position(|(_, _, btn)| *btn))
        .or(if pointers.is_empty() { None } else { Some(0) });

    let mut devices = keyboards;
    for (index, (mut device, _, _)) in pointers.into_iter().enumerate() {
        if Some(index) == primary_index {
            device.role = DeviceRole::PrimaryPointer;
        }
        devices.push(device);
    }

    devices
}

fn spawn_reader(
    device: DeviceInfo,
    tx: mpsc::UnboundedSender<RawInput>,
    stop: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let file = match OpenOptions::new().read(true).open(&device.path) {
            Ok(f) => f,
            Err(e) => {
                warn!(
                    path = %device.path.display(),
                    name = %device.name,
                    "Failed to open input device for recording: {}",
                    e
                );
                return;
            }
        };

        info!(
            path = %device.path.display(),
            name = %device.name,
            role = ?device.role,
            "Recording input device"
        );

        let fd = file.as_raw_fd();
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let mut buffer = [0u8; std::mem::size_of::<InputEvent>() * 64];

        while stop.load(Ordering::Relaxed) {
            // SAFETY: `poll_fd` points to a single valid pollfd structure.
            let ready = unsafe { libc::poll(&mut poll_fd, 1, 200) };
            if ready < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            if ready == 0 {
                continue;
            }

            // SAFETY: buffer is sized to hold whole `InputEvent` records and is valid for writes.
            let bytes_read = unsafe {
                libc::read(
                    fd,
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if bytes_read <= 0 {
                continue;
            }

            let count = bytes_read as usize / std::mem::size_of::<InputEvent>();
            for i in 0..count {
                // SAFETY: each chunk is `size_of::<InputEvent>()` bytes inside the populated buffer.
                let ev = unsafe {
                    std::ptr::read_unaligned(
                        buffer
                            .as_ptr()
                            .add(i * std::mem::size_of::<InputEvent>())
                            .cast::<InputEvent>(),
                    )
                };
                if ev.type_ != EV_KEY && ev.type_ != EV_REL && ev.type_ != EV_ABS {
                    continue;
                }
                let abs = if ev.type_ == EV_ABS {
                    match ev.code {
                        ABS_X => Some(device.abs_x),
                        ABS_Y => Some(device.abs_y),
                        _ => None,
                    }
                } else {
                    None
                };
                let input = RawInput {
                    role: device.role,
                    event_type: ev.type_,
                    code: ev.code,
                    value: ev.value,
                    time_us: ev.tv_sec * 1_000_000 + ev.tv_usec,
                    abs,
                };
                if tx.send(input).is_err() {
                    return;
                }
            }
        }
    })
}

fn query_xdotool_pointer(display: &str) -> Option<(i32, i32)> {
    let output = std::process::Command::new("xdotool")
        .env("DISPLAY", display)
        .args(["getmouselocation", "--shell"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut x = None;
    let mut y = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("X=") {
            x = value.trim().parse::<i32>().ok();
        } else if let Some(value) = line.strip_prefix("Y=") {
            y = value.trim().parse::<i32>().ok();
        }
    }
    match (x, y) {
        (Some(x), Some(y)) => Some((x, y)),
        _ => None,
    }
}

fn scale_abs(info: &InputAbsInfo, raw: i32, screen_extent: u32) -> i32 {
    let min = info.minimum;
    let max = info.maximum;
    if max <= min || screen_extent == 0 {
        return raw;
    }
    let scaled = (f64::from(raw - min) / f64::from(max - min) * f64::from(screen_extent - 1))
        .round() as i32;
    scaled.clamp(0, screen_extent as i32 - 1)
}

struct RecorderState {
    sequence: i32,
    pointer_x: i32,
    pointer_y: i32,
    modifiers: Modifiers,
    last_screenshot: Option<Instant>,
    last_click: Option<(String, i64)>,
    screen_width: u32,
    screen_height: u32,
    display: String,
}

impl RecorderState {
    fn next_sequence(&mut self) -> i32 {
        self.sequence += 1;
        self.sequence
    }
}

async fn capture_and_upload(
    api_client: &WorkerApiClient,
    session_id: i64,
    display: &str,
    width: u32,
    height: u32,
) -> Result<String, String> {
    let upload = api_client
        .get_recording_screenshot_upload_url(session_id)
        .await?;

    let display = display.to_string();
    let png_bytes = tokio::task::spawn_blocking(move || {
        let image = capture_screen(&display, width, height);
        image.encode_png()
    })
    .await
    .map_err(|e| format!("Screenshot capture task failed: {}", e))??;

    api_client
        .upload_image_to_presigned_url(&upload.upload_url, png_bytes)
        .await?;

    Ok(upload.object_key)
}

async fn flush_events(
    api_client: &WorkerApiClient,
    session_id: i64,
    events: &mut Vec<RecordingEventItem>,
) -> Result<bool, String> {
    if events.is_empty() {
        let response = api_client
            .post_recording_events(session_id, Vec::new())
            .await?;
        return Ok(response.stop_requested);
    }

    let batch = events.clone();
    let response = api_client
        .post_recording_events(session_id, batch)
        .await?;
    events.clear();
    Ok(response.stop_requested)
}

/// Capture and stream desktop input events for an active recording session.
pub async fn run_recording_session(
    config: &WorkerConfig,
    api_client: &WorkerApiClient,
    recording_session_id: i64,
) -> Result<(), String> {
    let devices = enumerate_devices();
    if devices.is_empty() {
        return Err(
            "No readable input devices found. Add the worker user to the 'input' group (and restart the worker) to enable recording."
                .to_string(),
        );
    }

    info!(
        recording_session_id = recording_session_id,
        device_count = devices.len(),
        "Starting desktop input recording"
    );

    let stop = Arc::new(AtomicBool::new(true));
    let (tx, mut rx) = mpsc::unbounded_channel::<RawInput>();

    let mut handles = Vec::new();
    for device in devices {
        let handle = spawn_reader(device, tx.clone(), stop.clone());
        handles.push(handle);
    }
    drop(tx);

    let mut state = RecorderState {
        sequence: 0,
        pointer_x: if config.screen_width > 0 {
            (config.screen_width / 2) as i32
        } else {
            0
        },
        pointer_y: if config.screen_height > 0 {
            (config.screen_height / 2) as i32
        } else {
            0
        },
        modifiers: Modifiers::new(),
        last_screenshot: None,
        last_click: None,
        screen_width: config.screen_width.max(1),
        screen_height: config.screen_height.max(1),
        display: config.display.clone(),
    };

    if let Some((x, y)) = query_xdotool_pointer(&config.display) {
        state.pointer_x = x;
        state.pointer_y = y;
        debug!(x = x, y = y, "Initialized pointer position from xdotool");
    }

    let mut events: Vec<RecordingEventItem> = Vec::new();
    let mut last_flush = Instant::now();
    let mut last_keepalive = Instant::now();
    let mut recording = true;

    while recording {
        let received = tokio::time::timeout(Duration::from_millis(250), rx.recv()).await;

        match received {
            Ok(Some(input)) => {
                handle_raw_input(
                    input,
                    &mut state,
                    api_client,
                    recording_session_id,
                    &mut events,
                )
                .await;
            }
            Ok(None) => break,
            Err(_) => {}
        }

        let should_flush = events.len() >= BATCH_SIZE || last_flush.elapsed() >= FLUSH_INTERVAL;
        let should_keepalive = last_keepalive.elapsed() >= KEEPALIVE_INTERVAL;

        if should_flush || (events.is_empty() && should_keepalive) {
            match flush_events(api_client, recording_session_id, &mut events).await {
                Ok(stop_requested) => {
                    last_flush = Instant::now();
                    last_keepalive = Instant::now();
                    if stop_requested {
                        info!(
                            recording_session_id = recording_session_id,
                            "Server requested recording stop"
                        );
                        recording = false;
                    }
                }
                Err(e) => {
                    error!(
                        recording_session_id = recording_session_id,
                        "Failed to post recording events: {}", e
                    );
                    // Avoid hammering the server while it is unreachable.
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    last_flush = Instant::now();
                    last_keepalive = Instant::now();
                }
            }
        }
    }

    stop.store(false, Ordering::Relaxed);
    for handle in handles {
        let _ = handle.join();
    }

    if let Err(e) = api_client
        .post_worker_stop_recording(recording_session_id)
        .await
    {
        warn!(
            recording_session_id = recording_session_id,
            "Failed to finalize recording session: {}", e
        );
    } else {
        info!(
            recording_session_id = recording_session_id,
            "Recording session finalized"
        );
    }

    Ok(())
}

async fn handle_raw_input(
    input: RawInput,
    state: &mut RecorderState,
    api_client: &WorkerApiClient,
    session_id: i64,
    events: &mut Vec<RecordingEventItem>,
) {
    match (input.role, input.event_type) {
        (DeviceRole::Keyboard, EV_KEY) => {
            state.modifiers.apply(input.code, input.value != 0);
            if is_modifier(input.code) {
                return;
            }
            if input.value != 1 {
                return;
            }
            let Some(name) = key_name(input.code) else {
                return;
            };
            let prefix = state.modifiers.prefix();
            let combo = if prefix.is_empty() {
                name.to_string()
            } else {
                format!("{}+{}", prefix, name)
            };
            let sequence = state.next_sequence();
            events.push(RecordingEventItem {
                sequence_number: sequence,
                event_type: "key_press".to_string(),
                x: None,
                y: None,
                button: None,
                key_combo: Some(combo),
                screenshot_object_key: None,
                captured_at: Some(Utc::now()),
            });
        }
        (DeviceRole::PrimaryPointer | DeviceRole::SecondaryPointer, EV_ABS) => {
            if let Some(info) = input.abs {
                if input.code == ABS_X {
                    state.pointer_x = scale_abs(&info, input.value, state.screen_width);
                } else if input.code == ABS_Y {
                    state.pointer_y = scale_abs(&info, input.value, state.screen_height);
                }
            }
        }
        (DeviceRole::PrimaryPointer | DeviceRole::SecondaryPointer, EV_REL) => {
            if input.code == REL_X {
                state.pointer_x =
                    (state.pointer_x + input.value).clamp(0, state.screen_width.saturating_sub(1) as i32);
            } else if input.code == REL_Y {
                state.pointer_y =
                    (state.pointer_y + input.value).clamp(0, state.screen_height.saturating_sub(1) as i32);
            }
        }
        (DeviceRole::PrimaryPointer | DeviceRole::SecondaryPointer, EV_KEY) => {
            let button = match input.code {
                BTN_LEFT => "left",
                BTN_RIGHT => "right",
                BTN_MIDDLE => "middle",
                _ => return,
            };
            if input.value != 1 {
                return;
            }

            // Mouse motion and button events can come from different kernel devices
            // (e.g. the VirtualBox tablet reports ABS coordinates while the emulated
            // PS/2 mouse reports the buttons). Ignore the same click if a second
            // device reports it within a few milliseconds.
            if let Some((last_button, last_time)) = &state.last_click {
                if last_button == button && (input.time_us - *last_time).abs() <= 50_000 {
                    return;
                }
            }
            state.last_click = Some((button.to_string(), input.time_us));

            let should_capture = state
                .last_screenshot
                .map(|last| last.elapsed() >= SCREENSHOT_DEBOUNCE)
                .unwrap_or(true);

            let screenshot = if should_capture {
                match capture_and_upload(
                    api_client,
                    session_id,
                    &state.display,
                    state.screen_width,
                    state.screen_height,
                )
                .await
                {
                    Ok(key) => {
                        state.last_screenshot = Some(Instant::now());
                        Some(key)
                    }
                    Err(e) => {
                        debug!("Recording screenshot failed: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            let sequence = state.next_sequence();
            events.push(RecordingEventItem {
                sequence_number: sequence,
                event_type: "mouse_click".to_string(),
                x: Some(state.pointer_x),
                y: Some(state.pointer_y),
                button: Some(button.to_string()),
                key_combo: None,
                screenshot_object_key: screenshot,
                captured_at: Some(Utc::now()),
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_letter_and_modifier_keys() {
        assert_eq!(key_name(30), Some("a"));
        assert_eq!(key_name(28), Some("Return"));
        assert_eq!(key_name(57), Some("space"));
        assert_eq!(key_name(59), Some("F1"));
        assert_eq!(key_name(99), Some("Print"));
        assert_eq!(key_name(999), None);
    }

    #[test]
    fn builds_modifier_combos() {
        let mut modifiers = Modifiers::new();
        assert_eq!(modifiers.prefix(), "");
        modifiers.apply(KEY_LEFTCTRL, true);
        modifiers.apply(KEY_LEFTSHIFT, true);
        assert_eq!(modifiers.prefix(), "ctrl+shift");
        modifiers.apply(KEY_LEFTCTRL, false);
        assert_eq!(modifiers.prefix(), "shift");
    }

    #[test]
    fn scales_absolute_coordinates() {
        let info = InputAbsInfo {
            value: 0,
            minimum: 0,
            maximum: 32767,
            fuzz: 0,
            flat: 0,
            resolution: 0,
        };
        assert_eq!(scale_abs(&info, 0, 1920), 0);
        assert_eq!(scale_abs(&info, 32767, 1920), 1919);
        assert_eq!(scale_abs(&info, 16383, 1920), 959);
    }

    #[test]
    fn detects_bits() {
        let bits = [0b0000_0100u8];
        assert!(test_bit(&bits, 2));
        assert!(!test_bit(&bits, 3));
    }
}
