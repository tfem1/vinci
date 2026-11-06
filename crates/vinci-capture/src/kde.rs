//! Direct screen capture on KDE Plasma through KWin's `zkde_screencast_unstable_v1`
//! Wayland protocol — no xdg-desktop-portal dialog and no "screen is being shared"
//! indicator, like Spectacle or KDE's remote desktop.
//!
//! KWin only advertises this restricted interface to a process whose executable is the
//! `Exec=` of a `.desktop` file declaring `X-KDE-Wayland-Interfaces=zkde_screencast_unstable_v1`
//! (VINCI's installers write that entry). Elsewhere, or when KWin does not grant it,
//! [`KdeStream::start`] returns `Ok(None)` and the caller uses the portal instead.
//!
//! The result is a PipeWire node id on the user's PipeWire daemon; `pipewiresrc
//! path=<node>` reads it like a portal stream (no fd needed).

use crate::CaptureError;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, BindError, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};

#[allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    clippy::all
)]
pub mod proto {
    use wayland_client;
    use wayland_client::protocol::*;
    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/zkde-screencast-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocols/zkde-screencast-unstable-v1.xml");
}

use proto::zkde_screencast_stream_unstable_v1::{self as stream_v1, ZkdeScreencastStreamUnstableV1};
use proto::zkde_screencast_unstable_v1::ZkdeScreencastUnstableV1;

/// The interface name KWin checks in `X-KDE-Wayland-Interfaces`.
pub const INTERFACE: &str = "zkde_screencast_unstable_v1";

#[derive(Debug, Clone)]
struct OutputInfo {
    output: wl_output::WlOutput,
    name: Option<String>,
    size: Option<(i32, i32)>,
}

#[derive(Default)]
struct State {
    outputs: Vec<OutputInfo>,
    node: Option<u32>,
    failed: Option<String>,
    closed: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(state: &mut Self, proxy: &wl_output::WlOutput, event: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let Some(o) = state.outputs.iter_mut().find(|o| &o.output == proxy) else { return };
        match event {
            wl_output::Event::Mode { flags: WEnum::Value(f), width, height, .. } if f.contains(wl_output::Mode::Current) => {
                o.size = Some((width, height));
            }
            wl_output::Event::Name { name } => o.name = Some(name),
            _ => {}
        }
    }
}

impl Dispatch<ZkdeScreencastUnstableV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZkdeScreencastUnstableV1,
        _: <ZkdeScreencastUnstableV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZkdeScreencastStreamUnstableV1, ()> for State {
    fn event(state: &mut Self, _: &ZkdeScreencastStreamUnstableV1, event: stream_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            stream_v1::Event::Created { node } => state.node = Some(node),
            stream_v1::Event::Failed { error } => state.failed = Some(error),
            stream_v1::Event::Closed => state.closed = true,
        }
    }
}

/// A running KWin screencast. Dropping it closes the stream.
pub struct KdeStream {
    pub node: u32,
    /// Output size in pixels (current mode).
    pub size: Option<(i32, i32)>,
    pub output_name: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

type Started = Result<Option<(u32, Option<(i32, i32)>, String)>, String>;

impl KdeStream {
    /// Ask KWin for a PipeWire stream of output `monitor` (0-based, in compositor
    /// order; out-of-range picks the first). `Ok(None)`: KWin does not offer the
    /// interface to this process (not Plasma, or no matching `.desktop` entry).
    pub fn start(monitor: u32, cursor: bool) -> Result<Option<Self>, CaptureError> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Ok(None);
        }
        let (tx, rx) = mpsc::channel::<Started>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = std::thread::Builder::new()
            .name("vinci-kwin-screencast".into())
            .spawn(move || run(monitor, cursor, tx, stop2))
            .map_err(|e| CaptureError::Portal(e.to_string()))?;
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(Some((node, size, output_name)))) => Ok(Some(Self { node, size, output_name, stop, thread: Some(thread) })),
            Ok(Ok(None)) => {
                let _ = thread.join();
                Ok(None)
            }
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(CaptureError::Portal(format!("KWin screencast: {e}")))
            }
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err(CaptureError::Portal("KWin screencast: no answer from the compositor".into()))
            }
        }
    }
}

impl Drop for KdeStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Is the interface offered to this process? (No stream is created.)
pub fn available() -> bool {
    let Ok(conn) = Connection::connect_to_env() else { return false };
    let Ok((globals, _queue)) = registry_queue_init::<State>(&conn) else { return false };
    globals.contents().with_list(|l| l.iter().any(|g| g.interface == INTERFACE))
}

fn run(monitor: u32, cursor: bool, tx: mpsc::Sender<Started>, stop: Arc<AtomicBool>) {
    let mut state = State::default();
    let setup = (|| -> Result<Option<(EventQueue<State>, ZkdeScreencastStreamUnstableV1, Connection)>, String> {
        let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let screencast: ZkdeScreencastUnstableV1 = match globals.bind(&qh, 1..=3, ()) {
            Ok(s) => s,
            Err(BindError::NotPresent) => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        let outputs: Vec<(u32, u32)> =
            globals.contents().with_list(|l| l.iter().filter(|g| g.interface == "wl_output").map(|g| (g.name, g.version)).collect());
        for (name, version) in outputs {
            let output = globals.registry().bind::<wl_output::WlOutput, _, _>(name, version.min(4), &qh, ());
            state.outputs.push(OutputInfo { output, name: None, size: None });
        }
        queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
        let target = state.outputs.get(monitor as usize).or_else(|| state.outputs.first()).cloned().ok_or("KWin reported no output")?;
        let pointer = if cursor {
            proto::zkde_screencast_unstable_v1::Pointer::Embedded
        } else {
            proto::zkde_screencast_unstable_v1::Pointer::Hidden
        };
        let stream = screencast.stream_output(&target.output, pointer as u32, &qh, ());
        let deadline = Instant::now() + Duration::from_secs(8);
        while state.node.is_none() && state.failed.is_none() && !state.closed {
            if Instant::now() > deadline {
                return Err("KWin did not create the stream".into());
            }
            dispatch_for(&conn, &mut queue, &mut state, Duration::from_millis(200))?;
        }
        if let Some(e) = state.failed.take() {
            return Err(e);
        }
        if state.closed {
            return Err("KWin closed the stream".into());
        }
        let size = target.size;
        let name = target.name.unwrap_or_else(|| "output".into());
        let _ = tx.send(Ok(Some((state.node.unwrap_or_default(), size, name))));
        Ok(Some((queue, stream, conn)))
    })();
    match setup {
        Ok(Some((mut queue, stream, conn))) => {
            // Keep the Wayland connection (and so the stream) alive until stopped.
            while !stop.load(Ordering::SeqCst) && !state.closed {
                if dispatch_for(&conn, &mut queue, &mut state, Duration::from_millis(250)).is_err() {
                    break;
                }
            }
            stream.close();
            let _ = conn.flush();
        }
        Ok(None) => {
            let _ = tx.send(Ok(None));
        }
        Err(e) => {
            let _ = tx.send(Err(e));
        }
    }
}

/// Dispatch whatever arrives within `timeout`.
fn dispatch_for(conn: &Connection, queue: &mut EventQueue<State>, state: &mut State, timeout: Duration) -> Result<(), String> {
    queue.dispatch_pending(state).map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    if let Some(guard) = queue.prepare_read() {
        let mut fds = [libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        // SAFETY: one valid pollfd for the duration of the call.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout.as_millis() as i32) };
        if n > 0 {
            guard.read().map_err(|e| e.to_string())?;
        }
    }
    queue.dispatch_pending(state).map_err(|e| e.to_string())?;
    Ok(())
}
