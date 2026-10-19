//! Static file server for the browser viewer (`cargo xtask serve-viewer`).
//!
//! The session's viewer stage runs this binary with `serve-viewer`, so the
//! launcher's own process supervision, logging and readiness probe apply to
//! it like to any other stage, and the run path needs no interpreter.
//!
//! The server exits when the process that launched it goes away: a stage
//! lives in its own process group, so neither a terminal ctrl-c nor a
//! crashed launcher reaches it, and an unowned server would keep the port
//! and serve a tree nobody is watching.

use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::error::XtaskError;
use crate::output::print_line;

mod http;

/// How long a connection may take to send its request head. A browser
/// sends it at once; a stalled peer must not pin a thread forever.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the server checks that its launcher is still alive.
const PARENT_CHECK_INTERVAL: Duration = Duration::from_millis(250);

/// Options for `cargo xtask serve-viewer`.
#[derive(Debug, PartialEq, Eq)]
pub struct ServeViewerArgs {
    /// The directory served as `/` (`--dir`).
    pub dir: PathBuf,
    /// The TCP port to listen on (`--port`; 0 picks a free one).
    pub port: u16,
    /// The address to bind (`--bind`, default loopback).
    pub bind: IpAddr,
}

/// Parses `serve-viewer` arguments (after the subcommand name).
///
/// # Errors
///
/// Returns [`XtaskError::Usage`] for unknown flags, missing values, or a
/// malformed port or address.
pub fn parse_args(args: &[String]) -> Result<ServeViewerArgs, XtaskError> {
    let mut dir = None;
    let mut port = None;
    let mut bind = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let flag = arg.as_str();
        let value = match flag {
            "--dir" | "--port" | "--bind" => iter.next().ok_or_else(|| XtaskError::Usage {
                message: format!("{flag} requires a value"),
            })?,
            other => {
                return Err(XtaskError::Usage {
                    message: format!(
                        "unknown serve-viewer argument {other:?} (expected --dir, --port, --bind)"
                    ),
                });
            }
        };
        match flag {
            "--dir" => dir = Some(PathBuf::from(value)),
            "--port" => {
                port = Some(value.parse().map_err(|_| XtaskError::Usage {
                    message: format!("--port expects a port number, got {value:?}"),
                })?);
            }
            _ => {
                bind = value.parse().map_err(|_| XtaskError::Usage {
                    message: format!("--bind expects an IP address, got {value:?}"),
                })?;
            }
        }
    }
    match (dir, port) {
        (Some(dir), Some(port)) => Ok(ServeViewerArgs { dir, port, bind }),
        _ => Err(XtaskError::Usage {
            message: "serve-viewer requires --dir <dir> and --port <port>".to_owned(),
        }),
    }
}

/// Serves `args.dir` until the launching process exits.
///
/// Prints `LISTENING <port>` once the socket is bound, so a caller that
/// asked for port 0 learns the port it got.
///
/// # Errors
///
/// Returns [`XtaskError::Io`] when the directory cannot be resolved, the
/// socket cannot be bound, or the accept thread cannot start or stops.
pub fn run_serve_viewer(args: &ServeViewerArgs) -> Result<(), XtaskError> {
    let root = args.dir.canonicalize().map_err(|source| XtaskError::Io {
        context: "resolving the viewer directory",
        source,
    })?;
    let listener = TcpListener::bind((args.bind, args.port)).map_err(|source| XtaskError::Io {
        context: "binding the viewer port",
        source,
    })?;
    let port = listener
        .local_addr()
        .map_err(|source| XtaskError::Io {
            context: "reading the viewer's bound address",
            source,
        })?
        .port();
    let parent = std::os::unix::process::parent_id();
    let root = Arc::new(root);
    let accept_root = Arc::clone(&root);
    let accepting = std::thread::Builder::new()
        .name("viewer-accept".to_owned())
        .spawn(move || serve(&listener, &accept_root))
        .map_err(|source| XtaskError::Io {
            context: "starting the viewer accept thread",
            source,
        })?;
    print_line(&format!("LISTENING {port}"));
    tracing::info!(root = %root.display(), port, bind = %args.bind, "serving the viewer");
    // Returning from `main` ends every connection thread with it.
    while std::os::unix::process::parent_id() == parent {
        if accepting.is_finished() {
            return Err(XtaskError::Io {
                context: "serving the viewer",
                source: std::io::Error::other("the accept loop stopped"),
            });
        }
        std::thread::sleep(PARENT_CHECK_INTERVAL);
    }
    tracing::info!(parent, "the launching process exited; stopping the viewer");
    Ok(())
}

/// Accepts connections forever, serving each on its own thread.
pub(crate) fn serve(listener: &TcpListener, root: &Arc<PathBuf>) {
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!(%error, "accepting a viewer connection failed");
                continue;
            }
        };
        let root = Arc::clone(root);
        let spawned = std::thread::Builder::new()
            .name("viewer-conn".to_owned())
            .spawn(move || handle_connection(stream, &root));
        if let Err(error) = spawned {
            tracing::warn!(%error, "starting a viewer connection thread failed");
        }
    }
}

fn handle_connection(mut stream: TcpStream, root: &Path) {
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok();
    // The readiness probe connects and closes without a request; that is
    // not an error worth logging.
    let Some(request_line) = http::read_request_line(&mut stream) else {
        return;
    };
    let response = http::respond(&request_line, root);
    tracing::debug!(request = %request_line, status = response.status, "viewer request");
    if let Err(error) = response.write_to(&mut stream) {
        tracing::debug!(%error, "the viewer client went away mid-response");
    }
}

#[cfg(test)]
mod tests;
