//! The server stops on `SIGTERM` and `SIGINT` (ADR-0022). These tests start the real
//! executable, because a signal handler is for the complete process.
#![cfg(unix)]

mod common;

use std::net::TcpStream;
use std::time::Duration;

use common::server::Server;

trait Signal {
    fn signal(&self, signal: libc::c_int);
}

impl Signal for Server {
    fn signal(&self, signal: libc::c_int) {
        let pid = libc::pid_t::try_from(self.child.id()).unwrap();
        assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    }
}

#[test]
fn sigterm_stops_the_server() {
    let mut server = Server::start();
    assert!(server.get("/").starts_with("HTTP/1.1 303"));
    server.signal(libc::SIGTERM);
    let status = server.wait(Duration::from_secs(10));
    assert!(status.success(), "{status:?}");
    assert!(server.log.iter().any(|line| line.contains("The server stops.")), "{:?}", server.log);
    // The listener is closed.
    assert!(TcpStream::connect(("127.0.0.1", server.port)).is_err());
}

#[test]
fn sigint_stops_the_server() {
    let mut server = Server::start();
    server.signal(libc::SIGINT);
    let status = server.wait(Duration::from_secs(10));
    assert!(status.success(), "{status:?}");
}

#[test]
fn second_signal_stops_the_server_immediately() {
    let mut server = Server::start();
    // An open connection without a request. The server waits for it after the first signal.
    let _idle = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    server.signal(libc::SIGTERM);
    server.wait_for_line("The server stops.");
    server.signal(libc::SIGTERM);
    let status = server.wait(Duration::from_secs(2));
    assert_eq!(status.code(), Some(130));
}
