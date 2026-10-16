//! `--serial-pty PATH`: the S3's USB serial as a pseudo-terminal, so host tools open PATH like the
//! real device's /dev/ttyACM0. Bytes written to PATH go to the same RX FIFO as `POST /serial`;
//! the firmware's USB serial output is copied to it. The bridge test is `usb_tests` in
//! `soc/esp32s3/periph.rs` (it needs the S3 bus).

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::path::{Path, PathBuf};

pub struct SerialPty {
    master: File,
    link: PathBuf,
}

impl SerialPty {
    /// Open a pty (raw, no echo, master non-blocking) and make `link` a symlink to its slave.
    pub fn open(link: &Path) -> std::io::Result<SerialPty> {
        // SAFETY: plain libc calls on an fd and buffers owned here; the fd moves into a File.
        unsafe {
            let fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let master = File::from_raw_fd(fd);
            if libc::grantpt(fd) != 0 || libc::unlockpt(fd) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut t) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::cfmakeraw(&mut t);
            if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK);
            let mut name = [0 as libc::c_char; 128];
            if libc::ptsname_r(fd, name.as_mut_ptr(), name.len()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let slave = PathBuf::from(std::ffi::CStr::from_ptr(name.as_ptr()).to_string_lossy().into_owned());
            let _ = std::fs::remove_file(link);
            std::os::unix::fs::symlink(&slave, link)?;
            Ok(SerialPty { master, link: link.to_path_buf() })
        }
    }

    /// Bytes the host wrote to the port since the last call (empty if none, or nobody has it open).
    pub fn read_input(&mut self) -> Vec<u8> {
        let mut buf = [0u8; 4096];
        match self.master.read(&mut buf) {
            Ok(n) => buf[..n].to_vec(),
            Err(_) => Vec::new(), // EAGAIN, or EIO while no slave is open
        }
    }

    /// Firmware output for the host. Dropped when the host does not keep up (a real port would too).
    pub fn write_output(&mut self, data: &[u8]) {
        let mut rest = data;
        while !rest.is_empty() {
            match self.master.write(rest) {
                Ok(n) if n > 0 => rest = &rest[n..],
                _ => break,
            }
        }
    }
}

impl Drop for SerialPty {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.link);
    }
}
