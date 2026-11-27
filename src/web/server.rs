//! The HTTP server (HTTP/1.1 with the standard library).
//!
//! The application is for one person on the same computer, or for a family behind a
//! TLS reverse proxy (ADR-0023). Thus a small server is sufficient: one thread for
//! each connection, one request for each connection, a limit for the number of
//! connections and a limit for each client. The server reads the complete request
//! with size limits and time limits. Then [`App::handle`] makes the response.
//! See ADR-0021 and ADR-0024.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::IpAddr;
use std::net::{TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{App, Request, Response};
use crate::db::log;
use crate::shutdown::{Event, Stop};

/// The maximum number of connections at the same time.
const MAX_CONNECTIONS: usize = 64;
/// The maximum number of connections from one client at the same time (ADR-0024).
/// Thus one device cannot use all connections.
const MAX_CONNECTIONS_PER_CLIENT: usize = 16;
/// The maximum size of the request line and the headers together.
const MAX_HEAD_BYTES: usize = 64 * 1024;
/// The maximum number of header lines.
const MAX_HEADERS: usize = 100;
/// The time limit for one read or write operation on the connection.
const IO_TIMEOUT: Duration = Duration::from_secs(30);
/// The time limit to receive the request line and the headers (ADR-0024). A slow
/// client cannot keep a connection open with one byte in each `IO_TIMEOUT`.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// The time limit to receive the complete request, with the body. An upload of
/// 10 MB in this time needs approximately 85 kB/s.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// The maximum time to wait for the active connections after a stop signal.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// Serve the application on the listener. The function returns after a stop signal
/// (`SIGTERM` or `SIGINT` on Unix, see [`crate::shutdown`]) or if the listener has an
/// error. On other systems, Ctrl+C stops the process: SQLite keeps the database consistent.
pub fn serve(app: Arc<App>, listener: TcpListener) -> io::Result<()> {
    let stop = Stop::install()?;
    stop.prepare(&listener)?;
    let active = Arc::new(AtomicUsize::new(0));
    let clients = ClientCounts::default();
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
        let peer = stream.peer_addr().ok().map(|a| a.ip());
        let from_proxy =
            peer.is_some_and(|ip| app.settings.trusted_proxies.iter().any(|net| net.contains(ip)));
        // Behind a trusted proxy, the client is known only after the headers.
        let client_guard = if from_proxy { None } else { clients.acquire(crate::net::client_key(peer)) };
        if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS || (!from_proxy && client_guard.is_none())
        {
            active.fetch_sub(1, Ordering::SeqCst);
            log::info(&format!("{} \"-\" 503", peer.map_or_else(|| "-".to_string(), |ip| ip.to_string())));
            refuse(&stream);
            continue;
        }
        let app = app.clone();
        let guard = ActiveGuard(active.clone());
        let clients = clients.clone();
        let spawned = std::thread::Builder::new().name("feljosit-connection".into()).spawn(move || {
            let _guard = guard;
            let connection = Connection { stream, peer, from_proxy, client_guard, clients };
            handle_connection(&app, connection);
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

/// Send "503 Too many connections" from the accept loop. The short time limit keeps
/// the accept loop free if the client does not read.
fn refuse(stream: &TcpStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
    let _ = write_response(stream, &Response::text(503, "Too many connections"), false);
}

/// The number of connections of each client.
#[derive(Clone, Default)]
struct ClientCounts(Arc<Mutex<HashMap<String, usize>>>);

impl ClientCounts {
    /// Count one more connection of the client. `None` if the client has the maximum.
    fn acquire(&self, key: String) -> Option<ClientGuard> {
        let mut counts = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let count = counts.entry(key.clone()).or_insert(0);
        if *count >= MAX_CONNECTIONS_PER_CLIENT {
            return None;
        }
        *count += 1;
        Some(ClientGuard { counts: self.0.clone(), key })
    }
}

/// Decrease the number of connections of the client at the end of a connection.
struct ClientGuard {
    counts: Arc<Mutex<HashMap<String, usize>>>,
    key: String,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(count) = counts.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.key);
            }
        }
    }
}

/// An accepted connection.
struct Connection {
    stream: TcpStream,
    peer: Option<IpAddr>,
    /// The connection comes from a trusted reverse proxy.
    from_proxy: bool,
    /// The count of the client. For a proxy connection, it is set after the headers.
    client_guard: Option<ClientGuard>,
    clients: ClientCounts,
}

/// An error in the request. The server sends the status and closes the connection.
struct BadRequest(u16, &'static str);

/// The error for a failed read. A time limit gives "408 Request Timeout".
fn read_error(error: &io::Error) -> BadRequest {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => BadRequest(408, "Request Timeout"),
        _ => BadRequest(400, "Bad Request"),
    }
}

/// A reader with a time limit for the complete request, not only for one read.
struct DeadlineReader<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "The time limit of the request is over."));
        }
        self.stream.set_read_timeout(Some(remaining.min(IO_TIMEOUT)))?;
        let mut stream = self.stream;
        stream.read(buf)
    }
}

fn handle_connection(app: &App, mut connection: Connection) {
    let stream = &connection.stream;
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_nodelay(true);
    let started = Instant::now();
    let peer_is_loopback = connection.peer.is_some_and(|ip| ip.is_loopback());
    // The address for the log. Behind a proxy, it is known after the headers.
    let mut client = connection.peer;
    let mut reader = BufReader::new(DeadlineReader { stream, deadline: started + HEAD_TIMEOUT });
    let result = read_head(&mut reader, connection.peer, peer_is_loopback).and_then(|req| {
        let Some(mut req) = req else { return Ok(None) };
        client = app.client_address(&req);
        if connection.from_proxy {
            // The limit for each client also applies behind the proxy.
            let key = crate::net::client_key(client);
            connection.client_guard =
                Some(connection.clients.acquire(key).ok_or(BadRequest(503, "Too many connections"))?);
        }
        reader.get_mut().deadline = started + REQUEST_TIMEOUT;
        read_body(app, &mut reader, stream, &mut req)?;
        Ok(Some(req))
    });
    let client = client.map_or_else(|| "-".to_string(), |ip| ip.to_string());
    let (req, response) = match result {
        Ok(Some(req)) => {
            let response = run(app, &req);
            (req, response)
        }
        // The client closed the connection before it sent a request.
        Ok(None) => return,
        Err(BadRequest(status, message)) => {
            log::info(&format!("{client} \"-\" {status}"));
            let _ = write_response(stream, &Response::text(status, message), false);
            return;
        }
    };
    log::info(&format!("{client} \"{} {}\" {}", req.method, req.path, response.status));
    let _ = write_response(stream, &response, req.method == "HEAD");
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
    let count = reader.by_ref().take(limit).read_until(b'\n', &mut line).map_err(|e| read_error(&e))?;
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

/// Read the request line and the headers. `Ok(None)` means that the client sent nothing.
fn read_head(
    reader: &mut impl BufRead,
    peer: Option<IpAddr>,
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

    Ok(Some(Request {
        method: method.to_string(),
        path: path.to_string(),
        query: query.to_string(),
        headers,
        body: Vec::new(),
        peer_is_loopback,
        peer,
    }))
}

/// Read the body of the request (`Content-Length`).
fn read_body(
    app: &App,
    reader: &mut impl BufRead,
    stream: &TcpStream,
    req: &mut Request,
) -> Result<(), BadRequest> {
    let bad = || BadRequest(400, "Bad Request");
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
                _ => return Ok(()),
            }
        }
    };
    if length > 0 && req.header("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        let mut writer = stream;
        writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").map_err(|_| bad())?;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|e| read_error(&e))?;
    req.body = body;
    Ok(())
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
        408 => "Request Timeout",
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
        assert_eq!(reason(408), "Request Timeout");
        assert_eq!(reason(299), "");
    }

    #[test]
    fn a_slow_client_stops_at_the_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        // The client sends one byte each 50 ms. Each read is fast, but the head is never complete.
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            for byte in b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n".iter().cycle().take(200) {
                if stream.write_all(&[*byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let (stream, _) = listener.accept().unwrap();
        let started = Instant::now();
        let mut reader = BufReader::new(DeadlineReader {
            stream: &stream,
            deadline: started + Duration::from_millis(400),
        });
        let result = read_head(&mut reader, None, false);
        let elapsed = started.elapsed();
        assert!(matches!(result, Err(BadRequest(408, _))));
        assert!(elapsed >= Duration::from_millis(390), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        drop(reader);
        drop(stream);
        client.join().unwrap();
    }

    #[test]
    fn the_connections_of_each_client_are_counted() {
        let counts = ClientCounts::default();
        let guards: Vec<ClientGuard> =
            (0..MAX_CONNECTIONS_PER_CLIENT).map(|_| counts.acquire("192.168.1.66".into()).unwrap()).collect();
        assert!(counts.acquire("192.168.1.66".into()).is_none());
        // A different client is not affected.
        let other = counts.acquire("192.168.1.20".into()).unwrap();
        drop(guards);
        assert!(counts.acquire("192.168.1.66".into()).is_some());
        drop(other);
        assert!(counts.0.lock().unwrap().is_empty(), "No entries without connections");
    }
}
