use super::*;

fn remaining_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn output_preserves_binary_bytes_and_releases_candidate_files_before_replay() {
    let directory = tempfile::tempdir().unwrap();
    let mut output = CapturedOutput::create(directory.path()).unwrap();
    output
        .stdout
        .file
        .write_all(b"codex-cli 0.156.1\r\n\0\xff")
        .unwrap();
    output
        .stderr
        .file
        .write_all(b"\xfe\0warning\r\ntrailing bytes")
        .unwrap();

    let replay = output.seal(&mut || Ok(())).unwrap();

    assert_eq!(
        remaining_names(directory.path()),
        ["worker-output.stderr", "worker-output.stdout"]
    );
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    replay
        .replay(&mut stdout, &mut stderr, &mut || Ok(()))
        .unwrap();
    assert_eq!(stdout, b"codex-cli 0.156.1\r\n\0\xff");
    assert_eq!(stderr, b"\xfe\0warning\r\ntrailing bytes");
    assert!(remaining_names(directory.path()).is_empty());
}

#[test]
fn output_replays_stderr_above_stdout_limit_without_losing_tail() {
    let directory = tempfile::tempdir().unwrap();
    let mut output = CapturedOutput::create(directory.path()).unwrap();
    let expected = vec![0xa7; 1_048_593];
    output.stderr.file.write_all(&expected).unwrap();
    output.stderr.file.write_all(b"END\0\xff").unwrap();
    let replay = output.seal(&mut || Ok(())).unwrap();
    let mut stderr = Vec::new();

    replay
        .replay(&mut Vec::new(), &mut stderr, &mut || Ok(()))
        .unwrap();

    assert_eq!(&stderr[..1_048_593], expected);
    assert_eq!(&stderr[1_048_593..], b"END\0\xff");
    assert!(remaining_names(directory.path()).is_empty());
}

#[test]
fn output_over_limit_still_replays_stderr_and_never_succeeds() {
    let directory = tempfile::tempdir().unwrap();
    let mut output = CapturedOutput::create(directory.path()).unwrap();
    output.stdout.file.set_len(1_048_577).unwrap();
    output
        .stderr
        .file
        .write_all(b"original failure tail")
        .unwrap();
    let replay = output.seal(&mut || Ok(())).unwrap();
    let mut stderr = Vec::new();

    let failure = replay
        .replay(&mut Vec::new(), &mut stderr, &mut || Ok(()))
        .unwrap_err();

    assert_eq!(failure.to_string(), "版本探针 stdout 超出原有上限");
    assert_eq!(stderr, b"original failure tail");
    assert!(remaining_names(directory.path()).is_empty());
}

#[test]
fn output_cancelled_while_sealing_releases_every_owned_file() {
    let directory = tempfile::tempdir().unwrap();
    let mut output = CapturedOutput::create(directory.path()).unwrap();
    output
        .stdout
        .file
        .write_all(b"partial native output")
        .unwrap();
    output
        .stderr
        .file
        .write_all(b"\xff\0native failure\r\n")
        .unwrap();
    let mut checks = 0;
    let replay = output
        .seal(&mut || {
            checks += 1;
            // stdout 已经转存，随后取消仍不能把这部分输出回放到调用方。
            if checks < 3 {
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::Interrupted, "原取消"))
            }
        })
        .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let failure = replay
        .replay(&mut stdout, &mut stderr, &mut || Ok(()))
        .unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::Interrupted);
    assert_eq!(failure.to_string(), "原取消");
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
    assert_eq!(
        remaining_names(directory.path()),
        [CANCELLED_STDERR_RECEIPT]
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.path().join(CANCELLED_STDERR_RECEIPT)).unwrap())
            .unwrap();
    assert_eq!(receipt["total_bytes"], 18);
    assert_eq!(receipt["captured_bytes"], 18);
    assert_eq!(receipt["truncated"], false);
    assert_eq!(
        serde_json::from_value::<Vec<u8>>(receipt["prefix_bytes"].clone()).unwrap(),
        b"\xff\0native failure\r\n"
    );
    fs::remove_file(directory.path().join(CANCELLED_STDERR_RECEIPT)).unwrap();
    fs::rename(
        directory.path(),
        directory.path().with_extension("released"),
    )
    .unwrap();
    fs::remove_dir(directory.path().with_extension("released")).unwrap();
}

#[test]
fn output_cancelled_stderr_receipt_bounds_prefix_and_records_full_length() {
    let directory = tempfile::tempdir().unwrap();
    let mut output = CapturedOutput::create(directory.path()).unwrap();
    output.stderr.file.write_all(&[0xfe; 8192]).unwrap();
    output.stderr.file.write_all(b"excluded tail").unwrap();
    let replay = output
        .seal(&mut || Err(io::ErrorKind::Interrupted.into()))
        .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let failure = replay
        .replay(&mut stdout, &mut stderr, &mut || Ok(()))
        .unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::Interrupted);
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
    assert_eq!(
        remaining_names(directory.path()),
        [CANCELLED_STDERR_RECEIPT]
    );
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.path().join(CANCELLED_STDERR_RECEIPT)).unwrap())
            .unwrap();
    assert_eq!(receipt["total_bytes"], 8205);
    assert_eq!(receipt["captured_bytes"], 8192);
    assert_eq!(receipt["truncated"], true);
    assert_eq!(
        serde_json::from_value::<Vec<u8>>(receipt["prefix_bytes"].clone()).unwrap(),
        [0xfe; 8192]
    );
}

#[test]
fn output_cancelled_stderr_receipt_never_overwrites_an_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let mut output = CapturedOutput::create(directory.path()).unwrap();
    output
        .stderr
        .file
        .write_all(b"cancelled native stderr")
        .unwrap();
    fs::write(
        directory.path().join(CANCELLED_STDERR_RECEIPT),
        b"unrelated",
    )
    .unwrap();
    let replay = output
        .seal(&mut || Err(io::Error::new(io::ErrorKind::Interrupted, "原取消")))
        .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let failure = replay
        .replay(&mut stdout, &mut stderr, &mut || Ok(()))
        .unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::Interrupted);
    assert_eq!(failure.to_string(), "原取消");
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
    assert_eq!(
        remaining_names(directory.path()),
        [CANCELLED_STDERR_RECEIPT]
    );
    assert_eq!(
        fs::read(directory.path().join(CANCELLED_STDERR_RECEIPT)).unwrap(),
        b"unrelated"
    );
}

#[test]
fn output_cancelled_seal_preserves_prior_failure_without_replay() {
    let directory = tempfile::tempdir().unwrap();
    let mut output = CapturedOutput::create(directory.path()).unwrap();
    output.stdout.file.set_len(1_048_577).unwrap();
    output.stderr.file.write_all(b"original stderr").unwrap();
    let replay = output
        .seal(&mut || Err(io::ErrorKind::Interrupted.into()))
        .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    let failure = replay
        .replay(&mut stdout, &mut stderr, &mut || Ok(()))
        .unwrap_err();

    assert_eq!(failure.to_string(), "版本探针 stdout 超出原有上限");
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
    assert_eq!(
        remaining_names(directory.path()),
        [CANCELLED_STDERR_RECEIPT]
    );
}

#[test]
fn output_creation_failure_never_overwrites_an_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("candidate.stderr"),
        b"original other generation",
    )
    .unwrap();

    let failure = CapturedOutput::create(directory.path()).err().unwrap();

    assert_eq!(failure.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(remaining_names(directory.path()), ["candidate.stderr"]);
    assert_eq!(
        fs::read(directory.path().join("candidate.stderr")).unwrap(),
        b"original other generation"
    );
}

#[test]
fn output_seal_failure_releases_capture_but_preserves_existing_replay_file() {
    let directory = tempfile::tempdir().unwrap();
    let output = CapturedOutput::create(directory.path()).unwrap();
    fs::write(directory.path().join("worker-output.stderr"), b"unrelated").unwrap();

    let failure = output.seal(&mut || Ok(())).err().unwrap();

    assert_eq!(failure.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(remaining_names(directory.path()), ["worker-output.stderr"]);
    assert_eq!(
        fs::read(directory.path().join("worker-output.stderr")).unwrap(),
        b"unrelated"
    );
}

struct FailedWriter {
    fail_flush: bool,
}

impl Write for FailedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail_flush {
            Ok(bytes.len())
        } else {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::WriteZero.into())
    }
}

#[test]
fn output_write_failure_preserves_stderr_and_releases_replay_files() {
    let directory = tempfile::tempdir().unwrap();
    let mut output = CapturedOutput::create(directory.path()).unwrap();
    output.stdout.file.write_all(b"version\n").unwrap();
    output.stderr.file.write_all(b"stderr tail").unwrap();
    let replay = output.seal(&mut || Ok(())).unwrap();
    let mut stderr = Vec::new();

    let failure = replay
        .replay(
            &mut FailedWriter { fail_flush: false },
            &mut stderr,
            &mut || Ok(()),
        )
        .unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(stderr, b"stderr tail");
    assert!(remaining_names(directory.path()).is_empty());
}

#[test]
fn output_flush_failure_is_not_reported_as_success() {
    let directory = tempfile::tempdir().unwrap();
    let output = CapturedOutput::create(directory.path()).unwrap();
    let replay = output.seal(&mut || Ok(())).unwrap();

    let failure = replay
        .replay(
            &mut Vec::new(),
            &mut FailedWriter { fail_flush: true },
            &mut || Ok(()),
        )
        .unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::WriteZero);
    assert!(remaining_names(directory.path()).is_empty());
}
