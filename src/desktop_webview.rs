//! Windows desktop shell for the complete management UI.
//!
//! The React/Ant Design frontend is the single source of truth for device,
//! mapping and settings screens. It is hosted inside the application window
//! with WebView2, so users never need to open an external browser and native
//! and browser control surfaces cannot drift apart again.

use bevy::{
    prelude::*,
    window::{PrimaryWindow, RawHandleWrapper},
};
use std::time::Duration;
use wry::{
    Rect, WebView, WebViewBuilder,
    dpi::{PhysicalPosition, PhysicalSize},
};

use crate::{
    config::LocalConfig,
    mask::ui::basic::{ProjectionBodyMarker, TITLEBAR_HEIGHT},
};

pub struct DesktopWebViewPlugin;

struct DesktopWebViewState {
    webview: Option<WebView>,
    retry_timer: Timer,
    last_size: UVec2,
    last_visible: bool,
}

impl Default for DesktopWebViewState {
    fn default() -> Self {
        Self {
            webview: None,
            // Give the local Axum server time to bind before first navigation.
            retry_timer: Timer::new(Duration::from_millis(500), TimerMode::Repeating),
            last_size: UVec2::ZERO,
            last_visible: false,
        }
    }
}

impl Plugin for DesktopWebViewPlugin {
    fn build(&self, app: &mut App) {
        app.insert_non_send_resource(DesktopWebViewState::default())
            .add_systems(Update, sync_desktop_webview);
    }
}

fn webview_bounds(window: &Window) -> (Rect, UVec2) {
    let scale = window.resolution.scale_factor() as f64;
    let width = window.resolution.physical_width().max(1);
    let full_height = window.resolution.physical_height().max(1);
    let titlebar = (TITLEBAR_HEIGHT as f64 * scale).round() as u32;
    let content_height = full_height.saturating_sub(titlebar).max(1);
    let size = UVec2::new(width, content_height);
    (
        Rect {
            position: PhysicalPosition::new(0, titlebar as i32).into(),
            size: PhysicalSize::new(width, content_height).into(),
        },
        size,
    )
}

fn sync_desktop_webview(
    time: Res<Time>,
    mut state: NonSendMut<DesktopWebViewState>,
    mut windows: Query<(&mut Window, &RawHandleWrapper), With<PrimaryWindow>>,
    projection: Query<&Node, With<ProjectionBodyMarker>>,
) {
    let Ok((mut window, raw_handle)) = windows.single_mut() else {
        return;
    };

    let projecting = projection
        .iter()
        .any(|node| node.display != Display::None);
    let should_show = !projecting;
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
