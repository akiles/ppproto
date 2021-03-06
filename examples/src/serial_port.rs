use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::sys::termios;
use nix::Error;

pub struct SerialPort {
    fd: RawFd,
}

impl SerialPort {
    pub fn new(path: &Path) -> io::Result<Self> {
        let fd = nix::fcntl::open(
            path,
            OFlag::O_RDWR | OFlag::O_NOCTTY,
            nix::sys::stat::Mode::empty(),
        )
        .map_err(to_io_error)?;

        let mut cfg = termios::tcgetattr(fd).map_err(to_io_error)?;
        cfg.input_flags = termios::InputFlags::empty();
        cfg.output_flags = termios::OutputFlags::empty();
        cfg.control_flags = termios::ControlFlags::empty();
        cfg.local_flags = termios::LocalFlags::empty();
        termios::cfmakeraw(&mut cfg);
        cfg.input_flags |= termios::InputFlags::IGNBRK;
        cfg.control_flags |= termios::ControlFlags::CREAD;
        cfg.control_flags |= termios::ControlFlags::CRTSCTS;
        termios::cfsetospeed(&mut cfg, termios::BaudRate::B115200).map_err(to_io_error)?;
        termios::cfsetispeed(&mut cfg, termios::BaudRate::B115200).map_err(to_io_error)?;
        termios::cfsetspeed(&mut cfg, termios::BaudRate::B115200).map_err(to_io_error)?;
        termios::tcsetattr(fd, termios::SetArg::TCSANOW, &cfg).map_err(to_io_error)?;
        termios::tcflush(fd, termios::FlushArg::TCIOFLUSH).map_err(to_io_error)?;

        Ok(Self { fd })
    }

    pub fn set_nonblocking(&mut self, nonblocking: bool) -> io::Result<()> {
        let f = if nonblocking {
            OFlag::O_NONBLOCK
        } else {
            OFlag::empty()
        };
        fcntl(self.fd, FcntlArg::F_SETFL(f)).map_err(to_io_error)?;
        Ok(())
    }
}

impl AsRawFd for SerialPort {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl io::Read for SerialPort {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        nix::unistd::read(self.fd, buf).map_err(to_io_error)
    }
}

impl io::Write for SerialPort {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        nix::unistd::write(self.fd, buf).map_err(to_io_error)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn to_io_error(e: Error) -> io::Error {
    match e {
        Error::Sys(errno) => errno.into(),
        e => io::Error::new(io::ErrorKind::InvalidInput, e),
    }
}
