//! xdg-desktop-portal ScreenCast session: monitor picker, restore token and the
//! PipeWire file descriptor handed to `pipewiresrc`.
//! Owned by the capture task.

use std::os::fd::OwnedFd;

use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use ashpd::desktop::{PersistMode, ResponseError, Session};

use crate::error::{AppError, Result};

/// A started ScreenCast session. Keep it (and therefore the portal session)
/// alive for as long as the pipeline reads from the PipeWire fd.
pub struct PortalStream {
    /// PipeWire node id of the selected monitor.
    pub node_id: u32,
    /// Token the portal issued so later sessions can skip the picker.
    pub restore_token: Option<String>,
    /// Human readable description of the selection, e.g. "Monitor 2560×1440".
    pub label: String,
    /// Reported stream size in compositor coordinates, if the portal sent it.
    pub size: Option<(i32, i32)>,
    fd: Option<OwnedFd>,
    session: Session<Screencast>,
    _proxy: Screencast,
}

impl PortalStream {
    /// Hand the PipeWire fd to the capture pipeline. It must stay open (and be
    /// passed as an integer property) for the lifetime of `pipewiresrc`.
    pub fn take_fd(&mut self) -> Result<OwnedFd> {
        self.fd
            .take()
            .ok_or_else(|| AppError::Portal("the PipeWire fd was already taken".into()))
    }

    /// Close the portal session (ends the share and any picker dialog).
    pub async fn close(mut self) {
        self.fd.take();
        if let Err(err) = self.session.close().await {
            log::debug!("could not close the ScreenCast session: {err}");
        }
    }
}

/// Run the full ScreenCast flow. With a `restore_token` the portal normally
/// skips the picker; without one it shows it. A cancelled (or dismissed)
/// picker comes back as a clean `AppError::Portal`.
pub async fn open(restore_token: Option<&str>) -> Result<PortalStream> {
    let proxy = Screencast::new().await.map_err(map_error)?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(map_error)?;

    let options = SelectSourcesOptions::default()
        .set_cursor_mode(CursorMode::EMBEDDED)
        .set_sources(SourceType::MONITOR)
        .set_multiple(false)
        .set_persist_mode(PersistMode::ExplicitlyRevoked)
        .set_restore_token(restore_token);

    proxy
        .select_sources(&session, options)
        .await
        .map_err(map_error)?
        .response()
        .map_err(map_error)?;

    let request = proxy
        .start(&session, None, StartCastOptions::default())
        .await
        .map_err(map_error)?;
    let streams = request.response().map_err(map_error)?;

    let stream = streams
        .streams()
        .iter()
        .find(|stream| {
            stream
                .source_type()
                .map(|kind| kind.contains(SourceType::MONITOR))
                .unwrap_or(true)
        })
        .or_else(|| streams.streams().first())
        .ok_or_else(|| AppError::Portal("the screen picker returned no stream".into()))?;

    let node_id = stream.pipe_wire_node_id();
    let size = stream.size().filter(|(w, h)| *w > 0 && *h > 0);
    let label = match size {
        Some((width, height)) => format!("Monitor {width}×{height}"),
        None => "Monitor".to_string(),
    };
    let restore_token = streams.restore_token().map(str::to_string);

    let fd = proxy
        .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
        .await
        .map_err(map_error)?;

    Ok(PortalStream {
        node_id,
        restore_token,
        label,
        size,
        fd: Some(fd),
        session,
        _proxy: proxy,
    })
}

const CANCELLED: &str = "screen selection was cancelled";

/// True when the user dismissed the picker rather than the portal breaking.
pub fn is_cancelled(err: &AppError) -> bool {
    matches!(err, AppError::Portal(message) if message == CANCELLED)
}

fn map_error(err: ashpd::Error) -> AppError {
    match err {
        ashpd::Error::Response(ResponseError::Cancelled)
        | ashpd::Error::Portal(ashpd::PortalError::Cancelled(_)) => {
            AppError::Portal(CANCELLED.into())
        }
        other => AppError::Portal(format!("screen capture portal failed: {other}")),
    }
}
