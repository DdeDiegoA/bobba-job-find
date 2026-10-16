//! The loopback redirect listener (RFC 8252 §7.3, AC2).
//!
//! It binds `127.0.0.1` on an **ephemeral** port (never a fixed one, and never
//! `0.0.0.0`), accepts exactly one callback, answers it, and goes away. The
//! `state` check happens in [`super::PendingFlow::take_callback`]; this module
//! only reads the wire.
//!
//! Requests for any other path (a browser's `/favicon.ico`, a port scan) are
//! answered `404` and do **not** count as the callback, so they cannot consume
//! the one chance the flow gets.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use super::{CallbackQuery, OAuthError};

/// The single path the provider is redirected to.
pub const CALLBACK_PATH: &str = "/cb";

/// Longest request line and header block accepted from the callback connection.
const MAX_REQUEST_BYTES: usize = 8 * 1024;

/// How long one connection may take to say something. A browser on loopback
/// answers in well under this; a connection that stays silent (a stray probe)
/// costs the flow at most this long before the next one is accepted.
const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// The browser page shown after the redirect. It is served before any app UI can
/// render and outside the running app's i18n resources, so the one line is
/// written in both shipped languages (AD-17 EN/ES) rather than in a catalog.
const DONE_PAGE: &str = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<title>Boba Job Find</title></head><body style=\"font-family:system-ui;padding:2rem\">\
<h1>Boba Job Find</h1><p>Connected. You can close this window.</p>\
<p lang=\"es\">Conectado. Puedes cerrar esta ventana.</p></body></html>";

/// Bind the loopback redirect on an ephemeral port. Returns the listener and the
/// exact `redirect_uri` the provider must call back.
pub fn bind() -> Result<(TcpListener, String), OAuthError> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .map_err(|_| OAuthError::Listen)?;
    let port = listener
        .local_addr()
        .map_err(|_| OAuthError::Listen)?
        .port();
    Ok((listener, format!("http://127.0.0.1:{port}{CALLBACK_PATH}")))
}

/// Wait for the one callback, answer it, and close (AC2).
///
/// Bounded by `timeout`; the listener is polled because a blocking `accept`
/// cannot be given a deadline without an async runtime here.
pub fn accept_callback(
    listener: &TcpListener,
    timeout: Duration,
) -> Result<CallbackQuery, OAuthError> {
    listener
        .set_nonblocking(true)
        .map_err(|_| OAuthError::Listen)?;
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                if let Some(query) = serve(stream) {
                    return Ok(query);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(OAuthError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return Err(OAuthError::Listen),
        }
    }
}

/// Serve one connection: `/cb…` is the callback, anything else is a 404 that the
/// caller keeps waiting past. `None` means "not the callback".
fn serve(mut stream: TcpStream) -> Option<CallbackQuery> {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let target = read_request_target(&mut stream)?;
    let path = target.split_once('?').map(|(p, _)| p).unwrap_or(&target);

    if path != CALLBACK_PATH {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }

    let _ = stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{DONE_PAGE}",
            DONE_PAGE.len()
        )
        .as_bytes(),
    );
    Some(CallbackQuery::parse(&target))
}

/// The request target of the request line, e.g. `/cb?code=x&state=y`.
///
/// Reads the request line and drains the headers, so the browser sees the
/// response before the socket closes.
fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    let mut target = None;
    let mut read = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        read += line.len();
        if read > MAX_REQUEST_BYTES {
            return None;
        }
        if line == "\r\n" || line == "\n" {
            return target;
        }
        if target.is_none() {
            let mut parts = line.split_whitespace();
            let _method = parts.next()?;
            target = parts.next().map(str::to_string);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    const WAIT: Duration = Duration::from_secs(5);

    /// Drive one request against a listener, exactly as a browser would (origin
    /// form: `GET /cb?… HTTP/1.1`), and return the raw response.
    fn request(uri: &str) -> String {
        let rest = uri.trim_start_matches("http://");
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, "/".to_string()),
        };
        let mut stream = TcpStream::connect(authority).expect("connect");
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes())
            .expect("write");
        stream.set_read_timeout(Some(WAIT)).expect("timeout");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read");
        response
    }

    #[test]
    fn bind_uses_loopback_with_an_ephemeral_port() {
        let (first, uri) = bind().expect("bind");
        let (second, other) = bind().expect("bind");
        assert!(uri.starts_with("http://127.0.0.1:"), "{uri}");
        assert!(uri.ends_with(CALLBACK_PATH));
        assert!(!uri.contains(":0/"), "a real port, never the placeholder");
        assert_ne!(uri, other, "each flow gets its own port");
        drop((first, second));
    }

    #[test]
    fn one_callback_is_read_and_answered() {
        let (listener, uri) = bind().expect("bind");
        let caller = std::thread::spawn(move || request(&format!("{uri}?code=abc%2F1&state=s1")));

        let query = accept_callback(&listener, WAIT).expect("callback");
        assert_eq!(query.code.as_deref(), Some("abc/1"));
        assert_eq!(query.state.as_deref(), Some("s1"));

        let response = caller.join().expect("join");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains("close this window"), "{response}");
    }

    #[test]
    fn a_stray_request_does_not_consume_the_callback() {
        let (listener, uri) = bind().expect("bind");
        let stray_uri = format!(
            "http://{}/favicon.ico",
            uri.trim_start_matches("http://").split('/').next().unwrap()
        );
        let callback_uri = format!("{uri}?code=real&state=s1");

        // A browser asking for the icon first is answered 404 while the flow
        // keeps waiting for the real redirect (AC2).
        let caller = std::thread::spawn(move || {
            let stray = request(&stray_uri);
            let real = request(&callback_uri);
            (stray, real)
        });

        let query = accept_callback(&listener, WAIT).expect("callback");
        assert_eq!(query.code.as_deref(), Some("real"));

        let (stray, real) = caller.join().expect("join");
        assert!(stray.starts_with("HTTP/1.1 404"), "{stray}");
        assert!(real.starts_with("HTTP/1.1 200 OK"), "{real}");
    }

    #[test]
    fn waiting_for_a_callback_times_out() {
        let (listener, _uri) = bind().expect("bind");
        let error = accept_callback(&listener, Duration::from_millis(50)).expect_err("timeout");
        assert_eq!(error, OAuthError::Timeout);
    }
}
