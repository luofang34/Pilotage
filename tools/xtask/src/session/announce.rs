//! The ready banner and the served connect facts.

use crate::backend::SessionContext;
use crate::cli::SimArgs;
use crate::output::print_line;
use crate::readiness::{session_manifest, viewer_url};

use super::open_in_browser;

/// Resolves the session's reachable address, writes the manifest, and
/// prints the banner — one call so the three artifacts can never carry
/// different facts.
pub(super) fn publish_connect_facts(
    args: &SimArgs,
    ctx: &SessionContext,
    actual_port: u16,
    certificate: &str,
) {
    let session_host = if ctx.lan {
        lan_address().unwrap_or_else(|| "127.0.0.1".to_owned())
    } else {
        "127.0.0.1".to_owned()
    };
    write_session_manifest(ctx, &session_host, actual_port, certificate);
    announce_ready(args, &session_host, actual_port, certificate);
}

/// Prints the ready URL and opens it in the default browser when asked.
/// Under `--lan` the native connect facts follow: the same three values a
/// browser takes from the query string, for a client that takes them from
/// a settings screen instead.
fn announce_ready(args: &SimArgs, session_host: &str, actual_port: u16, certificate: &str) {
    let url = viewer_url(args.viewer_port, actual_port, certificate);
    print_line("");
    print_line(&format!("session ready: {url}"));
    if args.lan {
        print_line(&format!(
            "native clients: https://{session_host}:{actual_port}/pilotage"
        ));
        print_line(&format!("  certificate sha-256: {certificate}"));
        print_line(&format!(
            "  connect manifest: http://{session_host}:{}/session.json",
            args.viewer_port
        ));
    }
    print_line("press ctrl-c to stop the session");
    if args.open {
        open_in_browser(&url);
    }
}

/// The address another device on this network reaches this machine at:
/// the local address of a routed UDP socket. No datagram is sent; a
/// machine with no route reports nothing and the caller falls back to
/// loopback rather than guessing among interfaces.
fn lan_address() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Writes the served session manifest (`clients/web/session.json`): any
/// viewer tab — including one whose URL pins an older session's
/// certificate — re-reads it after a failed connect and converges on
/// THIS session. Best-effort: a failed write only loses stale-tab
/// convergence, never the session.
fn write_session_manifest(ctx: &SessionContext, session_host: &str, port: u16, certificate: &str) {
    let path = ctx.repo_root.join("clients/web/session.json");
    if let Err(error) = std::fs::write(&path, session_manifest(session_host, port, certificate)) {
        print_line(&format!(
            "warning: could not write {}: {error}",
            path.display()
        ));
    }
}
