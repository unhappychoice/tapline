use super::*;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::Command;

#[test]
fn initialization_output_failure_restores_terminal() {
    check_output_failure(1);
}

#[test]
fn restoration_output_failure_restores_terminal() {
    check_output_failure(4);
}

fn check_output_failure(fail_on_flush: usize) {
    if std::env::var_os("TAPLINE_RAW_MODE_TEST_CHILD").is_some() {
        let mut output = FailingOutput { fail_on_flush };
        let mut ran = false;
        let error = with_alt_screen(&mut output, |_| {
            ran = true;
            assert!(terminal::is_raw_mode_enabled()?);
            Ok(())
        })
        .unwrap_err();
        assert_eq!(ran, fail_on_flush != 1);
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::BrokenPipe
        );
        return;
    }
    check_in_child();
}

fn check_in_child() {
    let (master, slave) = open_pty();
    let before = settings(&slave);
    let name = std::thread::current().name().unwrap().to_owned();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture"])
        .env("TAPLINE_RAW_MODE_TEST_CHILD", "1")
        .stdin(slave.try_clone().unwrap())
        .output()
        .unwrap();
    let after = settings(&slave);
    drop(master);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(before.c_iflag, after.c_iflag);
    assert_eq!(before.c_oflag, after.c_oflag);
    assert_eq!(before.c_cflag, after.c_cflag);
    assert_eq!(local_modes(&before), local_modes(&after));
    assert_eq!(before.c_cc, after.c_cc);
    assert_eq!(unsafe { libc::cfgetispeed(&before) }, unsafe {
        libc::cfgetispeed(&after)
    });
    assert_eq!(unsafe { libc::cfgetospeed(&before) }, unsafe {
        libc::cfgetospeed(&after)
    });
}

fn local_modes(settings: &libc::termios) -> libc::tcflag_t {
    // Darwin sets PENDIN when restoring canonical mode; it is pending input
    // state maintained by the kernel, rather than a mode we can restore.
    #[cfg(target_os = "macos")]
    let ignored = libc::PENDIN;
    #[cfg(not(target_os = "macos"))]
    let ignored = 0;
    settings.c_lflag & !ignored
}

fn open_pty() -> (File, File) {
    let (mut master, mut slave) = (-1, -1);
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0, "{}", io::Error::last_os_error());
    unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
}

fn settings(file: &File) -> libc::termios {
    let mut settings = std::mem::MaybeUninit::uninit();
    assert_eq!(
        unsafe { libc::tcgetattr(file.as_raw_fd(), settings.as_mut_ptr()) },
        0
    );
    unsafe { settings.assume_init() }
}

struct FailingOutput {
    fail_on_flush: usize,
}

impl Write for FailingOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.fail_on_flush -= 1;
        if self.fail_on_flush == 0 {
            assert!(terminal::is_raw_mode_enabled()?);
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "output failed"));
        }
        Ok(())
    }
}
