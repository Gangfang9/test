//! Native client for the cloud membership gateway. No U验证 key enters the WebView.

use crate::web::JsonResponse;
use crate::{scrcpy::controller::ControllerCommand, utils::share::ControlledDevice};
use axum::{
    Json, Router,
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    process::Command,
    sync::{OnceLock, RwLock},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

const GATEWAYS: [&str; 2] = ["https://jxzs.host.mg", "https://www.jxzs.top"];
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const HEARTBEAT_GRACE: Duration = Duration::from_secs(90);

#[derive(Default)]
struct Session {
    token: Option<String>,
    account: Option<String>,
    member: bool,
    checked: Option<Instant>,
}
static SESSION: OnceLock<RwLock<Session>> = OnceLock::new();
static DEVICE_ID: OnceLock<Result<String, String>> = OnceLock::new();
fn session() -> &'static RwLock<Session> {
    SESSION.get_or_init(|| RwLock::new(Session::default()))
}

pub fn is_member() -> bool {
    let state = session().read().expect("membership lock poisoned");
    state.member
        && state
            .checked
            .is_some_and(|at| at.elapsed() < HEARTBEAT_GRACE)
}

#[cfg(target_os = "windows")]
fn fingerprint() -> Result<String, String> {
    let output = Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .map_err(|_| "无法读取本机机器码".to_string())?;
    if !output.status.success() {
        return Err("无法读取本机机器码".into());
    }
    let text = if output.stdout.starts_with(&[0xff, 0xfe]) {
        String::from_utf16_lossy(
            &output.stdout[2..]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let guid = text
        .lines()
        .find(|s| s.contains("MachineGuid"))
        .and_then(|s| s.split_whitespace().last())
        .filter(|s| s.len() >= 16)
        .ok_or_else(|| "本机机器码格式错误".to_string())?;
    // Use PCI adapters regardless of which one currently has a network link;
    // otherwise changing from Wi-Fi to Ethernet would change the device ID.
    let mac_output = Command::new("powershell.exe")
        .args([
            "-NoProfile", "-NonInteractive", "-Command",
            "Get-CimInstance Win32_NetworkAdapter | Where-Object { $_.PNPDeviceID -like 'PCI*' -and $_.MACAddress } | Sort-Object MACAddress | Select-Object -ExpandProperty MACAddress",
        ])
        .output()
        .map_err(|_| "无法读取网卡识别码".to_string())?;
    if !mac_output.status.success() {
        return Err("无法读取网卡识别码".into());
    }
    let mac_text = String::from_utf8_lossy(&mac_output.stdout);
    let macs: Vec<_> = mac_text
        .lines()
        .map(str::trim)
        .filter(|part| {
            part.len() == 17 && part.as_bytes().iter().filter(|&&c| c == b':').count() == 5
        })
        .collect();
    if macs.is_empty() {
        return Err("没有可用的网卡识别码".into());
    }
    // U验证 accepts at most 64 characters for udid. The full BLAKE3 hex
    // digest fits that limit and keeps the displayed last eight characters.
    Ok(blake3::hash(
        format!("{}:{}", guid.to_lowercase(), macs.join(",").to_lowercase()).as_bytes(),
    )
    .to_hex()
    .to_string())
}
#[cfg(not(target_os = "windows"))]
fn fingerprint() -> Result<String, String> {
    Err("会员客户端仅支持 Windows".into())
}
fn device_id() -> Result<&'static str, String> {
    DEVICE_ID
        .get_or_init(fingerprint)
        .as_ref()
        .map(String::as_str)
        .map_err(Clone::clone)
}

#[derive(Deserialize)]
struct GatewayReply {
    ok: bool,
    data: Option<Value>,
    error: Option<String>,
}
type ApiError = (StatusCode, Json<JsonResponse>);
type ApiResult = Result<Json<JsonResponse>, ApiError>;
fn error(status: StatusCode, msg: impl Into<String>) -> ApiError {
    (status, Json(JsonResponse::new(status.as_u16(), msg, None)))
}
fn success(data: Value) -> Json<JsonResponse> {
    Json(JsonResponse::success("成功", Some(data)))
}

async fn cloud(path: &str, body: Value, token: Option<&str>) -> Result<Value, ApiError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "无法初始化网络连接"))?;
    let device = device_id().map_err(|msg| error(StatusCode::SERVICE_UNAVAILABLE, msg))?;
    for (index, gateway) in GATEWAYS.iter().enumerate() {
        let mut request = client
            .post(format!("{gateway}/v1/{path}"))
            .json(&body)
            .header("X-Device-ID", device.clone());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(request_error) => {
                let category = if request_error.is_timeout() {
                    "timeout"
                } else if request_error.is_connect() {
                    "connect/dns/tls"
                } else {
                    "request"
                };
                // A connect failure occurs before the request is sent. Do not
                // retry ambiguous timeouts or HTTP errors: register and redeem
                // might already have changed server state.
                log::warn!(
                    "[Membership] {path} request to {gateway} failed ({category}): {request_error:#}"
                );
                if request_error.is_connect() && index + 1 < GATEWAYS.len() {
                    continue;
                }
                return Err(error(StatusCode::SERVICE_UNAVAILABLE, "无法连接服务器"));
            }
        };
        let status = response.status();
        let reply: GatewayReply = response
            .json()
            .await
            .map_err(|_| error(StatusCode::BAD_GATEWAY, "会员服务器返回格式错误"))?;
        if !status.is_success() || !reply.ok {
            return Err(error(
                if status.is_client_error() {
                    status
                } else {
                    StatusCode::BAD_GATEWAY
                },
                reply.error.unwrap_or_else(|| "会员验证失败".into()),
            ));
        }
        return reply
            .data
            .ok_or_else(|| error(StatusCode::BAD_GATEWAY, "会员服务器缺少数据"));
    }
    Err(error(StatusCode::SERVICE_UNAVAILABLE, "无法连接服务器"))
}

#[derive(Deserialize)]
struct Credentials {
    account: String,
    password: String,
}
#[derive(Deserialize)]
struct Card {
    card: String,
}
#[derive(Serialize)]
struct LocalStatus {
    logged_in: bool,
    member: bool,
    account: Option<String>,
    device_suffix: Option<String>,
}
fn local_status() -> Value {
    let state = session().read().expect("membership lock poisoned");
    json!(LocalStatus {
        logged_in: state.token.is_some(),
        member: state.member
            && state
                .checked
                .is_some_and(|at| at.elapsed() < HEARTBEAT_GRACE),
        account: state.account.clone(),
        device_suffix: device_id().ok().map(|id| id[id.len() - 8..].to_string()),
    })
}
async fn status() -> Json<JsonResponse> {
    success(local_status())
}
async fn register(Json(input): Json<Credentials>) -> ApiResult {
    let data = cloud("register", json!({"account":input.account,"password":input.password,"device_id":device_id().map_err(|msg| error(StatusCode::SERVICE_UNAVAILABLE,msg))?}), None).await?;
    Ok(success(data))
}
async fn login(Json(input): Json<Credentials>) -> ApiResult {
    let data = cloud("login", json!({"account":input.account,"password":input.password,"device_id":device_id().map_err(|msg| error(StatusCode::SERVICE_UNAVAILABLE,msg))?}), None).await?;
    let token = data
        .get("session")
        .and_then(Value::as_str)
        .filter(|s| s.len() >= 32)
        .ok_or_else(|| error(StatusCode::BAD_GATEWAY, "登录响应缺少会话"))?;
    let member = data.get("member").and_then(Value::as_bool).unwrap_or(false);
    {
        let mut state = session().write().expect("membership lock poisoned");
        state.token = Some(token.to_string());
        state.account = Some(input.account);
        state.member = member;
        state.checked = Some(Instant::now());
    }
    Ok(success(local_status()))
}
async fn redeem(Json(input): Json<Card>) -> ApiResult {
    let token = session()
        .read()
        .expect("membership lock poisoned")
        .token
        .clone()
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "请先登录"))?;
    cloud("redeem", json!({"card":input.card}), Some(&token)).await?;
    let data = cloud("heartbeat", json!({}), Some(&token)).await?;
    let member = data.get("member").and_then(Value::as_bool).unwrap_or(false);
    let mut state = session().write().expect("membership lock poisoned");
    if state.token.as_deref() == Some(&token) {
        state.member = member;
        state.checked = Some(Instant::now());
    }
    drop(state);
    Ok(success(local_status()))
}
async fn logout() -> Json<JsonResponse> {
    let token = {
        let mut state = session().write().expect("membership lock poisoned");
        state.account = None;
        state.member = false;
        state.checked = None;
        state.token.take()
    };
    if let Some(token) = token {
        let _ = cloud("logout", json!({}), Some(&token)).await;
    }
    success(local_status())
}
pub async fn refresh() {
    let token = session()
        .read()
        .expect("membership lock poisoned")
        .token
        .clone();
    let Some(token) = token else { return };
    let result = cloud("heartbeat", json!({}), Some(&token)).await;
    let mut state = session().write().expect("membership lock poisoned");
    if state.token.as_deref() != Some(&token) {
        return;
    }
    match result {
        Ok(data) => {
            state.member = data.get("member").and_then(Value::as_bool).unwrap_or(false);
            state.checked = Some(Instant::now());
        }
        Err(_) => {
            state.member = false;
            state.checked = None;
        }
    }
}
pub async fn heartbeat_loop(shutdown_tx: UnboundedSender<ControllerCommand>) {
    let mut last_check = Instant::now();
    let mut shutdown_sent = HashSet::new();
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if last_check.elapsed() >= HEARTBEAT_INTERVAL {
            refresh().await;
            last_check = Instant::now();
        }
        if is_member() {
            shutdown_sent.clear();
            continue;
        }
        for device in ControlledDevice::get_device_list().await {
            if device.main && shutdown_sent.insert(device.scid.clone()) {
                let _ = shutdown_tx.send(ControllerCommand::ShutdownMain(device.scid));
            }
        }
    }
}
pub async fn shutdown_logout() {
    let _ = logout().await;
}
pub fn router() -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/redeem", post(redeem))
        .route("/logout", post(logout))
}
pub async fn require_active(request: Request<Body>, next: Next) -> Response {
    let path = request.uri().path();
    if path.starts_with("/api/")
        && !path.starts_with("/api/member/")
        && path != "/api/config/get_config"
        && !is_member()
    {
        return error(StatusCode::FORBIDDEN, "会员未开通或验证已失效").into_response();
    }
    next.run(request).await
}
