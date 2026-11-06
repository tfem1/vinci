//! Wayland screen capture through `xdg-desktop-portal` (ScreenCast) + PipeWire.
//!
//! The portal session must stay open for as long as we capture, so it lives on a
//! dedicated thread running a small single-threaded Tokio runtime. With
//! `PersistMode::ExplicitlyRevoked` and the restore token saved in VINCI's settings,
//! the compositor's "share your screen" dialog is shown only the first time.

use crate::CaptureError;
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::time::Duration;

pub struct PortalSession {
    pub fd: OwnedFd,
    pub node_id: u32,
    pub size: Option<(i32, i32)>,
    pub restore_token: Option<String>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

type StartResult = Result<(OwnedFd, u32, Option<(i32, i32)>, Option<String>), String>;

impl PortalSession {
    pub fn start(restore_token: Option<String>, cursor: bool) -> Result<Self, CaptureError> {
        let (tx, rx) = mpsc::channel::<StartResult>();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("vinci-portal".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
                    use ashpd::desktop::PersistMode;
                    let run = async {
                        let proxy = Screencast::new()
                            .await
                            .map_err(|e| format!("no screen-sharing service (xdg-desktop-portal) answered ({e})"))?;
                        let session = proxy.create_session().await.map_err(|e| e.to_string())?;
                        let cursor_mode = if cursor { CursorMode::Embedded } else { CursorMode::Hidden };
                        proxy
                            .select_sources(
                                &session,
                                cursor_mode,
                                SourceType::Monitor | SourceType::Window,
                                false,
                                restore_token.as_deref(),
                                PersistMode::ExplicitlyRevoked,
                            )
                            .await
                            .map_err(|e| e.to_string())?;
                        let response = proxy.start(&session, None).await.map_err(|e| e.to_string())?.response().map_err(|e| {
                            if e.to_string().contains("Cancelled") {
                                "screen sharing was cancelled by the user".to_string()
                            } else {
                                format!("the desktop's screen-sharing service failed ({e})")
                            }
                        })?;
                        let stream = response.streams().first().ok_or("the portal returned no stream")?.clone();
                        let token = response.restore_token().map(String::from);
                        let fd = proxy.open_pipe_wire_remote(&session).await.map_err(|e| e.to_string())?;
                        Ok::<_, String>((fd, stream.pipe_wire_node_id(), stream.size(), token, session))
                    };
                    match run.await {
                        Ok((fd, node, size, token, session)) => {
                            let _ = tx.send(Ok((fd, node, size, token)));
                            let _ = stop_rx.await;
                            let _ = session.close().await;
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                        }
                    }
                });
            })
            .map_err(|e| CaptureError::Portal(e.to_string()))?;
        // The user may need time to answer the dialog the first time.
        match rx.recv_timeout(Duration::from_secs(120)) {
            Ok(Ok((fd, node_id, size, token))) => {
                Ok(Self { fd, node_id, size, restore_token: token, stop: Some(stop_tx), thread: Some(thread) })
            }
            Ok(Err(e)) => Err(CaptureError::Portal(e)),
            Err(_) => Err(CaptureError::Portal("timed out waiting for the screen-sharing dialog".into())),
        }
    }
}

impl Drop for PortalSession {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
