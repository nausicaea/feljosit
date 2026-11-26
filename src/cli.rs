//! The command line interface.
//!
//! Without a command, the application starts the web server and opens the browser.
//! Thus a double-click on the executable file starts the application.

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use crate::config::{LOOPBACK_HOSTS, Settings};
use crate::db::{self, log};
use crate::services::{Error as ServiceError, users};
use crate::web::App;

const USAGE: &str = "\
Feljosit: local-first budgeting.

Usage:
  feljosit                        Start the web application and open the browser.
  feljosit serve [OPTIONS]        Start the web application.
      --host HOST                 Address to bind to (default: 127.0.0.1).
      --port PORT                 Port (default: 8765).
      --open                      Open the browser.
  feljosit migrate                Upgrade the database schema.
  feljosit create-user USERNAME   Add a user.
  feljosit set-password USERNAME  Set a new password for a user.
  feljosit --version              Show the version.
  feljosit --help                 Show this help.

Environment variables: FELJOSIT_DATA_DIR, FELJOSIT_HOST, FELJOSIT_PORT,
FELJOSIT_ALLOWED_HOSTS, FELJOSIT_SECURE_COOKIES. See the README.
";

/// The input and the output of a command (the tests replace them).
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// Read a password without an echo.
    pub read_password: &'a mut dyn FnMut(&str) -> io::Result<String>,
    /// Start the server. The tests replace it.
    pub run_server: &'a mut dyn FnMut(Settings, ServeOptions) -> Result<(), String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeOptions {
    pub host: String,
    pub port: u16,
    pub open_browser: bool,
}

enum Command {
    Serve(Option<String>, Option<u16>, bool),
    Migrate,
    CreateUser(String),
    SetPassword(String),
    Help,
    Version,
}

fn parse(args: &[String]) -> Result<Command, String> {
    let Some(first) = args.first() else {
        return Ok(Command::Serve(None, None, true));
    };
    let rest = &args[1..];
    let one_name = |command: &str| match rest {
        [name] if !name.starts_with('-') => Ok(name.clone()),
        _ => Err(format!("The command {command} needs one username.")),
    };
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" => Ok(Command::Version),
        "migrate" if rest.is_empty() => Ok(Command::Migrate),
        "create-user" => one_name("create-user").map(Command::CreateUser),
        "set-password" => one_name("set-password").map(Command::SetPassword),
        "serve" => {
            let (mut host, mut port, mut open) = (None, None, false);
            let mut iter = rest.iter();
            while let Some(arg) = iter.next() {
                let (name, inline) = match arg.split_once('=') {
                    Some((n, v)) => (n, Some(v.to_string())),
                    None => (arg.as_str(), None),
                };
                let mut value = || {
                    inline.clone().or_else(|| iter.next().cloned()).ok_or(format!("{name} needs a value."))
                };
                match name {
                    "--host" => host = Some(value()?),
                    "--port" => {
                        port = Some(value()?.parse().map_err(|_| "--port needs a port number.".to_string())?)
                    }
                    "--open" => open = true,
                    _ => return Err(format!("Unknown option: {arg}")),
                }
            }
            Ok(Command::Serve(host, port, open))
        }
        other => Err(format!("Unknown command: {other}")),
    }
}

fn read_new_password(io: &mut Io<'_>) -> Result<String, ServiceError> {
    let read = |io: &mut Io<'_>, prompt: &str| {
        (io.read_password)(prompt)
            .map_err(|e| ServiceError::service(format!("Cannot read the password: {e}")))
    };
    let first = read(io, "Password: ")?;
    let second = read(io, "Password (again): ")?;
    if first != second {
        return Err(ServiceError::service("The two passwords are not the same."));
    }
    Ok(first)
}

fn admin_command(command: &Command, settings: &Settings, io: &mut Io<'_>) -> Result<(), String> {
    let mut conn = db::open(settings)?;
    db::migrate(&mut conn)?;
    db::restrict_database_file(settings).map_err(|e| e.to_string())?;
    let (create, username) = match command {
        Command::Migrate => {
            let _ = writeln!(io.out, "The database schema is up to date.");
            return Ok(());
        }
        Command::CreateUser(name) => (true, name),
        Command::SetPassword(name) => (false, name),
        _ => unreachable!("only the administration commands"),
    };
    let password = read_new_password(io).map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if create {
        let user = users::create_user(&tx, username, &password).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        let _ = writeln!(io.out, "The user '{}' is added.", user.username);
    } else {
        users::reset_password(&tx, username, &password).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        let _ = writeln!(io.out, "The password is changed.");
    }
    Ok(())
}

/// Run a command with the environment variables `env`. Return the exit code.
pub fn run(args: &[String], env: &std::collections::HashMap<String, String>, io: &mut Io<'_>) -> i32 {
    let command = match parse(args) {
        Ok(command) => command,
        Err(message) => {
            let _ = writeln!(io.err, "ERROR: {message}\n\n{USAGE}");
            return 2;
        }
    };
    let settings = match Settings::from_map(env) {
        Ok(settings) => settings,
        Err(message) => {
            let _ = writeln!(io.err, "ERROR: {message}");
            return 2;
        }
    };
    let result = match command {
        Command::Help => {
            let _ = write!(io.out, "{USAGE}");
            Ok(())
        }
        Command::Version => {
            let _ = writeln!(io.out, "feljosit {}", crate::VERSION);
            Ok(())
        }
        Command::Serve(host, port, open_browser) => {
            let options = ServeOptions {
                host: host.unwrap_or_else(|| settings.host.clone()),
                port: port.unwrap_or(settings.port),
                open_browser,
            };
            if !LOOPBACK_HOSTS.contains(&options.host.as_str()) {
                let _ = writeln!(
                    io.err,
                    "WARNING: The application listens on {}. Other computers can connect. \
                     Use a TLS reverse proxy and set FELJOSIT_SECURE_COOKIES=true.",
                    options.host
                );
            }
            (io.run_server)(settings, options)
        }
        ref admin => admin_command(admin, &settings, io),
    };
    match result {
        Ok(()) => 0,
        Err(message) => {
            let _ = writeln!(io.err, "ERROR: {message}");
            1
        }
    }
}

/// The address for the browser. An address for all interfaces becomes the loopback
/// address.
fn browser_url(host: &str, port: u16) -> String {
    let host = match host {
        "0.0.0.0" => "127.0.0.1",
        "::" => "::1",
        other => other,
    };
    if host.contains(':') { format!("http://[{host}]:{port}/") } else { format!("http://{host}:{port}/") }
}

/// Start the web server. This function returns when the user stops the server.
pub fn run_server(settings: Settings, options: ServeOptions) -> Result<(), String> {
    let url = browser_url(&options.host, options.port);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Cannot start the server: {e}"))?;
    let address = if options.host.contains(':') {
        format!("[{}]:{}", options.host, options.port)
    } else {
        format!("{}:{}", options.host, options.port)
    };
    let listener = match runtime.block_on(tokio::net::TcpListener::bind(&address)) {
        Ok(listener) => listener,
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            if options.open_browser {
                // Possibly Feljosit runs already. Then the browser shows it.
                log::warning(&format!("The port {} is in use. The browser opens {url}.", options.port));
                let _ = open::that(&url);
                return Ok(());
            }
            return Err(format!(
                "The port {} is in use. Possibly Feljosit runs already. Use a different port \
                 (--port) or stop the other program.",
                options.port
            ));
        }
        Err(error) => return Err(format!("Cannot listen on {address}: {error}")),
    };
    let database = settings.database_path();
    let conn = db::open(&settings)?;
    let app = App::new(settings.clone(), conn)?;
    db::restrict_database_file(&settings).map_err(|e| e.to_string())?;
    log::info(&format!("Feljosit {} runs at {url}", crate::VERSION));
    log::info(&format!("The data is in {}", database.display()));
    log::info("Press Ctrl+C to stop the server.");
    if options.open_browser && open::that(&url).is_err() {
        log::warning(&format!("Cannot open the browser. Open {url} manually."));
    }
    runtime
        .block_on(crate::web::server::serve(Arc::new(app), listener))
        .map_err(|e| format!("The server stopped with an error: {e}"))
}

/// The entry point of the executable.
pub fn main(args: Vec<String>) -> i32 {
    // New files (database, WAL journal) are only readable by the owner.
    #[cfg(unix)]
    unsafe {
        libc::umask(0o077);
    }
    let started_without_arguments = args.is_empty();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut read_password = |prompt: &str| rpassword::prompt_password(prompt);
    let mut server = run_server;
    let env = std::env::vars().collect();
    let code = run(
        &args,
        &env,
        &mut Io {
            out: &mut stdout,
            err: &mut stderr,
            read_password: &mut read_password,
            run_server: &mut server,
        },
    );
    if code != 0 && started_without_arguments {
        // The window of a double-clicked program closes at the end. Keep the message
        // on the screen.
        eprintln!("Press Enter to close this window.");
        let _ = io::stdin().lock().read_line(&mut String::new());
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(browser_url("127.0.0.1", 8765), "http://127.0.0.1:8765/");
        assert_eq!(browser_url("0.0.0.0", 9000), "http://127.0.0.1:9000/");
        assert_eq!(browser_url("::1", 8765), "http://[::1]:8765/");
    }

    #[test]
    fn arguments() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(matches!(parse(&[]), Ok(Command::Serve(None, None, true))));
        assert!(matches!(
            parse(&args(&["serve", "--host", "0.0.0.0", "--port=9000"])),
            Ok(Command::Serve(Some(h), Some(9000), false)) if h == "0.0.0.0"
        ));
        assert!(parse(&args(&["serve", "--port", "x"])).is_err());
        assert!(parse(&args(&["create-user"])).is_err());
        assert!(parse(&args(&["unknown"])).is_err());
    }
}
