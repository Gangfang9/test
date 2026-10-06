use std::{net::SocketAddrV4, sync::Arc, thread, time::Duration};

use bevy::log;
use copypasta::{ClipboardContext, ClipboardProvider};
use rust_i18n::t;
use tokio::{
    net::TcpListener,
    sync::{
        broadcast,
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        oneshot,
    },
};

use crate::{
    config::LocalConfig,
    mask::mask_command::MaskCommand,
    scrcpy::control_msg::{ScrcpyControlMsg, ScrcpyDeviceMsg},
    utils::{LatestVideoFrame, mask_win_move_helper},
    web::ws::WebSocketNotification,
};

#[derive(Debug)]
pub enum ControllerCommand {
    StartProjection {
        session: Arc<super::session::ProjectionSession>,
        listener: TcpListener,
        args: Vec<String>,
        audio: bool,
    },
    ShutdownMain(String),
    ShutdownSub(String),
}

pub struct Controller;

impl Controller {
    pub fn start(
        addr: SocketAddrV4,
        cs_tx: broadcast::Sender<ScrcpyControlMsg>,
        v_tx: LatestVideoFrame,
        d_tx: UnboundedSender<ControllerCommand>,
        d_rx: UnboundedReceiver<ControllerCommand>,
        m_tx: crossbeam_channel::Sender<(MaskCommand, oneshot::Sender<Result<String, String>>)>,
        ws_tx: broadcast::Sender<WebSocketNotification>,
    ) {
        thread::spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    Controller::run_server(addr, cs_tx, v_tx, d_tx, d_rx, m_tx, ws_tx).await;
                });
        });
    }

    async fn cr_msg_handler(
        mut cr_rx: UnboundedReceiver<ScrcpyDeviceMsg>,
        m_tx: crossbeam_channel::Sender<(MaskCommand, oneshot::Sender<Result<String, String>>)>,
        ws_tx: broadcast::Sender<WebSocketNotification>,
    ) {
        loop {
            match cr_rx.recv().await {
                Some(msg) => match msg {
                    ScrcpyDeviceMsg::Clipboard { length: _, text } => {
                        if LocalConfig::get_clipboard_sync() {
                            let Ok(mut ctx) = ClipboardContext::new() else {
                                continue;
                            };
                            match ctx.set_contents(text) {
                                Ok(()) => log::info!(
                                    "[Controller] {}",
                                    t!("scrcpy.syncClipboardFromMain")
                                ),
                                Err(e) => log::info!(
                                    "[Controller] {}: {}",
                                    t!("scrcpy.syncClipboardFromMain"),
                                    e
                                ),
                            }
                        }
                    }
                    ScrcpyDeviceMsg::AckClipboard { .. } => {}
                    ScrcpyDeviceMsg::UhidOutput { .. } => {}
                    ScrcpyDeviceMsg::Rotation {
                        rotation,
                        width,
                        height,
                        scid,
                    } => {
                        if !super::session::ProjectionSession::current()
                            .is_some_and(|s| s.scid == scid)
                        {
                            continue;
                        }
                        ws_tx
                            .send(WebSocketNotification::ScrcpyDeviceRotation {
                                rotation,
                                width,
                                height,
                                scid: scid.clone(),
                            })
                            .ok();
                        let msg = mask_win_move_helper(width, height, &m_tx).await;
                        log::info!(
                            "[Controller] {}. {}",
                            t!(
                                "scrcpy.deviceRotation",
                                scid => scid,
                                degree => rotation * 90,
                            ),
                            msg
                        );
                    }
                    ScrcpyDeviceMsg::Unknown => {
                        log::warn!("[Controller] {}", t!("scrcpy.unknownControlMsg"))
                    }
                },
                None => {
                    log::info!("[Controller] {}", t!("scrcpy.crChannelClosed"));
                    break;
                }
            }
        }
    }

    async fn run_server(
        addr: SocketAddrV4,
        cs_tx: broadcast::Sender<ScrcpyControlMsg>,
        v_tx: LatestVideoFrame,
        d_tx: UnboundedSender<ControllerCommand>,
        mut d_rx: UnboundedReceiver<ControllerCommand>,
        m_tx: crossbeam_channel::Sender<(MaskCommand, oneshot::Sender<Result<String, String>>)>,
        ws_tx: broadcast::Sender<WebSocketNotification>,
    ) {
        log::info!(
            "[Projection] controller ready; per-session listeners (configured {})",
            addr
        );
        let (cr_tx, cr_rx) = mpsc::unbounded_channel();
        tokio::spawn(Self::cr_msg_handler(cr_rx, m_tx.clone(), ws_tx.clone()));
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if !crate::membership::is_member() {
                        if let Some(session) = super::session::ProjectionSession::current() { session.stop(); }
                    }
                }
                command = d_rx.recv() => match command {
                    Some(ControllerCommand::StartProjection { session, listener, args, audio }) => {
                        let video = v_tx.begin_session();
                        let context = super::projection::Context { cs: cs_tx.clone(), cr: cr_tx.clone(), m: m_tx.clone(), ws: ws_tx.clone(), commands: d_tx.clone() };
                        tokio::spawn(super::projection::run(session, listener, args, audio, video, context));
                    }
                    Some(ControllerCommand::ShutdownMain(scid) | ControllerCommand::ShutdownSub(scid)) => {
                        if let Some(session) = super::session::ProjectionSession::current().filter(|s| s.scid == scid) { session.stop(); }
                    }
                    None => { if let Some(session) = super::session::ProjectionSession::current() { session.stop(); } break; }
                }
            }
        }
    }
}
