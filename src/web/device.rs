use std::{collections::BTreeMap, time::Duration};
use tokio_util::sync::CancellationToken;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use rand::Rng;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    sync::{broadcast, mpsc::UnboundedSender},
    time::sleep,
};

use crate::{
    config::LocalConfig,
    scrcpy::{
        adb::{Adb, Device},
        constant::Keycode,
        control_msg::ScrcpyControlMsg,
        controller::ControllerCommand,
        device_action, managed_adb,
        media::AudioCodec,
        session::{self, ProjectionSession},
    },
    utils::{relate_to_root_path, share::ControlledDevice},
    web::{JsonResponse, WebServerError, ws::WebSocketNotification},
};

const SCRCPY_SERVER_VERSION: &str = "4.0";
#[derive(Debug, Clone)]
pub struct AppStateDevice {
    cs_tx: broadcast::Sender<ScrcpyControlMsg>,
    d_tx: UnboundedSender<ControllerCommand>,
    ws_tx: broadcast::Sender<WebSocketNotification>,
}

pub fn routers(
    cs_tx: broadcast::Sender<ScrcpyControlMsg>,
    d_tx: UnboundedSender<ControllerCommand>,
    ws_tx: broadcast::Sender<WebSocketNotification>,
) -> Router {
    Router::new()
        .route("/device_list", get(device_list))
        .route("/control_device", post(control_device))
        .route("/decontrol_device", post(decontrol_device))
        .route("/reconnect_device", post(reconnect_device))
        .route("/adb_restart", post(adb_restart))
        .route("/adb_screenshot", post(adb_screenshot))
        .route("/adb_apps", post(adb_apps))
        .route("/adb_displays", post(adb_displays))
        .route("/adb_start_app", post(adb_start_app))
        .route("/control/set_display_power", post(set_display_power))
        .route("/control/set_pointer_location", post(set_pointer_location))
        .route("/control/send_key", post(send_key))
        .with_state(AppStateDevice { cs_tx, d_tx, ws_tx })
}

async fn device_list() -> Result<JsonResponse, WebServerError> {
    let all_devices = managed_adb::devices()
        .await
        .map_err(WebServerError::internal_error)?
        .into_iter()
        .filter(|device| is_usb_device_id(&device.id))
        .collect::<Vec<_>>();
    Ok(JsonResponse::success(
        t!("web.device.deviceListObtained"),
        Some(json!({
            "controlled_devices": ControlledDevice::get_device_list().await,
            "adb_devices": all_devices, "projection": ProjectionSession::status(),
        })),
    ))
}

pub fn list_usb_devices() -> Result<Vec<Device>, String> {
    let config = LocalConfig::get();
    Adb::new(config.adb_path).devices().map(|devices| {
        devices
            .into_iter()
            .filter(|device| is_usb_device_id(&device.id))
            .collect()
    })
}

pub async fn start_usb_device(
    device_id: &str,
    d_tx: &UnboundedSender<ControllerCommand>,
    ws_tx: &broadcast::Sender<WebSocketNotification>,
) -> Result<(), String> {
    _control_device(device_id, d_tx, ws_tx)
        .await
        .map(|_| ())
        .map_err(|error| error.1)
}

pub fn restart_adb_and_list_usb_devices() -> Result<Vec<Device>, String> {
    let config = LocalConfig::get();
    Adb::new(config.adb_path).restart_server().map(|devices| {
        devices
            .into_iter()
            .filter(|device| is_usb_device_id(&device.id))
            .collect()
    })
}

fn gen_scid() -> String {
    let mut rng = rand::rng();
    let suffix: String = (0..6)
        .map(|_| rng.random_range(1..=9).to_string())
        .collect();
    format!("10{}", suffix) // ensure 8 digits(HEX) and less than MAX_INT32
}

fn is_usb_device_id(device_id: &str) -> bool {
    !device_id.contains(':')
        && !device_id.starts_with("emulator-")
        && !device_id.contains("._adb-tls-")
}

#[derive(Deserialize)]
struct PostDataControlDevice {
    device_id: String,
}

async fn _control_device(
    device_id: &str,
    d_tx: &UnboundedSender<ControllerCommand>,
    ws_tx: &broadcast::Sender<WebSocketNotification>,
) -> Result<JsonResponse, WebServerError> {
    session::new_intent();
    start_projection(device_id, 0, None, d_tx, ws_tx).await
}

pub async fn resume_usb_device(
    device_id: &str,
    retries: u8,
    intent: u64,
    d_tx: &UnboundedSender<ControllerCommand>,
    ws_tx: &broadcast::Sender<WebSocketNotification>,
) -> Result<(), String> {
    // Recover only this USB transport. Never restart the shared daemon automatically.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let mut reconnected = false;
    loop {
        if intent != session::intent() || !crate::membership::is_member() {
            return Err("自动恢复已取消".into());
        }
        let devices = managed_adb::devices().await?;
        match devices
            .iter()
            .find(|d| d.id == device_id)
            .map(|d| d.status.as_str())
        {
            Some("device") => break,
            Some("unauthorized") => return Err("请在设备上允许 USB 调试授权".into()),
            Some("offline") if !reconnected => {
                reconnected = true;
                let _ = managed_adb::run(
                    Some(device_id),
                    vec!["reconnect".into()],
                    Some(Duration::from_secs(2)),
                    CancellationToken::new(),
                )
                .await;
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("设备仍离线，请检查 USB 连接和调试授权后再投屏".into());
        }
        sleep(Duration::from_millis(400)).await;
    }
    start_projection(device_id, retries, Some(intent), d_tx, ws_tx)
        .await
        .map(|_| ())
        .map_err(|e| e.1)
}

async fn start_projection(
    device_id: &str,
    retries: u8,
    intent: Option<u64>,
    d_tx: &UnboundedSender<ControllerCommand>,
    ws_tx: &broadcast::Sender<WebSocketNotification>,
) -> Result<JsonResponse, WebServerError> {
    let _operation = session::OPERATIONS
        .try_lock()
        .map_err(|_| WebServerError::bad_request("正在处理投屏或重启，请稍后重试".into()))?;
    if !crate::membership::is_member() {
        return Err(WebServerError::bad_request("会员验证已失效".into()));
    }
    if intent.is_some_and(|i| i != session::intent()) {
        return Err(WebServerError::bad_request("自动恢复已取消".into()));
    }
    if !is_usb_device_id(device_id) {
        return Err(WebServerError::bad_request("仅支持 USB 实体设备".into()));
    }
    let devices = managed_adb::devices()
        .await
        .map_err(WebServerError::internal_error)?;
    match devices
        .iter()
        .find(|d| d.id == device_id)
        .map(|d| d.status.as_str())
    {
        Some("device") => {}
        Some("unauthorized") => {
            return Err(WebServerError::bad_request(
                "请在设备上允许 USB 调试授权".into(),
            ));
        }
        _ => {
            return Err(WebServerError::bad_request(
                "设备离线，请重新连接 USB 或检查 USB 调试".into(),
            ));
        }
    }
    session::cleanup_pending(device_id)
        .await
        .map_err(WebServerError::internal_error)?;
    if intent.is_some_and(|i| i != session::intent()) || !crate::membership::is_member() {
        return Err(WebServerError::bad_request(
            "投屏已取消或会员验证已失效".into(),
        ));
    }
    let scid = gen_scid();
    let projection = ProjectionSession::reserve(device_id.into(), scid.clone(), retries)
        .map_err(WebServerError::bad_request)?;
    let config = LocalConfig::get();
    let preparation: Result<(), String> = async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let local = format!(
            "tcp:{}",
            listener.local_addr().map_err(|e| e.to_string())?.port()
        );
        let remote = format!("localabstract:scrcpy_{scid}");
        let server =
            relate_to_root_path(["assets", &format!("JXZS-server-v{SCRCPY_SERVER_VERSION}")]);
        managed_adb::run(
            Some(device_id),
            vec![
                "push".into(),
                server.to_string_lossy().into_owned(),
                "/data/local/tmp/scrcpy-server.jar".into(),
            ],
            Some(Duration::from_secs(8)),
            projection.token.clone(),
        )
        .await?;
        managed_adb::run(
            Some(device_id),
            vec!["reverse".into(), remote, local],
            Some(Duration::from_secs(3)),
            projection.token.clone(),
        )
        .await?;
        if !crate::membership::is_member() || projection.token.is_cancelled() {
            return Err("投屏已取消或会员验证已失效".into());
        }
        let mut args = vec![
            format!("echo $$ > /data/local/tmp/jxzs-{scid}.pid;"),
            "CLASSPATH=/data/local/tmp/scrcpy-server.jar".into(),
            "exec".into(),
            "app_process".into(),
            "/".into(),
            "com.genymobile.scrcpy.Server".into(),
            SCRCPY_SERVER_VERSION.into(),
            format!("scid={scid}"),
            "video=true".into(),
            "display_id=0".into(),
            format!("audio={}", config.audio_enabled),
            format!("stay_awake={}", config.stay_awake),
            format!("screen_off_timeout={}", config.screen_off_timeout),
            format!("power_off_on_close={}", config.power_off_on_close),
            format!("video_codec={}", config.video_codec),
            format!("video_bit_rate={}", config.video_bit_rate),
        ];
        if config.capture_orientation >= 0 {
            args.push(format!(
                "capture_orientation=@{}",
                config.capture_orientation
            ));
        }
        if config.video_max_size > 0 {
            args.push(format!("max_size={}", config.video_max_size));
        }
        if config.video_max_fps > 0 {
            args.push(format!("max_fps={}", config.video_max_fps));
        }
        if config.audio_enabled {
            args.push(format!("audio_codec={}", config.audio_codec));
            args.push(format!("audio_source={}", config.audio_source));
            args.push(format!(
                "audio_dup={}",
                config.audio_source.is_playback() && config.audio_dup
            ));
            if !matches!(config.audio_codec, AudioCodec::Raw) {
                args.push(format!("audio_bit_rate={}", config.audio_bit_rate));
            }
        }
        let mut sockets = vec!["main_video".into()];
        if config.audio_enabled {
            sockets.push("main_audio".into());
        }
        sockets.push("main_control".into());
        ControlledDevice::add_device(device_id.into(), scid.clone(), true, sockets).await;
        d_tx.send(ControllerCommand::StartProjection {
            session: projection.clone(),
            listener,
            args,
            audio: config.audio_enabled,
        })
        .map_err(|e| e.to_string())?;
        Ok(())
    }
    .await;
    if let Err(e) = preparation {
        projection.token.cancel();
        ControlledDevice::remove_device(&scid).await;
        if managed_adb::cleanup(device_id, &scid).await.is_err() {
            session::remember_cleanup(device_id, &scid);
        }
        projection.finish(e.clone());
        let _ = ws_tx.send(WebSocketNotification::ScrcpyDeviceConnection {
            scid,
            main: true,
            connected: false,
        });
        return Err(WebServerError::internal_error(e));
    }
    if let Err(e) = projection.wait_ready().await {
        projection.stop();
        let _ = projection.wait_stopped().await;
        return Err(WebServerError::internal_error(e));
    }
    Ok(JsonResponse::success(
        "投屏已连接".into(),
        Some(json!({"scid": scid, "device_id": device_id})),
    ))
}

async fn control_device(
    State(state): State<AppStateDevice>,
    Json(payload): Json<PostDataControlDevice>,
) -> Result<JsonResponse, WebServerError> {
    let device_id = payload.device_id;

    _control_device(&device_id, &state.d_tx, &state.ws_tx).await
}

#[derive(Deserialize)]
struct PostDataReconnectDevice {
    device_id: String,
}

async fn reconnect_device(
    State(state): State<AppStateDevice>,
    Json(payload): Json<PostDataReconnectDevice>,
) -> Result<JsonResponse, WebServerError> {
    let device_id = payload.device_id;
    let device_list = ControlledDevice::get_device_list().await;
    for device in device_list {
        if device.device_id == device_id {
            _decontrol_device(&device_id, &state.d_tx).await?;
            _control_device(&device_id, &state.d_tx, &state.ws_tx).await?;
            return Ok(JsonResponse::success(
                format!("{}: {}", t!("web.device.reconnectDevice"), device_id),
                None,
            ));
        }
    }
    Err(WebServerError::bad_request(format!(
        "{}: {}",
        t!("web.device.deviceNotFound"),
        device_id
    )))
}

#[cfg(test)]
mod mvp_tests {
    use super::is_usb_device_id;

    #[test]
    fn accepts_physical_usb_serials() {
        assert!(is_usb_device_id("R58M1234ABC"));
        assert!(is_usb_device_id("1A2B3C4D5E"));
    }

    #[test]
    fn rejects_network_and_emulator_transports() {
        assert!(!is_usb_device_id("192.168.1.20:5555"));
        assert!(!is_usb_device_id("phone.local:5555"));
        assert!(!is_usb_device_id("emulator-5554"));
        assert!(!is_usb_device_id(
            "adb-R58M1234ABC-abc123._adb-tls-connect._tcp"
        ));
    }
}

#[derive(Deserialize)]
struct PostDataDeControlDevice {
    device_id: String,
}

async fn _decontrol_device(
    device_id: &str,
    _d_tx: &UnboundedSender<ControllerCommand>,
) -> Result<JsonResponse, WebServerError> {
    let _operation = session::OPERATIONS
        .try_lock()
        .map_err(|_| WebServerError::bad_request("正在处理设备操作，请稍后重试".into()))?;
    if let Some(projection) = ProjectionSession::current().filter(|s| s.device_id == device_id) {
        projection.stop();
        projection
            .wait_stopped()
            .await
            .map_err(WebServerError::internal_error)?;
    } else {
        session::new_intent();
    }
    Ok(JsonResponse::success("投屏已停止".into(), None))
}

async fn decontrol_device(
    State(state): State<AppStateDevice>,
    Json(payload): Json<PostDataDeControlDevice>,
) -> Result<JsonResponse, WebServerError> {
    let device_id = payload.device_id;
    _decontrol_device(&device_id, &state.d_tx).await
}

#[derive(Deserialize)]
struct PostDataAdbDevice {
    device_id: String,
}

#[derive(Deserialize)]
struct PostDataStartApp {
    device_id: String,
    package_name: String,
    component: String,
    display_id: i32,
    force_stop: bool,
}

#[derive(Debug, Clone, Serialize)]
struct AndroidApp {
    package_name: String,
    activity_name: String,
    component: String,
}

#[derive(Debug, Clone, Serialize)]
struct AndroidDisplay {
    display_id: i32,
    width: Option<u32>,
    height: Option<u32>,
    density: Option<u32>,
    rotation: Option<u32>,
    name: Option<String>,
}

async fn ensure_device_controlled(device_id: &str) -> Result<(), WebServerError> {
    let device_list = ControlledDevice::get_device_list().await;
    if device_list
        .iter()
        .any(|device| device.device_id == device_id)
    {
        Ok(())
    } else {
        Err(WebServerError::bad_request(format!(
            "{}: {}",
            t!("web.device.deviceNotFound"),
            device_id
        )))
    }
}

fn adb_shell_text<S>(device_id: &str, args: S) -> Result<String, String>
where
    S: IntoIterator,
    S::Item: Into<String>,
{
    let mut output = Vec::<u8>::new();
    Device::shell(device_id, args, &mut output)?;
    Ok(String::from_utf8_lossy(&output).to_string())
}

fn is_package_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

fn is_activity_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$')
}

fn is_valid_package_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.split('.').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

fn parse_component(component: &str) -> Option<AndroidApp> {
    let (package_name, activity_name) = component.split_once('/')?;
    if !is_valid_package_name(package_name)
        || activity_name.is_empty()
        || activity_name.len() > 255
        || !activity_name.chars().all(is_activity_char)
    {
        return None;
    }

    Some(AndroidApp {
        package_name: package_name.to_string(),
        activity_name: activity_name.to_string(),
        component: component.to_string(),
    })
}

fn is_valid_component(component: &str, package_name: &str) -> bool {
    parse_component(component)
        .map(|app| app.package_name == package_name)
        .unwrap_or(false)
}

fn parse_launcher_apps(output: &str) -> Vec<AndroidApp> {
    let mut apps = BTreeMap::new();
    for line in output.lines() {
        for candidate in
            line.split(|c: char| !(is_package_char(c) || is_activity_char(c) || c == '/'))
        {
            if !candidate.contains('/') {
                continue;
            }
            if let Some(app) = parse_component(candidate) {
                apps.entry(app.component.clone()).or_insert(app);
            }
        }
    }
    apps.into_values().collect()
}

fn parse_after_i32(text: &str, marker: &str) -> Option<i32> {
    let start = text.find(marker)? + marker.len();
    let rest = text[start..].trim_start();
    let end = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '-')
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn parse_after_u32(text: &str, marker: &str) -> Option<u32> {
    parse_after_i32(text, marker).and_then(|value| value.try_into().ok())
}

fn parse_display_name(line: &str) -> Option<String> {
    let start = line.find("DisplayInfo{\"")? + "DisplayInfo{\"".len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_display_size_after(line: &str, marker: &str) -> (Option<u32>, Option<u32>) {
    let Some(real_start) = line.find(marker) else {
        return (None, None);
    };
    let rest = &line[real_start + marker.len()..];
    let Some((width, rest)) = rest.split_once(" x ") else {
        return (None, None);
    };
    let width = width.trim().parse::<u32>().ok();
    let height_end = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    let height = rest[..height_end].parse::<u32>().ok();
    (width, height)
}

fn parse_display_size(line: &str) -> (Option<u32>, Option<u32>) {
    let real_size = parse_display_size_after(line, "real ");
    if real_size.0.is_some() && real_size.1.is_some() {
        return real_size;
    }

    parse_display_size_after(line, "app ")
}

fn parse_display_header_id(line: &str) -> Option<i32> {
    let rest = line.trim_start().strip_prefix("Display ")?;
    let end = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(index, _)| index)
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    rest[..end].parse().ok()
}

fn display_from_id(display_id: i32) -> Option<AndroidDisplay> {
    if display_id < 0 {
        return None;
    }

    Some(AndroidDisplay {
        display_id,
        width: None,
        height: None,
        density: None,
        rotation: None,
        name: None,
    })
}

fn merge_display(current: &mut AndroidDisplay, display: AndroidDisplay) {
    if current.width.is_none() {
        current.width = display.width;
    }
    if current.height.is_none() {
        current.height = display.height;
    }
    if current.density.is_none() {
        current.density = display.density;
    }
    if current.rotation.is_none() {
        current.rotation = display.rotation;
    }
    if current.name.is_none() {
        current.name = display.name;
    }
}

fn parse_displays(output: &str) -> Vec<AndroidDisplay> {
    let mut displays = BTreeMap::new();
    for line in output.lines() {
        let display = if line.contains("DisplayInfo{") && line.contains("displayId ") {
            let Some(display_id) = parse_after_i32(line, "displayId ") else {
                continue;
            };
            if display_id < 0 {
                continue;
            }

            let (width, height) = parse_display_size(line);
            AndroidDisplay {
                display_id,
                width,
                height,
                density: parse_after_u32(line, "density "),
                rotation: parse_after_u32(line, "rotation "),
                name: parse_display_name(line),
            }
        } else if let Some(display_id) = parse_display_header_id(line) {
            let Some(display) = display_from_id(display_id) else {
                continue;
            };
            display
        } else if let Some(display_id) = parse_after_i32(line, "mDisplayId=") {
            let Some(display) = display_from_id(display_id) else {
                continue;
            };
            display
        } else {
            continue;
        };

        displays
            .entry(display.display_id)
            .and_modify(|current| merge_display(current, display.clone()))
            .or_insert(display);
    }
    displays.into_values().collect()
}

fn query_launcher_apps(device_id: &str) -> Result<Vec<AndroidApp>, String> {
    let commands: [&[&str]; 3] = [
        &[
            "cmd",
            "package",
            "query-activities",
            "--brief",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
        ],
        &[
            "cmd",
            "package",
            "query-intent-activities",
            "--brief",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
        ],
        &[
            "pm",
            "query-intent-activities",
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
        ],
    ];

    let mut last_error = None;
    for command in commands {
        match adb_shell_text(device_id, command.iter().copied()) {
            Ok(output) => {
                let apps = parse_launcher_apps(&output);
                if !apps.is_empty() {
                    return Ok(apps);
                }
            }
            Err(error) => last_error = Some(error),
        }
    }

    if let Some(error) = last_error {
        Err(error)
    } else {
        Ok(Vec::new())
    }
}

async fn adb_apps(Json(payload): Json<PostDataAdbDevice>) -> Result<JsonResponse, WebServerError> {
    ensure_device_controlled(&payload.device_id).await?;

    let apps = query_launcher_apps(&payload.device_id).map_err(WebServerError::bad_request)?;
    if apps.is_empty() {
        return Err(WebServerError::bad_request(t!("web.device.noAppFound")));
    }

    Ok(JsonResponse::success(
        t!("web.device.getAdbAppsSuccess"),
        Some(json!({ "apps": apps })),
    ))
}

async fn adb_displays(
    Json(payload): Json<PostDataAdbDevice>,
) -> Result<JsonResponse, WebServerError> {
    ensure_device_controlled(&payload.device_id).await?;

    let output = adb_shell_text(&payload.device_id, ["dumpsys", "display"])
        .map_err(WebServerError::bad_request)?;
    let displays = parse_displays(&output);
    if displays.is_empty() {
        return Err(WebServerError::bad_request(t!("web.device.noDisplayFound")));
    }

    Ok(JsonResponse::success(
        t!("web.device.getAdbDisplaysSuccess"),
        Some(json!({ "displays": displays })),
    ))
}

async fn adb_start_app(
    Json(payload): Json<PostDataStartApp>,
) -> Result<JsonResponse, WebServerError> {
    ensure_device_controlled(&payload.device_id).await?;
    if !is_valid_package_name(&payload.package_name)
        || !is_valid_component(&payload.component, &payload.package_name)
        || payload.display_id < 0
    {
        return Err(WebServerError::bad_request(t!(
            "web.device.invalidStartAppParams"
        )));
    }

    let display_id = payload.display_id.to_string();
    if payload.force_stop {
        Device::shell_logged(
            &payload.device_id,
            ["am", "force-stop", &payload.package_name],
        )
        .map_err(WebServerError::bad_request)?;
    }

    Device::shell_logged(
        &payload.device_id,
        [
            "am",
            "start",
            "--display",
            &display_id,
            "-a",
            "android.intent.action.MAIN",
            "-c",
            "android.intent.category.LAUNCHER",
            "-n",
            &payload.component,
        ],
    )
    .map_err(WebServerError::bad_request)?;

    Ok(JsonResponse::success(
        t!("web.device.startAdbAppSuccess"),
        None,
    ))
}

#[derive(Deserialize)]
struct PostDataAddress {
    address: String,
}

async fn adb_connect(Json(payload): Json<PostDataAddress>) -> Result<JsonResponse, WebServerError> {
    let config = LocalConfig::get();
    let address = payload.address.trim().to_string();
    match Adb::new(config.adb_path).connect_device(&address) {
        Ok(_) => Ok(JsonResponse::success(
            format!("{}", t!("web.device.adbConnect", address => address)),
            None,
        )),
        Err(e) => Err(WebServerError::bad_request(format!(
            "{}: {}",
            t!("web.device.adbConnectFailed", address => address),
            e
        ))),
    }
}

#[derive(Deserialize)]
struct PostDataAdbPair {
    address: String,
    code: String,
}

async fn adb_pair(Json(payload): Json<PostDataAdbPair>) -> Result<JsonResponse, WebServerError> {
    let config = LocalConfig::get();
    match Adb::new(config.adb_path).pair_device(&payload.address, &payload.code) {
        Ok(_) => Ok(JsonResponse::success(
            format!(
                "{}",
                t!("web.device.adbPairSuccess", address => payload.address, code => payload.code)
            ),
            None,
        )),
        Err(e) => Err(WebServerError::bad_request(format!(
            "{}: {}",
            t!("web.device.adbPairFailed", address => payload.address, code => payload.code),
            e
        ))),
    }
}

async fn adb_restart() -> Result<JsonResponse, WebServerError> {
    let _operation = session::OPERATIONS
        .try_lock()
        .map_err(|_| WebServerError::bad_request("正在处理设备操作，请稍后重试".into()))?;
    let selected = ProjectionSession::current().map(|s| s.device_id.clone());
    ProjectionSession::stop_current()
        .await
        .map_err(WebServerError::internal_error)?;
    let token = CancellationToken::new();
    if let Some(device) = selected.as_deref() {
        let _ = managed_adb::run(
            Some(device),
            vec!["reconnect".into()],
            Some(Duration::from_secs(2)),
            token.clone(),
        )
        .await;
    }
    managed_adb::run(
        None,
        vec!["kill-server".into()],
        Some(Duration::from_secs(2)),
        token.clone(),
    )
    .await
    .map_err(WebServerError::internal_error)?;
    managed_adb::run(
        None,
        vec!["start-server".into()],
        Some(Duration::from_secs(3)),
        token,
    )
    .await
    .map_err(WebServerError::internal_error)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    let adb_devices = loop {
        let devices = managed_adb::devices()
            .await
            .map_err(WebServerError::internal_error)?;
        if devices
            .iter()
            .any(|d| is_usb_device_id(&d.id) && d.status == "device")
            || tokio::time::Instant::now() >= deadline
        {
            break devices;
        }
        sleep(Duration::from_millis(300)).await;
    };
    let message = if adb_devices
        .iter()
        .any(|d| is_usb_device_id(&d.id) && d.status == "device")
    {
        "USB 调试连接已恢复，可以重新投屏"
    } else if adb_devices.iter().any(|d| d.status == "unauthorized") {
        "ADB 已重启，请在设备上允许 USB 调试授权"
    } else {
        "ADB 已重启，但设备仍未上线，请重新连接 USB 或关闭再开启 USB 调试"
    };
    Ok(JsonResponse::success(
        message.into(),
        Some(json!({
            "controlled_devices": ControlledDevice::get_device_list().await, "adb_devices": adb_devices, "projection": ProjectionSession::status(),
        })),
    ))
}

#[derive(Deserialize)]
struct PostDataId {
    id: String,
}

async fn adb_screenshot(
    Json(payload): Json<PostDataId>,
) -> Result<impl IntoResponse, WebServerError> {
    let image_bytes = capture_adb_screenshot(&payload.id).map_err(WebServerError::bad_request)?;

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("image/png"));
    headers.insert("Cache-Control", HeaderValue::from_static("no-cache"));

    Ok((StatusCode::OK, headers, image_bytes))
}

pub fn capture_adb_screenshot(id: &str) -> Result<Vec<u8>, String> {
    let src = "/data/local/tmp/_screenshot_scrcpy_mask.png";

    let mut display_id_info = Vec::new();
    Device::shell(
        id,
        ["dumpsys", "SurfaceFlinger", "--display-id"],
        &mut display_id_info,
    )
    .map_err(|e| format!("failed get display id: {}", e))?;
    let text = String::from_utf8_lossy(&display_id_info);
    let first_line = text
        .lines()
        .next()
        .ok_or_else(|| "no display found".to_string())?;
    let display_id = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "invalid display line".to_string())?;

    Device::shell_logged(id, ["screencap", "-p", "-d", display_id, src])
        .map_err(|e| format!("{} {}: {}", t!("web.device.screenshotError"), id, e))?;

    let mut image_bytes = Vec::<u8>::new();
    Device::pull(id, src.to_string(), &mut image_bytes)
        .map_err(|e| format!("{}: {}", t!("web.device.failedGetScreenshotFile"), e))?;

    Device::shell_logged(id, ["rm", src])
        .map_err(|e| format!("{} {}: {}", t!("web.device.failedRemoveScreenshot"), id, e))?;
    Ok(image_bytes)
}

#[derive(Deserialize)]
struct PostDataSetDisplayPower {
    mode: bool,
}
async fn set_display_power(
    State(state): State<AppStateDevice>,
    Json(payload): Json<PostDataSetDisplayPower>,
) -> Result<JsonResponse, WebServerError> {
    if !ControlledDevice::is_any_device_controlled().await {
        return Err(WebServerError::bad_request(t!(
            "web.device.noDeviceControlled"
        )));
    }

    device_action::set_display_power(&state.cs_tx, payload.mode);
    Ok(JsonResponse::success(
        t!("web.device.setDisplayPowerSuccess"),
        None,
    ))
}

#[derive(Deserialize)]
struct PostDataSetPointerLocation {
    mode: bool,
}

async fn set_pointer_location(
    Json(payload): Json<PostDataSetPointerLocation>,
) -> Result<JsonResponse, WebServerError> {
    let device_list = ControlledDevice::get_device_list().await;
    if device_list.is_empty() {
        return Err(WebServerError::bad_request(t!(
            "web.device.noDeviceControlled"
        )));
    }

    let mode = if payload.mode { "1" } else { "0" };
    for device in device_list {
        let mut output = Vec::<u8>::new();
        Device::shell(
            &device.device_id,
            ["settings", "put", "system", "pointer_location", mode],
            &mut output,
        )
        .map_err(|e| {
            WebServerError::bad_request(format!(
                "{} {}: {}",
                t!("web.device.setPointerLocationFailed"),
                device.device_id,
                e
            ))
        })?;
    }

    Ok(JsonResponse::success(
        t!("web.device.setPointerLocationSuccess"),
        None,
    ))
}

#[derive(Deserialize)]
struct PostDataSendKey {
    keycode: Keycode,
}

async fn send_key(
    State(state): State<AppStateDevice>,
    Json(payload): Json<PostDataSendKey>,
) -> Result<JsonResponse, WebServerError> {
    if !ControlledDevice::is_any_device_controlled().await {
        return Err(WebServerError::bad_request(t!(
            "web.device.noDeviceControlled"
        )));
    }

    device_action::inject_keycode(&state.cs_tx, payload.keycode);
    Ok(JsonResponse::success(t!("web.device.sendKeySuccess"), None))
}
