//! Session coordinator; every exit joins workers before releasing the device.
use super::{
    connection::ScrcpyConnection,
    control_msg::{ScrcpyControlMsg, ScrcpyDeviceMsg},
    controller::ControllerCommand,
    managed_adb,
    media::VideoMsg,
    session::{self, ProjectionSession},
};
use crate::{
    mask::mask_command::MaskCommand,
    utils::{LatestVideoFrame, share::ControlledDevice},
    web::ws::WebSocketNotification,
};
use futures_util::FutureExt;
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc, oneshot},
    task::JoinSet,
};

pub struct Context {
    pub cs: broadcast::Sender<ScrcpyControlMsg>,
    pub cr: mpsc::UnboundedSender<ScrcpyDeviceMsg>,
    pub m: crossbeam_channel::Sender<(MaskCommand, oneshot::Sender<Result<String, String>>)>,
    pub ws: broadcast::Sender<WebSocketNotification>,
    pub commands: mpsc::UnboundedSender<ControllerCommand>,
}
async fn accept(listener: &TcpListener, session: &ProjectionSession) -> Result<TcpStream, String> {
    tokio::select! {
        _ = session.token.cancelled() => Err("投屏已取消".into()),
        result = tokio::time::timeout(Duration::from_secs(8), listener.accept()) => {
            result.map_err(|_| "投屏通道建立超时".to_string())?.map(|(socket, _)| socket).map_err(|e| e.to_string())
        }
    }
}
async fn window(context: &Context, scid: &str, connect: bool) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();
    context
        .m
        .send((
            MaskCommand::DeviceConnectionChange {
                connect,
                scid: scid.into(),
            },
            tx,
        ))
        .map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .map_err(|_| "投屏窗口响应超时".to_string())?
        .map_err(|e| e.to_string())?
        .map(|_| ())
}

pub async fn run(
    session: Arc<ProjectionSession>,
    listener: TcpListener,
    args: Vec<String>,
    audio: bool,
    video: LatestVideoFrame,
    context: Context,
) {
    let mut tasks = JoinSet::new();
    let process_session = session.clone();
    tasks.spawn(async move {
        let result = managed_adb::run(
            Some(&process_session.device_id),
            vec!["shell".into(), args.join(" ")],
            None,
            process_session.token.clone(),
        )
        .await;
        if !process_session.token.is_cancelled() {
            log::warn!(
                "[Projection {}] server ended: {:?}",
                process_session.scid,
                result
            );
        }
        process_session.token.cancel();
    });
    let outcome = stream(&session, &listener, audio, &video, &context, &mut tasks).await;
    drop(listener); // A new listener port isolates late connects from the next session.
    session.set_status("stopping", "正在释放投屏连接");
    session.token.cancel();
    // Hide promptly. The owner stays reserved until all workers have really exited.
    let _ = window(&context, &session.scid, false).await;
    while let Some(result) = tasks.join_next().await {
        if let Err(e) = result {
            log::warn!("[Projection {}] worker: {e}", session.scid);
        }
    }
    video.send(VideoMsg::Close);
    while crate::mask::companion::transport_active(&session.scid) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    ControlledDevice::remove_device(&session.scid).await;
    if let Err(e) = managed_adb::cleanup(&session.device_id, &session.scid).await {
        session::remember_cleanup(&session.device_id, &session.scid);
        log::warn!("[Projection {}] device cleanup deferred: {e}", session.scid);
    }
    let manual = session
        .manual_stop
        .load(std::sync::atomic::Ordering::Acquire);
    let message = if manual {
        String::new()
    } else {
        outcome.err().unwrap_or_else(|| "投屏连接已断开".into())
    };
    log::info!(
        "[Projection {}] stopped; {:?}; {}",
        session.scid,
        video.progress(),
        message
    );
    let retry = !manual && session.retries < 2 && crate::membership::is_member();
    let intent = session::intent();
    session.finish(message);
    let _ = context
        .ws
        .send(WebSocketNotification::ScrcpyDeviceConnection {
            scid: session.scid.clone(),
            main: true,
            connected: false,
        });
    if retry {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Err(e) = crate::web::device::resume_usb_device(
                &session.device_id,
                session.retries + 1,
                intent,
                &context.commands,
                &context.ws,
            )
            .await
            {
                log::warn!("[Projection] automatic recovery stopped: {e}");
            }
        });
    }
}

async fn stream(
    session: &Arc<ProjectionSession>,
    listener: &TcpListener,
    audio: bool,
    video: &LatestVideoFrame,
    context: &Context,
    tasks: &mut JoinSet<()>,
) -> Result<(), String> {
    let socket = accept(listener, session).await?;
    let (done_tx, done_rx) = oneshot::channel();
    let s = session.clone();
    let v = video.clone();
    thread::spawn(move || {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                tokio::select! { _ = s.token.cancelled() => {}, _ = ScrcpyConnection::new(socket).handle_video(s.token.clone(), v.clone(), true, &s.scid) => {} }
            });
        }));
        s.token.cancel();
        let _ = done_tx.send(());
    });
    tasks.spawn(async move {
        let _ = done_rx.await;
    });
    if audio {
        let socket = accept(listener, session).await?;
        let s = session.clone();
        let (tx, rx) = oneshot::channel();
        thread::spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
                    tokio::select! { _ = s.token.cancelled() => {}, _ = ScrcpyConnection::new(socket).handle_audio(s.token.clone(), false, &s.scid) => {} }
                });
            }));
            let _ = tx.send(());
        });
        tasks.spawn(async move {
            let _ = rx.await;
        });
    }
    let socket = accept(listener, session).await?;
    let s = session.clone();
    let m = context.m.clone();
    let cr = context.cr.clone();
    let cs_rx = context.cs.subscribe();
    tasks.spawn(async move {
        let future = ScrcpyConnection::new(socket).handle_control(cs_rx, cr, m, s.scid.clone(), true, s.token.clone(), false);
        tokio::select! { _ = s.token.cancelled() => {}, _ = std::panic::AssertUnwindSafe(future).catch_unwind() => {} }
        s.token.cancel();
    });
    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let mut shown = false;
    let mut refresh_at: Option<Instant> = None;
    let mut refreshed_frames = 0;
    let mut diagnostics_at = Instant::now();
    loop {
        tokio::select! { _ = session.token.cancelled() => return Err("投屏服务或连接已中断".into()), _ = tick.tick() => {} }
        let progress = video.progress();
        if !shown && progress.decoded > 0 {
            window(context, &session.scid, true).await?;
            session.mark_ready();
            shown = true;
            let _ = context
                .ws
                .send(WebSocketNotification::ScrcpyDeviceConnection {
                    scid: session.scid.clone(),
                    main: true,
                    connected: true,
                });
        }
        if !shown && start.elapsed() > Duration::from_secs(10) {
            return Err("未收到投屏首帧，请检查 USB 调试和设备编码器".into());
        }
        if diagnostics_at.elapsed() >= Duration::from_secs(10) {
            log::info!("[Projection {}] progress {:?}", session.scid, progress);
            diagnostics_at = Instant::now();
        }
        if let Some(requested) = refresh_at {
            if progress.decoded > refreshed_frames {
                refresh_at = None;
            } else if requested.elapsed() > Duration::from_secs(5) {
                return Err("视频刷新后仍未收到画面，正在恢复投屏".into());
            }
        } else if shown
            && progress
                .last_decode
                .is_some_and(|t| t.elapsed() > Duration::from_secs(30))
        {
            // A static screen is not a disconnect: ask the live encoder for a frame first.
            let state = managed_adb::run(
                Some(&session.device_id),
                vec!["get-state".into()],
                Some(Duration::from_secs(2)),
                session.token.clone(),
            )
            .await?;
            if state.trim() != "device" {
                return Err(format!("USB 调试设备状态为 {state}"));
            }
            context
                .cs
                .send(ScrcpyControlMsg::ResetVideo)
                .map_err(|e| e.to_string())?;
            refreshed_frames = progress.decoded;
            refresh_at = Some(Instant::now());
        }
        if progress
            .last_packet
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
            && progress
                .last_decode
                .is_some_and(|t| t.elapsed() > Duration::from_secs(8))
        {
            return Err("已收到视频数据但解码持续失败，正在恢复投屏".into());
        }
        if shown
            && progress
                .last_decode
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
            && progress
                .last_display
                .is_some_and(|t| t.elapsed() > Duration::from_secs(8))
        {
            return Err("视频显示未刷新，正在恢复投屏".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_accept_does_not_wait_for_missing_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let session = ProjectionSession::isolated();
        session.token.cancel();
        let result = tokio::time::timeout(Duration::from_millis(100), accept(&listener, &session))
            .await
            .unwrap();
        assert!(result.is_err());
    }
}
