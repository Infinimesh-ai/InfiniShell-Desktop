use super::*;
use std::fs::File;
use std::io::Read;
use std::sync::{Arc, Barrier};
use std::thread;

fn pipe() -> (File, OwnedFd, String) {
    let mut pair = [-1; 2];
    assert_eq!(
        unsafe { libc::pipe2(pair.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
        0
    );
    let reader = unsafe { File::from_raw_fd(pair[0]) };
    let writer = unsafe { OwnedFd::from_raw_fd(pair[1]) };
    let mut info = MaybeUninit::<libc::stat>::uninit();
    assert_eq!(unsafe { libc::fstat(pair[1], info.as_mut_ptr()) }, 0);
    let info = unsafe { info.assume_init() };
    let value = format!("v1:{}:{}:{}", info.st_dev, info.st_ino, "ab".repeat(16));
    (reader, writer, value)
}

#[test]
fn trace_requires_the_exact_nonblocking_write_pipe_identity() {
    let (reader, writer, value) = pipe();
    assert_eq!(validate_pipe(writer.as_raw_fd(), &value), Some([0xab; 16]));
    assert!(validate_pipe(reader.as_raw_fd(), &value).is_none());
    let (_, other, _) = pipe();
    assert!(validate_pipe(other.as_raw_fd(), &value).is_none());
    assert_eq!(
        unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, 0) },
        0
    );
    assert!(validate_pipe(writer.as_raw_fd(), &value).is_none());
}

#[test]
fn trace_rejects_ambiguous_authorization_without_echoing_it() {
    for value in [
        "",
        "v2:1:2:abcd",
        "v1:1:2:ABCDEFABCDEFABCDEFABCDEFABCDEFABCD",
        "v1:1:2:abababababababababababababababab:extra",
    ] {
        assert!(authorization(value).is_none());
    }
}

#[test]
fn trace_authorization_requires_guarded_send_and_the_inherited_pipe_together() {
    let (_reader, writer, value) = pipe();
    let arguments = ["cli-agent-notify", "--require-protocol", "1"];
    assert_eq!(
        authorized_pipe(
            arguments.map(OsString::from).into_iter(),
            &value,
            writer.as_raw_fd()
        ),
        Some([0xab; 16])
    );
    for arguments in [
        vec![],
        vec!["cli-agent-notify"],
        vec!["cli-agent-notify", "--protocol-version"],
        vec!["cli-agent-notify", "--require-protocol", "2"],
        vec!["cli-agent-notify", "--require-protocol", "1", "extra"],
        vec!["other", "--require-protocol", "1"],
    ] {
        assert!(
            authorized_pipe(
                arguments.into_iter().map(OsString::from),
                &value,
                writer.as_raw_fd()
            )
            .is_none()
        );
    }
    assert!(
        authorized_pipe(
            arguments.map(OsString::from).into_iter(),
            "",
            writer.as_raw_fd()
        )
        .is_none()
    );
}

#[test]
fn trace_records_only_fixed_fields_and_stops_at_the_record_budget() {
    let (mut reader, writer, _) = pipe();
    let mut trace = Trace {
        descriptor: writer,
        nonce: [0xab; 16],
        sequence: 0,
        active: true,
    };
    for _ in 0..MAX_RECORDS + 4 {
        trace.emit(Stage::Worker);
    }
    drop(trace);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes.len(), RECORD_SIZE * MAX_RECORDS as usize);
    let mut previous = 0;
    for (index, row) in bytes.chunks_exact(RECORD_SIZE).enumerate() {
        assert_eq!(&row[..4], b"INW1");
        assert_eq!(row[4], Stage::Worker as u8);
        assert_eq!(&row[5..8], &[0; 3]);
        assert_eq!(&row[8..12], &(index as u32).to_le_bytes());
        assert_eq!(&row[12..16], &std::process::id().to_le_bytes());
        let timestamp = u64::from_le_bytes(row[16..24].try_into().unwrap());
        assert!(timestamp >= previous);
        previous = timestamp;
        assert_eq!(&row[24..40], &[0xab; 16]);
        assert_eq!(&row[40..], &[0; 8]);
    }
}

#[test]
fn trace_full_pipe_disables_recording_without_waiting_or_retrying() {
    let (_reader, writer, _) = pipe();
    let fill = [0; 4096];
    while unsafe { libc::write(writer.as_raw_fd(), fill.as_ptr().cast(), fill.len()) } > 0 {}
    assert_eq!(
        std::io::Error::last_os_error().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let mut trace = Trace {
        descriptor: writer,
        nonce: [0; 16],
        sequence: 0,
        active: true,
    };
    trace.emit(Stage::Worker);
    assert!(!trace.active);
    assert_eq!(trace.sequence, 1);
    trace.emit(Stage::InputBegin);
    assert_eq!(trace.sequence, 1);
}

#[test]
fn trace_closed_pipe_disables_recording_without_changing_sigpipe_policy() {
    let (reader, writer, value) = pipe();
    // 复用初始化的只读信号检查；测试也不更改进程信号处理器。
    let nonce = authorized_pipe(
        ["cli-agent-notify", "--require-protocol", "1"]
            .map(OsString::from)
            .into_iter(),
        &value,
        writer.as_raw_fd(),
    )
    .unwrap();
    drop(reader);
    let mut trace = Trace {
        descriptor: writer,
        nonce,
        sequence: 0,
        active: true,
    };
    trace.emit(Stage::Worker);
    assert!(!trace.active);
    assert_eq!(trace.sequence, 1);
}

#[test]
fn trace_concurrent_phases_remain_whole_ordered_records_or_are_skipped() {
    let (mut reader, writer, _) = pipe();
    let trace = Arc::new(Mutex::new(Trace {
        descriptor: writer,
        nonce: [0xab; 16],
        sequence: 0,
        active: true,
    }));
    let barrier = Arc::new(Barrier::new(4));
    let threads = (0..4)
        .map(|_| {
            let trace = Arc::clone(&trace);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for _ in 0..4 {
                    if let Ok(mut trace) = trace.try_lock() {
                        trace.emit(Stage::Worker);
                    }
                }
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    drop(trace);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert!(!bytes.is_empty());
    assert!(bytes.len() <= 16 * RECORD_SIZE);
    assert_eq!(bytes.len() % RECORD_SIZE, 0);
    for (index, record) in bytes.chunks_exact(RECORD_SIZE).enumerate() {
        assert_eq!(&record[..4], b"INW1");
        assert_eq!(&record[8..12], &(index as u32).to_le_bytes());
        assert_eq!(&record[24..40], &[0xab; 16]);
    }
}
