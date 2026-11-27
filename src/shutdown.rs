//! Stop the server on `SIGTERM` and `SIGINT` (Unix).
//!
//! A container engine sends `SIGTERM` to stop a container (`docker stop`). In a
//! container, Feljosit is process 1. The kernel ignores a signal for process 1 if the
//! process has no handler. Thus the server must have its own handler (ADR-0022).
//!
//! The handler only writes one byte to a pipe ("self-pipe"). This is safe in a signal
//! handler. The accept loop of the server waits for the listener and for the pipe at
//! the same time. When the pipe has data, the loop stops. A second signal stops the
//! process immediately.
//!
//! On other systems, [`Stop::wait`] always returns [`Event::Connection`]. Then Ctrl+C
//! stops the process, as before.

use std::io;
use std::net::TcpListener;

/// The result of [`Stop::wait`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The listener has a connection (or an error) for `accept`.
    Connection,
    /// The process received a stop signal.
    Stop,
}

/// The stop signal of the process. Make it with [`Stop::install`].
#[derive(Debug)]
pub struct Stop {
    #[cfg(unix)]
    read_fd: libc::c_int,
}

#[cfg(unix)]
mod unix {
    use std::io;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    /// The write end of the pipe, for the signal handler.
    static WRITE_FD: AtomicI32 = AtomicI32::new(-1);
    /// True after the first signal.
    static RECEIVED: AtomicBool = AtomicBool::new(false);
    /// The read end of the pipe, or the OS error code of the installation.
    static READ_FD: OnceLock<Result<libc::c_int, i32>> = OnceLock::new();

    /// The signal handler. It uses only async-signal-safe functions. The write goes
    /// to a non-blocking pipe with free space and thus does not change `errno`.
    extern "C" fn on_signal(_signal: libc::c_int) {
        if RECEIVED.swap(true, Ordering::SeqCst) {
            // The second signal: the operator does not want to wait.
            unsafe { libc::_exit(130) };
        }
        let fd = WRITE_FD.load(Ordering::SeqCst);
        if fd >= 0 {
            let byte = 1u8;
            unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
        }
    }

    fn check(result: libc::c_int) -> io::Result<libc::c_int> {
        if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result) }
    }

    unsafe fn add_fd_flag(
        fd: libc::c_int,
        get: libc::c_int,
        set: libc::c_int,
        flag: libc::c_int,
    ) -> io::Result<()> {
        unsafe {
            let flags = check(libc::fcntl(fd, get))?;
            check(libc::fcntl(fd, set, flags | flag))?;
        }
        Ok(())
    }

    fn install() -> io::Result<libc::c_int> {
        let mut fds = [0 as libc::c_int; 2];
        unsafe {
            check(libc::pipe(fds.as_mut_ptr()))?;
            for fd in fds {
                add_fd_flag(fd, libc::F_GETFD, libc::F_SETFD, libc::FD_CLOEXEC)?;
            }
            add_fd_flag(fds[1], libc::F_GETFL, libc::F_SETFL, libc::O_NONBLOCK)?;
        }
        WRITE_FD.store(fds[1], Ordering::SeqCst);
        for signal in [libc::SIGTERM, libc::SIGINT] {
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                check(libc::sigaction(signal, &action, std::ptr::null_mut()))?;
            }
        }
        Ok(fds[0])
    }

    /// Install the handlers one time for the process.
    pub fn read_fd() -> io::Result<libc::c_int> {
        READ_FD
            .get_or_init(|| install().map_err(|e| e.raw_os_error().unwrap_or(libc::EINVAL)))
            .map_err(io::Error::from_raw_os_error)
    }

    /// True if the process received a stop signal.
    pub fn received() -> bool {
        RECEIVED.load(Ordering::SeqCst)
    }
}

impl Stop {
    /// Install the handlers for `SIGTERM` and `SIGINT`. A second call gives the same pipe.
    pub fn install() -> io::Result<Stop> {
        #[cfg(unix)]
        return Ok(Stop { read_fd: unix::read_fd()? });
        #[cfg(not(unix))]
        Ok(Stop {})
    }

    /// Prepare the listener for [`Stop::wait`]. On Unix, the listener becomes
    /// non-blocking. Then `accept` cannot block after `wait` if the client closed the
    /// connection in the meantime. Set each accepted stream to blocking again.
    pub fn prepare(&self, listener: &TcpListener) -> io::Result<()> {
        #[cfg(unix)]
        listener.set_nonblocking(true)?;
        #[cfg(not(unix))]
        let _ = listener;
        Ok(())
    }

    /// Wait for a connection on the listener or for a stop signal.
    pub fn wait(&self, listener: &TcpListener) -> io::Result<Event> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            loop {
                if unix::received() {
                    return Ok(Event::Stop);
                }
                let mut fds = [
                    libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                    libc::pollfd { fd: self.read_fd, events: libc::POLLIN, revents: 0 },
                ];
                let result = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
                if result < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if fds[1].revents != 0 || unix::received() {
                    return Ok(Event::Stop);
                }
                if fds[0].revents != 0 {
                    return Ok(Event::Connection);
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = listener;
            Ok(Event::Connection)
        }
    }
}
