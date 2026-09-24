//! Windows desktop shell for the complete management UI.
//!
//! The React/Ant Design frontend is the single source of truth for device,
//! mapping and settings screens. It is hosted inside the application window
//! with WebView2, so users never need to open an external browser and native
//! and browser control surfaces cannot drift apart again.

use bevy::{
    prelude::*,
    window::{Monitor, MonitorSelection, PrimaryMonitor, RawHandleWrapper, WindowCloseRequested, WindowPosition, WindowResolution},
};
use std::time::Duration;
use wry::{
    Rect, WebView, WebViewBuilder,
    dpi::{PhysicalPosition, PhysicalSize},
};

use crate::config::LocalConfig;

pub struct DesktopWebViewPlugin;

#[derive(Component)]
pub struct ManagementWindow;

struct DesktopWebViewState {
    webview: Option<WebView>,
    retry_timer: Timer,
    last_size: UVec2,
    last_visible: bool,
    last_member: Option<bool>,
}

impl Default for DesktopWebViewState {
    fn default() -> Self {
        Self {
            webview: None,
            // Give the local Axum server time to bind before first navigation.
            retry_timer: Timer::new(Duration::from_millis(500), TimerMode::Repeating),
            last_size: UVec2::ZERO,
            last_visible: false,
            last_member: None,
        }
    }
}

impl Plugin for DesktopWebViewPlugin {
    fn build(&self, app: &mut App) {
        app.insert_non_send_resource(DesktopWebViewState::default())
            .add_systems(PostStartup, spawn_management_window)
            .add_systems(Update, (sync_desktop_webview, exit_when_management_closes));
    }
}

fn exit_when_management_closes(
    mut close_events: MessageReader<WindowCloseRequested>,
    management: Query<Entity, With<ManagementWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    for event in close_events.read() {
        if management.get(event.window).is_ok() {
            exit.write(AppExit::Success);
        }
    }
}

fn spawn_management_window(
    mut commands: Commands,
    monitor: Query<&Monitor, With<PrimaryMonitor>>,
    mut state: NonSendMut<DesktopWebViewState>,
) {
    let member = crate::membership::is_member();
    let (width, height) = management_window_size(monitor.iter().next(), member);
    state.last_member = Some(member);
    commands.spawn((
        Window {
            title: "JX手游助手".into(),
            resolution: WindowResolution::new(width, height),
            position: WindowPosition::Centered(MonitorSelection::Primary),
            decorations: true,
            transparent: false,
            resizable: true,
            visible: false,
            focused: false,
            ..default()
        },
        ManagementWindow,
    ));
}

fn management_window_size(monitor: Option<&Monitor>, member: bool) -> (u32, u32) {
    let (screen_width, screen_height) = monitor
        .map(|monitor| (monitor.physical_width, monitor.physical_height))
        .unwrap_or((1152, 648));

    if member {
        (
            (screen_width as f32 * 0.6).round() as u32,
            (screen_height as f32 * 0.6).round() as u32,
        )
    } else {
        // Keep the membership gate as the only compact window until a valid
        // member session exists. The projection window remains independently
        // hidden until the user starts projection.
        (520.min(screen_width), 680.min(screen_height))
    }
}

fn webview_bounds(window: &Window) -> (Rect, UVec2) {
    let width = window.resolution.physical_width().max(1);
    let content_height = window.resolution.physical_height().max(1);
    let size = UVec2::new(width, content_height);
    (
        Rect {
            position: PhysicalPosition::new(0, 0).into(),
            size: PhysicalSize::new(width, content_height).into(),
        },
        size,
    )
}

fn sync_desktop_webview(
    time: Res<Time>,
    mut state: NonSendMut<DesktopWebViewState>,
    mut windows: Query<(&mut Window, &RawHandleWrapper), With<ManagementWindow>>,
    monitors: Query<&Monitor, With<PrimaryMonitor>>,
) {
    let Ok((mut window, raw_handle)) = windows.single_mut() else {
        return;
    };

    let member = crate::membership::is_member();
    if state.last_member != Some(member) {
        let (width, height) = management_window_size(monitors.iter().next(), member);
        window.resolution.set(width as f32, height as f32);
        window.position = WindowPosition::Centered(MonitorSelection::Primary);
        state.last_member = Some(member);
        log::info!(
            "[DesktopUI] resized management window for {} state",
            if member { "active member" } else { "membership" }
        );
    }

    let should_show = true;
    let (bounds, content_size) = webview_bounds(&window);

    if state.webview.is_none() {
        state.retry_timer.tick(time.delta());
        if !state.retry_timer.just_finished() {
            return;
        }

        let config = LocalConfig::get();
        let host = if config.web_bind_addr.is_unspecified() {
            "127.0.0.1".to_string()
        } else {
            config.web_bind_addr.to_string()
        };
        let url = format!("http://{host}:{}", config.web_port);
        // SAFETY: this system is forced onto Bevy's main thread by NonSendMut,
        // which is the thread on which the primary Windows handle is valid.
        let handle = unsafe { raw_handle.get_handle() };
        match WebViewBuilder::new()
            .with_url(&url)
            .with_bounds(bounds)
            .with_devtools(cfg!(debug_assertions))
            .build_as_child(&handle)
        {
            Ok(webview) => {
                if let Err(error) = webview.set_visible(should_show) {
                    log::warn!("[DesktopUI] failed to set initial visibility: {error}");
                }
                state.webview = Some(webview);
                state.last_size = content_size;
                state.last_visible = should_show;
                window.visible = true;
                window.focused = true;
                log::info!("[DesktopUI] WebView2 management UI loaded: {url}");
            }
            Err(error) => {
                log::error!("[DesktopUI] WebView2 initialization failed: {error}");
            }
        }
        return;
    }

    let size_changed = state.last_size != content_size;
    let visibility_changed = state.last_visible != should_show;
    if size_changed {
        let result = state.webview.as_ref().unwrap().set_bounds(bounds);
        if let Err(error) = result {
            log::warn!("[DesktopUI] failed to resize WebView2: {error}");
        } else {
            state.last_size = content_size;
        }
    }
    if visibility_changed {
        let result = state.webview.as_ref().unwrap().set_visible(should_show);
        if let Err(error) = result {
            log::warn!("[DesktopUI] failed to change visibility: {error}");
        } else {
            state.last_visible = should_show;
        }
    }
}
