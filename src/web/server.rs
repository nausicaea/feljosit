//! The HTTP server (HTTP/1.1 with the standard library).
//!
//! The application is for one person on the same computer (ADR-0002). Thus a small
//! server is sufficient: one thread for each connection, one request for each
//! connection and a limit for the number of connections. The server reads the complete
//! request with size limits and time limits. Then [`App::handle`] makes the response.
//! See ADR-0021.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::{App, Request, Response};
use crate::db::log;
use crate::shutdown::{Event, Stop};

/// The maximum number of connections at the same time.
const MAX_CONNECTIONS: usize = 64;
/// The maximum size of the request line and the headers together.
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// The maximum number of header lines.
const MAX_HEADERS: usize = 100;
/// The time limit for one read or write operation on the connection.
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// The maximum time to wait for the active connections after a stop signal.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// Serve the application on the listener. The function returns after a stop signal
/// (`SIGTERM` or `SIGINT` on Unix, see [`crate::shutdown`]) or if the listener has an
/// error. On other systems, Ctrl+C stops the process: SQLite keeps the database consistent.
pub fn serve(app: Arc<App>, listener: TcpListener) -> io::Result<()> {
    let stop = Stop::install()?;
    stop.prepare(&listener)?;
    let active = Arc::new(AtomicUsize::new(0));
    loop {
        if stop.wait(&listener)? == Event::Stop {
            break;
        }
        let stream =
            match listener.accept().and_then(|(stream, _)| stream.set_nonblocking(false).map(|()| stream)) {
                Ok(stream) => stream,
                // The client closed the connection before the accept.
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(error) => {
                    // For example "too many open files". Continue with the next connection.
                    log::warning(&format!("Cannot accept a connection: {error}"));
                    continue;
                }
            };
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            active.fetch_sub(1, Ordering::SeqCst);
            let _ = write_response(&stream, &Response::text(503, "Too many connections"), false);
            continue;
        }
        let app = app.clone();
        let guard = ActiveGuard(active.clone());
        let spawned = std::thread::Builder::new().name("feljosit-connection".into()).spawn(move || {
            let _guard = guard;
            handle_connection(&app, stream);
        });
        if let Err(error) = spawned {
            log::warning(&format!("Cannot start a thread for a connection: {error}"));
        }
    }
    log::info("The server stops.");
    drop(listener);
    let deadline = Instant::now() + STOP_TIMEOUT;
    while active.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let remaining = active.load(Ordering::SeqCst);
    if remaining > 0 {
        log::warning(&format!("{remaining} connections did not complete in time."));
    }
    Ok(())
}

/// Decrease the number of active connections at the end of a connection.
struct ActiveGuard(Arc<AtomicUsize>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// An error in the request. The server sends the status and closes the connection.
struct BadRequest(u16, &'static str);

fn handle_connection(app: &App, stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_nodelay(true);
    let peer_is_loopback = stream.peer_addr().is_ok_and(|a| a.ip().is_loopback());
    let peer = stream.peer_addr().map_or_else(|_| "-".to_string(), |a| a.ip().to_string());
    let mut reader = BufReader::new(&stream);
    let (req, response) = match read_request(app, &mut reader, &stream, peer_is_loopback) {
        Ok(Some(req)) => {
            let response = run(app, &req);
            (req, response)
        }
        // The client closed the connection before it sent a request.
        Ok(None) => return,
        Err(BadRequest(status, message)) => {
            let _ = write_response(&stream, &Response::text(status, message), false);
            return;
        }
    };
    log::info(&format!("{peer} \"{} {}\" {}", req.method, req.path, response.status));
    let _ = write_response(&stream, &response, req.method == "HEAD");
}

/// Run the application. A panic in a handler gives the status 500. The other
/// connections continue.
fn run(app: &App, req: &Request) -> Response {
    match catch_unwind(AssertUnwindSafe(|| app.handle(req))) {
        Ok(response) => response,
        Err(_) => {
            log::warning("A request handler stopped.");
            app.finish(req, Response::text(500, "Internal Server Error"))
        }
    }
}

/// Read one line of the head. The line end (CRLF or LF) is not in the result.
fn read_line(reader: &mut impl BufRead, budget: &mut usize) -> Result<Option<String>, BadRequest> {
    let mut line = Vec::new();
    let limit = u64::try_from(*budget).unwrap_or(u64::MAX).saturating_add(1);
    let count = reader
        .by_ref()
        .take(limit)
        .read_until(b'\n', &mut line)
        .map_err(|_| BadRequest(400, "Bad Request"))?;
    if count == 0 {
        return Ok(None);
    }
    if count > *budget {
        return Err(BadRequest(431, "Request Header Fields Too Large"));
    }
    *budget -= count;
    if line.pop() != Some(b'\n') {
        return Err(BadRequest(400, "Bad Request"));
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line).map(Some).map_err(|_| BadRequest(400, "Bad Request"))
}

/// Read a complete request. `Ok(None)` means that the client sent nothing.
fn read_request(
    app: &App,
    reader: &mut impl BufRead,
    stream: &TcpStream,
    peer_is_loopback: bool,
) -> Result<Option<Request>, BadRequest> {
    let bad = || BadRequest(400, "Bad Request");
    let mut budget = MAX_HEAD_BYTES;
    let Some(request_line) = read_line(reader, &mut budget)? else {
        return Ok(None);
    };
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") {
        return Err(BadRequest(505, "HTTP Version Not Supported"));
    }
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(bad());
    }
    // The origin form ("/path?query"). A proxy can send the absolute form.
    let target = match target.find("://") {
        Some(pos)
            if target[..pos].eq_ignore_ascii_case("http") || target[..pos].eq_ignore_ascii_case("https") =>
        {
            let rest = &target[pos + 3..];
            rest.find('/').map_or("/", |slash| &rest[slash..])
        }
        _ => target,
    };
    if !target.starts_with('/') || target.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(bad());
    }
    let target = target.split('#').next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let line = read_line(reader, &mut budget)?.ok_or_else(bad)?;
        if line.is_empty() {
            break;
        }
        // Folded header lines are not permitted (RFC 9112, section 5.2).
        if line.starts_with([' ', '\t']) || headers.len() >= MAX_HEADERS {
            return Err(bad());
        }
        let (name, value) = line.split_once(':').ok_or_else(bad)?;
        let valid_name = !name.is_empty()
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
        if !valid_name {
            return Err(bad());
        }
        headers.push((name.to_ascii_lowercase(), value.trim_matches([' ', '\t']).to_string()));
    }

    let mut req = Request {
        method: method.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        headers,
        body: Vec::new(),
        peer_is_loopback,
    };
    // Browsers send forms with Content-Length. Refuse Transfer-Encoding: this also
    // prevents request smuggling through a reverse proxy.
    if req.header("transfer-encoding").is_some() {
        return Err(BadRequest(501, "Transfer-Encoding is not supported"));
    }
    let lengths: Vec<&str> =
        req.headers.iter().filter(|(n, _)| n == "content-length").map(|(_, v)| v.as_str()).collect();
    if lengths.windows(2).any(|w| w[0] != w[1]) {
        return Err(bad());
    }
    let length = match lengths.first() {
        None => 0,
        Some(value) => {
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            match value.parse::<usize>() {
                // App::handle refuses the request from the header. Do not read the body.
                Ok(n) if n <= app.max_body_bytes() => n,
                _ => return Ok(Some(req)),
            }
        }
    };
    if length > 0 && req.header("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        let mut writer = stream;
        writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").map_err(|_| bad())?;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|_| bad())?;
    req.body = body;
    Ok(Some(req))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

/// A header value must not contain a line end or other control characters.
fn valid_header(name: &str, value: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_graphic() && b != b':')
        && value.bytes().all(|b| b == b'\t' || (b >= b' ' && b != 0x7f))
}

/// Write the response and close the connection.
fn write_response(mut stream: &TcpStream, response: &Response, head_only: bool) -> io::Result<()> {
    let status = if (100..=999).contains(&response.status) { response.status } else { 500 };
    let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    for (name, value) in &response.headers {
        let reserved = name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("connection");
        if !reserved && valid_header(name, value) {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", response.body.len()));
    stream.write_all(head.as_bytes())?;
    if !head_only {
        stream.write_all(&response.body)?;
    }
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    // Read what the client still sends (at most 64 KiB) before the socket closes. Else
    // the operating system can reset the connection and the client loses the response.
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = io::copy(&mut stream.take(64 * 1024), &mut io::sink());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_values_without_line_ends() {
        assert!(valid_header("location", "/a?b=c"));
        assert!(!valid_header("location", "/a\r\nset-cookie: x=y"));
        assert!(!valid_header("bad name", "x"));
    }

    #[test]
    fn reason_phrases() {
        assert_eq!(reason(303), "See Other");
        assert_eq!(reason(299), "");
    }
}
