use std::{
    fs::File,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream},
    sync::OnceLock,
    time::Duration,
};

use bevy::{
    log::{BoxedLayer, LogPlugin, tracing_subscriber::Layer},
    prelude::*,
    window::{PresentMode, WindowLevel},
};
use scrcpy_mask::{
    DEFAULT_LANGUAGE,
    config::LocalConfig,
    is_available_language,
    mask::{MaskPlugins, mask_command::MaskCommand},
    native_ui::NativeUiPlugin,
    scrcpy::{
        control_msg::ScrcpyControlMsg,
        controller::{self, ControllerCommand},
    },
    tokio_tasks::TokioTasksPlugin,
    utils::{
        ChannelReceiverM, ChannelReceiverV, ChannelSenderCS, ChannelSenderD, ChannelSenderM,
        ChannelSenderWS,
        LatestVideoFrame, relate_to_data_path,
    },
    web::{self, ws::WebSocketNotification},
};
#[cfg(target_os = "windows")]
use scrcpy_mask::desktop_webview::DesktopWebViewPlugin;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing_appender::non_blocking::WorkerGuard;

static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();
const PORT_FALLBACK_ATTEMPTS: u16 = 512;

fn existing_instance(config: &LocalConfig) -> bool {
    let connect_ip = if config.web_bind_addr.is_unspecified() {
        Ipv4Addr::LOCALHOST
    } else {
        config.web_bind_addr
    };
    let addr = SocketAddr::V4(SocketAddrV4::new(connect_ip, config.web_port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(Duration::from_millis(800)))
        .is_err()
        || stream
            .set_write_timeout(Some(Duration::from_millis(800)))
            .is_err()
        || stream
            .write_all(
                b"GET /api/config/get_config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .is_err()
    {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok()
        && response.starts_with("HTTP/1.1 200")
        && response.contains("\"web_port\"")
        && response.contains("\"controller_port\"")
}

fn server_ports_available(config: &LocalConfig, controller_port: u16, web_port: u16) -> bool {
    if controller_port == web_port {
        return false;
    }
    let Ok(controller_listener) =
        TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, controller_port))
    else {
        return false;
    };
    let Ok(web_listener) = TcpListener::bind(SocketAddrV4::new(config.web_bind_addr, web_port))
    else {
        return false;
    };
    drop(web_listener);
    drop(controller_listener);
    true
}

fn select_available_server_ports(config: &LocalConfig) -> Option<(u16, u16)> {
    for offset in 0..=PORT_FALLBACK_ATTEMPTS {
        let controller_port = config.controller_port.checked_add(offset)?;
        let web_port = config.web_port.checked_add(offset)?;
        if server_ports_available(config, controller_port, web_port) {
            return Some((controller_port, web_port));
        }
    }
    None
}

fn log_custom_layer(_app: &mut App) -> Option<BoxedLayer> {
    let file = File::create(relate_to_data_path(["app.log"])).unwrap_or_else(|e| {
        panic!("Failed to create log file: {}", e);
    });
    let (non_blocking, guard) = tracing_appender::non_blocking(file);
    let _ = LOG_GUARD.set(guard);
    Some(
        bevy::log::tracing_subscriber::fmt::layer()
            .with_writer(non_blocking)
            .with_file(false)
            .with_line_number(true)
            .with_ansi(false)
            .boxed(),
    )
}

fn main() {
    rust_i18n::set_locale(DEFAULT_LANGUAGE);

    if let Err(e) = LocalConfig::load() {
        println!("LocalConfig load failed. {}", e);
    }
    LocalConfig::prefer_bundled_adb();

    let mut local_config = LocalConfig::get();
    // update language
    let language = local_config.language.clone();
    if is_available_language(&language) {
        rust_i18n::set_locale(&language);
    } else {
        rust_i18n::set_locale(DEFAULT_LANGUAGE);
        LocalConfig::set_language(DEFAULT_LANGUAGE.to_string());
        local_config = LocalConfig::get();
    }

    if existing_instance(&local_config) {
        eprintln!("JX手游助手 is already running.");
        return;
    }

    let Some((controller_port, web_port)) = select_available_server_ports(&local_config) else {
        eprintln!("Unable to find available local ports for JX手游助手.");
        return;
    };
    if controller_port != local_config.controller_port || web_port != local_config.web_port {
        println!(
            "Configured ports are occupied; switching controller {} -> {} and web {} -> {}.",
            local_config.controller_port, controller_port, local_config.web_port, web_port
        );
        LocalConfig::set_controller_port(controller_port);
        LocalConfig::set_web_port(web_port);
        local_config = LocalConfig::get();
    }
    // update config file
    LocalConfig::save().unwrap();

    ffmpeg_next::init().unwrap();

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(LogPlugin {
                custom_layer: log_custom_layer,
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "JX手游助手".into(),
                    has_shadow: false,
                    transparent: true, // for windows: https://github.com/bevyengine/bevy/issues/7544
                    decorations: false,
                    present_mode: PresentMode::AutoVsync,
                    resizable: true,
                    visible: true,
                    focused: true,
                    window_level: if local_config.always_on_top {
                        WindowLevel::AlwaysOnTop
                    } else {
                        WindowLevel::Normal
                    },
                    #[cfg(target_os = "macos")]
                    composite_alpha_mode: bevy::window::CompositeAlphaMode::PostMultiplied,
                    ..default()
                }),
                ..default()
            }),
    )
    .add_plugins(TokioTasksPlugin::default())
    .add_plugins(MaskPlugins)
    .add_plugins(NativeUiPlugin)
    .add_systems(Startup, start_servers);

    #[cfg(target_os = "windows")]
    app.add_plugins(DesktopWebViewPlugin);

    #[cfg(target_os = "macos")]
    {
        app.insert_resource(bevy::ecs::schedule::MainThreadExecutor::default())
            .add_systems(Startup, macos_menu);
    }

    #[cfg(not(target_os = "macos"))]
    {
        use scrcpy_mask::window_alpha;
        app.add_systems(Startup, window_alpha::detect_alpha_mode);
        app.add_systems(PostStartup, window_alpha::apply_alpha_mode);
    }

    app.run();
}

#[cfg(target_os = "macos")]
fn macos_menu(executor: Res<bevy::ecs::schedule::MainThreadExecutor>) {
    use muda::{Menu, Submenu};
    // remove default menu
    executor
        .0
        .spawn(async move {
            let menu = Menu::new();
            let submenu = Submenu::new("JX手游助手", true);
            menu.append(&submenu).unwrap();
            menu.init_for_nsapp();
        })
        .detach();
}

fn start_servers(mut commands: Commands) {
    let config = LocalConfig::get();
    let web_addr = SocketAddrV4::new(config.web_bind_addr, config.web_port);
    let controller_addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, config.controller_port);

    let (cs_tx, _) = broadcast::channel::<ScrcpyControlMsg>(1000);
    let (ws_tx, _) = broadcast::channel::<WebSocketNotification>(1000);
    let v_channel = LatestVideoFrame::default();
    let (m_tx, m_rx) =
        crossbeam_channel::unbounded::<(MaskCommand, oneshot::Sender<Result<String, String>>)>();
    let (d_tx, d_rx) = mpsc::unbounded_channel::<ControllerCommand>();

    commands.insert_resource(ChannelSenderCS(cs_tx.clone()));
    commands.insert_resource(ChannelReceiverV(v_channel.clone()));
    commands.insert_resource(ChannelReceiverM(m_rx));
    commands.insert_resource(ChannelSenderM(m_tx.clone()));
    commands.insert_resource(ChannelSenderD(d_tx.clone()));
    commands.insert_resource(ChannelSenderWS(ws_tx.clone()));
    // Keep the complete legacy API available during the native migration, but
    // never open a browser. Remove this only after native feature parity.
    web::Server::start(
        web_addr,
        cs_tx.clone(),
        d_tx,
        m_tx.clone(),
        ws_tx.clone(),
        false,
    );
    controller::Controller::start(controller_addr, cs_tx, v_channel, d_rx, m_tx, ws_tx);
}
