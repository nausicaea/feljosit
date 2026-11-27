//! The limit of connections for each client (ADR-0024). These tests start the real
//! executable, because the limits are in the server, not in the application.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use common::server::Server;

/// The maximum number of connections for each client (see `src/web/server.rs`).
const PER_CLIENT: usize = 16;

fn connect(server: &Server) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    stream
}

fn response(mut stream: TcpStream) -> String {
    let mut text = String::new();
    let _ = stream.read_to_string(&mut text);
    text
}

/// A request head with a body that never comes, from `device` behind the proxy.
fn waiting_request(server: &Server, device: &str) -> TcpStream {
    let mut stream = connect(server);
    write!(
        stream,
        "POST /login HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Forwarded-For: {device}\r\n\
         Content-Type: application/x-www-form-urlencoded\r\nContent-Length: 10\r\n\r\n"
    )
    .unwrap();
    stream
}

#[test]
fn one_client_cannot_use_all_connections() {
    let mut server = Server::start();
    let mut idle: Vec<TcpStream> = (0..PER_CLIENT).map(|_| connect(&server)).collect();
    std::thread::sleep(Duration::from_millis(300));
    assert!(response(connect(&server)).starts_with("HTTP/1.1 503"));
    server.wait_for_line("127.0.0.1 \"-\" 503");
    // A closed connection frees its place.
    drop(idle.pop());
    std::thread::sleep(Duration::from_millis(300));
    assert!(server.get("/").starts_with("HTTP/1.1 303"));
}

#[test]
fn the_limit_applies_to_each_device_behind_a_proxy() {
    let mut server = Server::start_with(&[("FELJOSIT_TRUSTED_PROXIES", "127.0.0.1")]);
    let _waiting: Vec<TcpStream> =
        (0..PER_CLIENT).map(|_| waiting_request(&server, "192.168.1.66")).collect();
    std::thread::sleep(Duration::from_millis(300));
    assert!(response(waiting_request(&server, "192.168.1.66")).starts_with("HTTP/1.1 503"));
    // The log shows the device, not the proxy.
    server.wait_for_line("192.168.1.66 \"-\" 503");
    // A different device behind the same proxy can connect.
    let mut other = connect(&server);
    write!(other, "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Forwarded-For: 192.168.1.20\r\n\r\n").unwrap();
    assert!(response(other).starts_with("HTTP/1.1 303"));
}
