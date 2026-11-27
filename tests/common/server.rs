//! A test helper that starts the real executable.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A running server with the log lines of stderr.
pub struct Server {
    pub child: Child,
    pub port: u16,
    lines: mpsc::Receiver<String>,
    pub log: Vec<String>,
    _dir: super::TempDir,
}

impl Server {
    pub fn start() -> Server {
        Server::start_with(&[])
    }

    /// Start the server with more environment variables.
    pub fn start_with(env: &[(&str, &str)]) -> Server {
        let dir = super::tempdir();
        // A free port. Another program can take it before the server, but this is improbable.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut child = Command::new(env!("CARGO_BIN_EXE_feljosit"))
            .args(["serve", "--port", &port.to_string()])
            .env("FELJOSIT_DATA_DIR", dir.path())
            .envs(env.iter().copied())
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

    pub fn wait_for_line(&mut self, text: &str) {
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

    pub fn wait(&mut self, limit: Duration) -> ExitStatus {
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

    pub fn get(&self, path: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
