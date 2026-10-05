use super::*;
use std::os::fd::{FromRawFd, OwnedFd};
use tokio::io::AsyncReadExt;

fn pipe() -> (File, File) {
    let mut fds = [-1; 2];
    // SAFETY: pipe2 writes exactly two descriptors into the valid array.
    assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
    // SAFETY: successful pipe2 returned two distinct newly owned descriptors.
    unsafe {
        (
            File::from(OwnedFd::from_raw_fd(fds[0])),
            File::from(OwnedFd::from_raw_fd(fds[1])),
        )
    }
}

#[tokio::test]
async fn lifecycle_shell_input_pipe_bytes_eof_and_flag_restoration() {
    use std::io::Write;
    let (read, mut write) = pipe();
    let original = get_flags(read.as_raw_fd()).unwrap();
    let observer = read.try_clone().unwrap();
    let mut input = Input::from_file(read).unwrap();
    assert_ne!(
        get_flags(observer.as_raw_fd()).unwrap() & libc::O_NONBLOCK,
        0
    );
    write.write_all(b"1+1\nlast").unwrap();
    drop(write);
    let mut bytes = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        input.read_to_end(&mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(bytes, b"1+1\nlast");
    input.finish().unwrap();
    assert_eq!(get_flags(observer.as_raw_fd()).unwrap(), original);
}

#[tokio::test]
async fn lifecycle_shell_input_idle_drop_restores_flags_without_reader_task() {
    let (read, _write) = pipe();
    let observer = read.try_clone().unwrap();
    let original = get_flags(observer.as_raw_fd()).unwrap();
    let mut input = Input::from_file(read).unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), input.read_u8())
            .await
            .is_err()
    );
    drop(input);
    assert_eq!(get_flags(observer.as_raw_fd()).unwrap(), original);
}

#[tokio::test]
async fn lifecycle_shell_input_preserves_preexisting_nonblocking_flags() {
    let (read, _write) = pipe();
    let observer = read.try_clone().unwrap();
    let flags = get_flags(observer.as_raw_fd()).unwrap() | libc::O_NONBLOCK;
    set_flags(observer.as_raw_fd(), flags).unwrap();
    Input::from_file(read).unwrap().finish().unwrap();
    assert_eq!(get_flags(observer.as_raw_fd()).unwrap(), flags);
}

#[tokio::test]
async fn lifecycle_shell_input_regular_file_and_null_eof() {
    use std::io::{Seek, Write};
    let path = std::env::temp_dir().join(format!(
        "ios-shell-input-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let mut file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    // Unlink only this uniquely created fixture; the owned descriptor survives.
    std::fs::remove_file(path).unwrap();
    file.write_all(b"2+2\n").unwrap();
    file.rewind().unwrap();
    let mut input = Input::from_file(file).unwrap();
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(bytes, b"2+2\n");
    input.finish().unwrap();
    let mut input = Input::from_file(File::open("/dev/null").unwrap()).unwrap();
    assert_eq!(input.read(&mut [0; 8]).await.unwrap(), 0);
    input.finish().unwrap();
}

#[tokio::test]
async fn lifecycle_shell_input_native_admission_error_retains_io_kind() {
    let file = File::open("/dev/zero").unwrap();
    let observer = file.try_clone().unwrap();
    let original = get_flags(observer.as_raw_fd()).unwrap();
    let error = match Input::from_file(file) {
        Err(error) => error,
        Ok(_) => panic!("unpollable character input must retain its registration failure"),
    };
    assert_eq!(error.raw_os_error(), Some(libc::EPERM));
    assert_eq!(get_flags(observer.as_raw_fd()).unwrap(), original);
}

#[tokio::test]
async fn lifecycle_shell_input_owned_terminal_uses_readiness_and_restores_flags() {
    use std::io::{IsTerminal, Write};
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: valid descriptor output pointers; optional name/termios/winsize
    // pointers are null. Successful openpty creates two uniquely owned fds.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        },
        0
    );
    // SAFETY: each successful openpty output transfers exactly one owned fd.
    let (mut master, slave) = unsafe {
        (
            File::from(OwnedFd::from_raw_fd(master)),
            File::from(OwnedFd::from_raw_fd(slave)),
        )
    };
    assert!(slave.is_terminal());
    let observer = slave.try_clone().unwrap();
    let original = get_flags(observer.as_raw_fd()).unwrap();
    let mut input = Input::from_file(slave).unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), input.read_u8())
            .await
            .is_err()
    );
    master.write_all(b"6*7\n").unwrap();
    let mut line = [0; 4];
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        input.read_exact(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&line, b"6*7\n");
    input.finish().unwrap();
    assert_eq!(get_flags(observer.as_raw_fd()).unwrap(), original);
}

// The parent runs this exact test in an owned process with an idle stdin pipe.
// In the normal suite, its local fixture still exercises the real adapter.
#[tokio::test]
async fn lifecycle_shell_input_process_child() {
    let mut input = if std::env::var_os("IOS_TEST_OWNED_IDLE_STDIN").is_some() {
        Input::open().unwrap()
    } else {
        let (read, write) = pipe();
        let mut input = Input::from_file(read).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(40), input.read_u8())
                .await
                .is_err()
        );
        drop(write);
        input.finish().unwrap();
        return;
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(40), input.read_u8())
            .await
            .is_err()
    );
    input.finish().unwrap();
}

#[tokio::test]
async fn lifecycle_shell_input_runtime_exits_while_pipe_writer_remains_open() {
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cmd::webinspector::tasks::input::linux::tests::lifecycle_shell_input_process_child",
            "--nocapture",
        ])
        .env("IOS_TEST_OWNED_IDLE_STDIN", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let writer = child.stdin.take().unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(3), child.wait()).await;
    if status.is_err() {
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }
    assert!(status
        .expect("owned stdin child must exit before its writer closes")
        .unwrap()
        .success());
    drop(writer);
}
