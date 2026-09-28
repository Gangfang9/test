//! USB-only Android pointer presentation. Touch injection stays in the existing
//! member-gated control channel; the Android companion never grants authority.
use crate::{
    config::LocalConfig,
    mask::{
        MaskResizeState,
        mapping::{
            MappingState,
            cursor::{CursorFrameSet, CursorPosition, CursorState},
            utils::ControlMsgHelper,
        },
        mask_command::MaskSize,
        ui::basic::ProjectionToolbarCapture,
    },
    scrcpy::constant::MotionEventAction,
    utils::{ChannelSenderCS, share::ControlledDevice},
};
use bevy::{prelude::*, window::PrimaryWindow};
use serde::Serialize;
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    process::{Command, Stdio},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

static CONNECTED: AtomicBool = AtomicBool::new(false);
static FRAME: OnceLock<Mutex<(PointerFrame, Instant)>> = OnceLock::new();
const POINTER_ID: u64 = u64::MAX - 3;

#[derive(Clone, Copy, Serialize, Debug)]
struct PointerFrame {
    v: u8,
    visible: bool,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}
impl Default for PointerFrame {
    fn default() -> Self {
        Self {
            v: 1,
            visible: false,
            x: 0.,
            y: 0.,
            width: 1.,
            height: 1.,
        }
    }
}

#[derive(Resource, Default)]
pub struct CompanionPointer {
    pub visible: bool,
    restore_mapping: Option<MappingState>,
    touch_down: bool,
}

pub struct CompanionPlugin;
impl Plugin for CompanionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CompanionPointer>()
            .add_systems(Startup, start_transport)
            .add_systems(
                Update,
                toggle_pointer.before(CursorFrameSet::UpdatePosition),
            )
            .add_systems(Update, pointer_click.after(CursorFrameSet::ApplyCapture))
            .add_systems(
                Update,
                publish_pointer.after(CursorFrameSet::SyncVirtualCursor),
            );
    }
}

pub fn pointer_mode(pointer: Res<CompanionPointer>) -> bool {
    pointer.visible
}

fn toggle_pointer(
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mapping: Res<State<MappingState>>,
    mut next_mapping: ResMut<NextState<MappingState>>,
    mut next_cursor: ResMut<NextState<CursorState>>,
    mut pointer: ResMut<CompanionPointer>,
    cs: Res<ChannelSenderCS>,
    pos: Res<CursorPosition>,
    size: Res<MaskSize>,
) {
    let available =
        CONNECTED.load(Ordering::Acquire) && crate::membership::is_member() && window.visible;
    let toggle = available && window.focused && keys.just_pressed(KeyCode::Backquote);
    if pointer.visible && (!available || toggle) {
        if pointer.touch_down {
            ControlMsgHelper::send_touch(&cs.0, MotionEventAction::Up, POINTER_ID, size.0, pos.0);
            pointer.touch_down = false;
        }
        pointer.visible = false;
        if let Some(previous) = pointer.restore_mapping.take() {
            if crate::membership::is_member() && window.visible {
                next_mapping.set(previous);
            }
        }
    } else if toggle {
        pointer.restore_mapping = Some(*mapping.get());
        pointer.visible = true;
        // Existing Stop transitions release all held touches/scripts before
        // desktop pointer mode begins, avoiding stuck movement/fire keys.
        next_mapping.set(MappingState::Stop);
        next_cursor.set(CursorState::Normal);
    }
}

fn pointer_click(
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    cursor_state: Res<State<CursorState>>,
    mapping: Res<State<MappingState>>,
    mut pointer: ResMut<CompanionPointer>,
    pos: Res<CursorPosition>,
    size: Res<MaskSize>,
    cs: Res<ChannelSenderCS>,
    resize: Res<MaskResizeState>,
    toolbar: Res<ProjectionToolbarCapture>,
) {
    let valid = pointer.visible
        && *mapping.get() == MappingState::Stop
        && *cursor_state.get() == CursorState::Normal
        && window.focused
        && window.visible
        && !resize.active()
        && !toolbar.active()
        && crate::membership::is_member()
        && pos.0.x >= 0.
        && pos.0.y >= 0.
        && pos.0.x < size.0.x
        && pos.0.y < size.0.y;
    if pointer.touch_down && (!valid || buttons.just_released(MouseButton::Left)) {
        ControlMsgHelper::send_touch(&cs.0, MotionEventAction::Up, POINTER_ID, size.0, pos.0);
        pointer.touch_down = false;
    } else if valid && buttons.just_pressed(MouseButton::Left) {
        ControlMsgHelper::send_touch(&cs.0, MotionEventAction::Down, POINTER_ID, size.0, pos.0);
        pointer.touch_down = true;
    } else if valid && pointer.touch_down && buttons.pressed(MouseButton::Left) {
        ControlMsgHelper::send_touch(&cs.0, MotionEventAction::Move, POINTER_ID, size.0, pos.0);
    }
}

fn frame_for(pos: Vec2, size: Vec2, visible: bool) -> PointerFrame {
    if !pos.is_finite() || !size.is_finite() || size.x <= 0. || size.y <= 0. {
        return PointerFrame::default();
    }
    PointerFrame {
        visible: visible && pos.x >= 0. && pos.y >= 0. && pos.x < size.x && pos.y < size.y,
        x: (pos.x / size.x).clamp(0., 1.),
        y: (pos.y / size.y).clamp(0., 1.),
        width: size.x,
        height: size.y,
        v: 1,
    }
}

fn publish_pointer(
    pointer: Res<CompanionPointer>,
    pos: Res<CursorPosition>,
    size: Res<MaskSize>,
    window: Single<&Window, With<PrimaryWindow>>,
    cursor: Res<State<CursorState>>,
    resize: Res<MaskResizeState>,
    toolbar: Res<ProjectionToolbarCapture>,
) {
    let visible = pointer.visible
        && window.visible
        && window.focused
        && *cursor.get() == CursorState::Normal
        && !resize.active()
        && !toolbar.active()
        && crate::membership::is_member();
    *FRAME
        .get_or_init(|| Mutex::new((PointerFrame::default(), Instant::now())))
        .lock()
        .unwrap() = (frame_for(pos.0, size.0, visible), Instant::now());
}

// ADB operations run outside Bevy and have bounded waits. A missing APK never
// blocks rendering, existing projection, or membership checks.
fn adb(device: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(LocalConfig::get().adb_path);
    cmd.args(["-s", device])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                child.stdout.take()?.read_to_string(&mut out).ok()?;
                return Some(out.trim().to_string());
            }
            Ok(None) if start.elapsed() < Duration::from_secs(2) => {
                thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn start_transport() {
    FRAME.get_or_init(|| Mutex::new((PointerFrame::default(), Instant::now())));
    thread::Builder::new().name("jxzs-android-pointer".into()).spawn(|| loop {
        let Some(device) = ControlledDevice::get_main_device_blocking() else {
            thread::sleep(Duration::from_millis(500)); continue;
        };
        if !crate::membership::is_member() { thread::sleep(Duration::from_millis(500)); continue; }
        let Some(port) = adb(&device.device_id, &["forward", "tcp:0", "localabstract:jxzs_cursor_v1"])
            .and_then(|value| value.parse::<u16>().ok()) else {
            thread::sleep(Duration::from_secs(2)); continue;
        };
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        if let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            let _ = stream.set_nodelay(true);
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
            let mut hello = [0u8; 7];
            if stream.read_exact(&mut hello).is_ok() && &hello == b"JXZS/1\n" {
                CONNECTED.store(true, Ordering::Release);
                loop {
                    let current = ControlledDevice::get_main_device_blocking();
                    if !current.is_some_and(|d| d.scid == device.scid && d.device_id == device.device_id) { break; }
                    let (mut frame, updated) = *FRAME.get().unwrap().lock().unwrap();
                    if updated.elapsed() > Duration::from_millis(250) || !crate::membership::is_member() { frame.visible = false; }
                    let mut bytes = serde_json::to_vec(&frame).unwrap(); bytes.push(b'\n');
                    if stream.write_all(&bytes).is_err() { break; }
                    thread::sleep(Duration::from_millis(33));
                }
                let _ = stream.write_all(b"{\"v\":1,\"visible\":false,\"x\":0,\"y\":0,\"width\":1,\"height\":1}\n");
            }
        }
        CONNECTED.store(false, Ordering::Release);
        let endpoint = format!("tcp:{port}");
        let _ = adb(&device.device_id, &["forward", "--remove", &endpoint]);
        thread::sleep(Duration::from_secs(1));
    }).expect("start companion transport thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalized_hotspot_tracks_video_without_dpi_or_resolution_drift() {
        let frame = frame_for(Vec2::new(320., 180.), Vec2::new(1280., 720.), true);
        assert!(frame.visible);
        assert_eq!((frame.x, frame.y), (0.25, 0.25));
        let scaled = frame_for(Vec2::new(640., 360.), Vec2::new(2560., 1440.), true);
        assert_eq!((scaled.x, scaled.y), (frame.x, frame.y));
    }
    #[test]
    fn invalid_or_outside_coordinates_never_show_pointer() {
        assert!(!frame_for(Vec2::ZERO, Vec2::ZERO, true).visible);
        assert!(!frame_for(Vec2::new(f32::NAN, 1.), Vec2::ONE, true).visible);
        assert!(!frame_for(Vec2::new(-1., 20.), Vec2::new(100., 100.), true).visible);
        assert!(!frame_for(Vec2::new(100., 20.), Vec2::new(100., 100.), true).visible);
    }
}
