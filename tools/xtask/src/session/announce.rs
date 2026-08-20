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
) -> Option<MdnsAlias> {
    let lan_ip = if ctx.lan { lan_address() } else { None };
    // The mDNS name is the LAN identity that SURVIVES: DHCP renumbers
    // the address between sessions, and every stale URL then reads as
    // a dead host. The name is pure discovery — the certificate is
    // pinned by digest, never by name, so what resolves the host can
    // change freely without touching the trust anchor.
    let mdns = if ctx.lan { mdns_hostname() } else { None };
    // The DEDICATED alias outranks the machine name: `pilotage.local`
    // is the same on every machine that runs a session, so a client's
    // saved manifest URL is device-independent. Best-effort covers a
    // machine without dns-sd; a NAME CONFLICT on the network surfaces
    // asynchronously after spawn and is not detected here — the
    // machine-name URL printed beside the alias is the recovery.
    let alias = lan_ip
        .as_deref()
        .and_then(|ip| register_mdns_alias(ip, args.viewer_port));
    let session_host = alias
        .as_ref()
        .map(|_| MDNS_ALIAS_HOST.to_owned())
        .or(mdns)
        .or_else(|| lan_ip.clone())
        .unwrap_or_else(|| "127.0.0.1".to_owned());
    write_session_manifest(
        ctx,
        &session_host,
        lan_ip.as_deref(),
        actual_port,
        certificate,
    );
    announce_ready(
        args,
        &session_host,
        lan_ip.as_deref(),
        actual_port,
        certificate,
    );
    alias
}

/// The device-independent LAN name a session answers to while it runs.
const MDNS_ALIAS_HOST: &str = "pilotage.local";

/// A best-effort mDNS host alias held for the session's lifetime: the
/// spawned `dns-sd -P` proxy keeps the registration alive, and dropping
/// this kills it, so the name never outlives the session it names.
pub(super) struct MdnsAlias {
    child: std::process::Child,
}

impl Drop for MdnsAlias {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

/// Registers `pilotage.local` as a proxy for this machine's address.
fn register_mdns_alias(ip: &str, viewer_port: u16) -> Option<MdnsAlias> {
    let child = std::process::Command::new("dns-sd")
        .args([
            "-P",
            "Pilotage",
            "_pilotage._tcp",
            "local",
            &viewer_port.to_string(),
            MDNS_ALIAS_HOST,
            ip,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    Some(MdnsAlias { child })
}

/// This machine's mDNS name (`<name>.local`), the LAN identity Bonjour
/// already publishes. On macOS that identity is the LOCAL host name
/// (`scutil --get LocalHostName`), which can differ from the kernel
/// hostname — mDNS answers only for the advertised one, so publishing
/// the kernel name would hand out a URL no other device can resolve.
/// `None` when no usable name exists; the caller falls back to the
/// numeric address.
fn mdns_hostname() -> Option<String> {
    let advertised = std::process::Command::new("scutil")
        .args(["--get", "LocalHostName"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok());
    let fallback = || {
        std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
    };
    let name = advertised.or_else(fallback)?.trim().to_owned();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    Some(if name.ends_with(".local") {
        name
    } else {
        format!("{name}.local")
    })
}

/// Prints the ready URL and opens it in the default browser when asked.
/// Under `--lan` the native connect facts follow: the same three values a
/// browser takes from the query string, for a client that takes them from
/// a settings screen instead.
fn announce_ready(
    args: &SimArgs,
    session_host: &str,
    lan_ip: Option<&str>,
    actual_port: u16,
    certificate: &str,
) {
    let url = viewer_url("127.0.0.1", args.viewer_port, actual_port, certificate);
    print_line("");
    print_line(&format!("session ready: {url}"));
    if args.lan {
        // The same session from another device on this network — an
        // iPad's browser takes the whole story from the URL, exactly
        // like the local one. The mDNS name leads because it survives
        // DHCP renumbering; the numeric address follows for a client
        // that cannot resolve it.
        print_line(&format!(
            "on the LAN:    {}",
            viewer_url(session_host, args.viewer_port, actual_port, certificate)
        ));
        if let Some(ip) = lan_ip.filter(|ip| *ip != session_host) {
            print_line(&format!(
                "  by address:  {}",
                viewer_url(ip, args.viewer_port, actual_port, certificate)
            ));
        }
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
fn write_session_manifest(
    ctx: &SessionContext,
    session_host: &str,
    lan_ip: Option<&str>,
    port: u16,
    certificate: &str,
) {
    let path = ctx.repo_root.join("clients/web/session.json");
    if let Err(error) = std::fs::write(
        &path,
        session_manifest(session_host, lan_ip, port, certificate),
    ) {
        print_line(&format!(
            "warning: could not write {}: {error}",
            path.display()
        ));
    }
}
