//! The command line interface.

mod common;

use std::collections::HashMap;
use std::io;

use feljosit::cli::{Io, ServeOptions, run};
use feljosit::config::Settings;

struct Output {
    code: i32,
    out: String,
    err: String,
    served: Vec<ServeOptions>,
}

fn call(args: &[&str], dir: &std::path::Path, passwords: &[&str]) -> Output {
    feljosit::services::users::use_fast_hasher_for_tests();
    let env = HashMap::from([("FELJOSIT_DATA_DIR".to_string(), dir.join("data").display().to_string())]);
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut answers = passwords.iter().map(|p| p.to_string()).collect::<Vec<_>>().into_iter();
    let mut read_password = |_: &str| answers.next().ok_or_else(|| io::Error::other("no more input"));
    let mut served = Vec::new();
    let mut run_server = |_: Settings, options: ServeOptions| {
        served.push(options);
        Ok(())
    };
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let code = run(
        &args,
        &env,
        &mut Io {
            out: &mut out,
            err: &mut err,
            read_password: &mut read_password,
            run_server: &mut run_server,
        },
    );
    Output { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap(), served }
}

#[test]
fn migrate() {
    let dir = common::tempdir();
    let result = call(&["migrate"], dir.path(), &[]);
    assert_eq!(result.code, 0, "{}", result.err);
    assert!(result.out.contains("up to date"));
    assert!(dir.path().join("data").join("feljosit.sqlite3").exists());
}

#[test]
fn create_user_and_set_password() {
    let dir = common::tempdir();
    let password = "a long enough password";
    let first = call(&["create-user", "anna"], dir.path(), &[password, password]);
    assert_eq!(first.code, 0, "{}", first.err);
    assert!(first.out.contains("is added"));
    let again = call(&["create-user", "anna"], dir.path(), &[password, password]);
    assert_eq!(again.code, 1);
    assert!(again.err.contains("exists already"));
    assert_eq!(call(&["set-password", "anna"], dir.path(), &[password, password]).code, 0);
    assert_eq!(call(&["set-password", "nobody"], dir.path(), &[password, password]).code, 1);
}

#[test]
fn password_mismatch() {
    let dir = common::tempdir();
    let result = call(&["create-user", "anna"], dir.path(), &["first long password", "second long password"]);
    assert_eq!(result.code, 1);
    assert!(result.err.contains("not the same"));
}

#[test]
fn serve_warns_on_public_address() {
    let dir = common::tempdir();
    let result = call(&["serve", "--host", "0.0.0.0", "--port", "9000"], dir.path(), &[]);
    assert_eq!(result.code, 0);
    assert_eq!(result.served, vec![ServeOptions { host: "0.0.0.0".into(), port: 9000, open_browser: false }]);
    assert!(result.err.contains("WARNING"));
}

#[test]
fn start_without_arguments_opens_the_browser() {
    let dir = common::tempdir();
    let result = call(&[], dir.path(), &[]);
    assert_eq!(result.code, 0);
    assert_eq!(
        result.served,
        vec![ServeOptions { host: "127.0.0.1".into(), port: 8765, open_browser: true }]
    );
    assert!(result.err.is_empty());
}

#[test]
fn help_version_and_errors() {
    let dir = common::tempdir();
    assert!(call(&["--help"], dir.path(), &[]).out.contains("create-user"));
    assert!(call(&["--version"], dir.path(), &[]).out.starts_with("feljosit "));
    let unknown = call(&["unknown"], dir.path(), &[]);
    assert_eq!(unknown.code, 2);
    assert!(unknown.err.contains("Unknown command"));
}

#[cfg(unix)]
#[test]
fn database_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = common::tempdir();
    assert_eq!(call(&["migrate"], dir.path(), &[]).code, 0);
    let data = dir.path().join("data");
    assert_eq!(std::fs::metadata(&data).unwrap().permissions().mode() & 0o077, 0);
    let file = data.join("feljosit.sqlite3");
    assert_eq!(std::fs::metadata(file).unwrap().permissions().mode() & 0o077, 0);
}
