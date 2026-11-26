//! Read a password from the terminal without an echo.
//!
//! This module replaces the crate `rpassword` (see ADR-0021). On Unix it uses
//! `termios` from `libc`. On Windows it uses three functions of `kernel32.dll`.

use std::io::{self, BufRead, Write};

/// Show the prompt on the standard error and read one line from the standard input.
/// If the standard input is a terminal, the terminal does not show the characters.
pub fn prompt_password(prompt: &str) -> io::Result<String> {
    let mut stderr = io::stderr();
    stderr.write_all(prompt.as_bytes())?;
    stderr.flush()?;
    let line = {
        let _echo = EchoOff::new()?;
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        line
    };
    if line.is_empty() {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "The input stopped."));
    }
    let line = line.strip_suffix('\n').unwrap_or(&line);
    Ok(line.strip_suffix('\r').unwrap_or(line).to_string())
}

#[cfg(unix)]
struct EchoOff(Option<libc::termios>);

#[cfg(unix)]
impl EchoOff {
    fn new() -> io::Result<EchoOff> {
        let fd = libc::STDIN_FILENO;
        // SAFETY: isatty, tcgetattr and tcsetattr only read and write the termios
        // structure that is given. The structure is plain data.
        unsafe {
            if libc::isatty(fd) != 1 {
                return Ok(EchoOff(None));
            }
            let mut original: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut original) != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut changed = original;
            changed.c_lflag &= !libc::ECHO;
            // Show the line end. Then the next output starts on a new line.
            changed.c_lflag |= libc::ECHONL;
            if libc::tcsetattr(fd, libc::TCSANOW, &changed) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(EchoOff(Some(original)))
        }
    }
}

#[cfg(unix)]
impl Drop for EchoOff {
    fn drop(&mut self) {
        if let Some(original) = &self.0 {
            // SAFETY: see EchoOff::new.
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, original);
            }
        }
    }
}

#[cfg(windows)]
mod console {
    pub type Handle = *mut core::ffi::c_void;
    pub const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    pub const ENABLE_ECHO_INPUT: u32 = 0x0004;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn GetStdHandle(std_handle: u32) -> Handle;
        pub fn GetConsoleMode(console: Handle, mode: *mut u32) -> i32;
        pub fn SetConsoleMode(console: Handle, mode: u32) -> i32;
    }
}

#[cfg(windows)]
struct EchoOff(Option<(console::Handle, u32)>);

#[cfg(windows)]
impl EchoOff {
    fn new() -> io::Result<EchoOff> {
        // SAFETY: the functions only read the handle and write the mode value.
        unsafe {
            let handle = console::GetStdHandle(console::STD_INPUT_HANDLE);
            let mut mode = 0u32;
            // An error means that the input is not a console (for example a pipe).
            if handle.is_null() || console::GetConsoleMode(handle, &mut mode) == 0 {
                return Ok(EchoOff(None));
            }
            if console::SetConsoleMode(handle, mode & !console::ENABLE_ECHO_INPUT) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(EchoOff(Some((handle, mode))))
        }
    }
}

#[cfg(windows)]
impl Drop for EchoOff {
    fn drop(&mut self) {
        if let Some((handle, mode)) = self.0 {
            // SAFETY: see EchoOff::new.
            unsafe {
                console::SetConsoleMode(handle, mode);
            }
            // The console does not show the line end of the password. Show it now.
            eprintln!();
        }
    }
}

#[cfg(not(any(unix, windows)))]
struct EchoOff;

#[cfg(not(any(unix, windows)))]
impl EchoOff {
    fn new() -> io::Result<EchoOff> {
        Err(io::Error::other("This operating system cannot read a password without an echo."))
    }
}
