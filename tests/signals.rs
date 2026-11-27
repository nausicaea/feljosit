//! The server stops on `SIGTERM` and `SIGINT` (ADR-0022). These tests start the real
//! executable, because a signal handler is for the complete process.
#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A running server with the log lines of stderr.
struct Server {
    child: Child,
    port: u16,
    lines: mpsc::Receiver<String>,
    log: Vec<String>,
    _dir: common::TempDir,
}

impl Server {
    fn start() -> Server {
        let dir = common::tempdir();
        // A free port. Another program can take it before the server, but this is improbable.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut child = Command::new(env!("CARGO_BIN_EXE_feljosit"))
            .args(["serve", "--port", &port.to_string()])
            .env("FELJOSIT_DATA_DIR", dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let _ = sender.send(line);
            }
        });
        let mut server = Server { child, port, lines, log: Vec::new(), _dir: dir };
        server.wait_for_line("runs at");
        server
    }

    fn wait_for_line(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Ok(line) = self.lines.recv_timeout(Duration::from_millis(100)) {
                let found = line.contains(text);
                self.log.push(line);
                if found {
                    return;
                }
            }
        }
        panic!("The server did not write {text:?}. Log: {:?}", self.log);
    }

    fn signal(&self, signal: libc::c_int) {
        let pid = libc::pid_t::try_from(self.child.id()).unwrap();
        assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    }

    fn wait(&mut self, limit: Duration) -> ExitStatus {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                self.log.extend(self.lines.try_iter());
                return status;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("The server did not stop in {limit:?}. Log: {:?}", self.log);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn get(&self, path: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
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
