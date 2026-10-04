use std::fs::{self, OpenOptions};
use std::io::{self, Read as _, Seek as _, Write as _};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, BorrowedHandle, FromRawHandle as _};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use command::blocking::Command as BlockingCommand;
use command::managed::{Containment, ManagedTree};
use command::windows::SuspendedChild;
use windows::Win32::Foundation::NTSTATUS;
use windows::Win32::Storage::FileSystem::{
    FILE_TYPE_CHAR, FILE_TYPE_DISK, FILE_TYPE_PIPE, GetFileType,
};
use windows::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::LibraryLoader::LoadLibraryW;
use windows::Win32::System::Threading::{
    DEBUG_PROCESS, GetCurrentProcessId, GetProcessId, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::core::{Error as WindowsError, PCWSTR};

use super::*;

struct Fixture {
    directory: tempfile::TempDir,
    install: PathBuf,
    bin: PathBuf,
    program: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("install");
        let bin = install.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let program = bin.join("updater.exe");
        fs::write(&program, minimal_pe()).unwrap();
        Self {
            directory,
            install,
            bin,
            program,
        }
    }

    fn expected(&self) -> ExpectedFileIdentity {
        ExpectedFileIdentity::capture(&self.program).unwrap()
    }
}

const TEST_PE_OFFSET: usize = 0x80;
const TEST_HEADERS_SIZE: usize = 0x200;
const TEST_SECTION_RVA: u32 = 0x1000;
const TEST_SECTION_OFFSET: usize = 0x200;
const DYNAMIC_IMAGE_FIXTURE_ENV: &str = "INFINISHELL_WINDOWS_DYNAMIC_IMAGE_FIXTURE";
const DYNAMIC_IMAGE_MARKER_ENV: &str = "INFINISHELL_WINDOWS_DYNAMIC_IMAGE_MARKER";
const REAL_CLI_FIXTURE_ENV: &str = "INFINISHELL_WINDOWS_REAL_CLI_FIXTURE";
const REAL_CLI_ARGS_ENV: &str = "INFINISHELL_WINDOWS_REAL_CLI_ARGS";
const REAL_CLI_ENV_ENV: &str = "INFINISHELL_WINDOWS_REAL_CLI_ENV";
const REAL_CLI_TARGET_ENV: &str = "INFINISHELL_WINDOWS_REAL_CLI_TARGET";
const DEBUG_DRIVER_ENV: &str = "INFINISHELL_WINDOWS_ATOMIC_DEBUG_DRIVER";
const DEBUG_DRIVER_RECEIPT_ENV: &str = "INFINISHELL_WINDOWS_ATOMIC_DEBUG_RECEIPT";
const NODE_CONTROL_ENV: &str = "INFINISHELL_WINDOWS_NODE_CONTROL";
const NODE_CONTROL_SHA256_ENV: &str = "INFINISHELL_WINDOWS_NODE_CONTROL_SHA256";
const NODE_CONTROL_OUTPUT_ENV: &str = "INFINISHELL_WINDOWS_NODE_CONTROL_OUTPUT";
const DEBUG_DRIVER_TIMEOUT: Duration = Duration::from_secs(30);
const DEBUG_LARGE_IMAGE_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) fn record_rejected_image(path: &Path, system_directory: &File) {
    // 仅实机诊断测试输出文件名和位置类别；不输出完整路径，也不把类别作为信任依据。
    let basename = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.len() <= 128
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        });
    let system_path = final_path_from_handle(system_directory).ok();
    let relative = system_path
        .as_ref()
        .and_then(|system| system.parent())
        .and_then(|root| path.strip_prefix(root).ok());
    let category = match relative
        .and_then(|path| path.components().next())
        .and_then(|part| part.as_os_str().to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("microsoft.net") => "windows_microsoft_net",
        Some("winsxs") => "windows_winsxs",
        Some("system32") => "windows_system32_subdirectory",
        Some(_) => "windows_other",
        None => "outside_windows_or_unresolved",
    };
    eprintln!(
        "atomic_windows_rejected_image={}",
        serde_json::json!({ "basename": basename, "directory_category": category })
    );
}

fn run_debug_fixture_in_strict_job(test_name: &str, timeout: Duration) {
    run_debug_fixture_in_strict_job_with_stdin(test_name, timeout, false);
}

fn run_debug_fixture_in_strict_job_with_stdin(
    test_name: &str,
    timeout: Duration,
    keep_stdin_open: bool,
) {
    let (_, test_module) = module_path!()
        .split_once("::")
        .expect("测试模块路径应包含 crate 名称");
    let exact_name = format!("{test_module}::{test_name}");
    let directory = tempfile::tempdir().unwrap();
    let receipt = directory.path().join("native-exit.txt");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            &exact_name,
            "--nocapture",
            "--test-threads=1",
        ])
        .env(DEBUG_DRIVER_ENV, "1")
        .env(DEBUG_DRIVER_RECEIPT_ENV, &receipt)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .inherit_managed_job();
    let driver = command.spawn().unwrap();
    // 子测试先等待授权，父测试可在任何调试事件发生前为整棵树建立严格 Job。
    let mut tree = ManagedTree::claim(driver).unwrap();
    assert_eq!(tree.containment(), Containment::WindowsJob);
    let mut input = tree.child_mut().stdin.take().unwrap();
    let authorization = input.write_all(b"1");
    // 旧夹具默认立即发出 EOF；新对照可在整个原生测试期间保留写端。
    let held_input = if keep_stdin_open {
        Some(input)
    } else {
        drop(input);
        None
    };
    if let Err(failure) = authorization {
        drop(held_input);
        let cleanup = tree.terminate_and_confirm(Duration::from_secs(5));
        panic!("调试夹具授权失败：{failure}；严格 Job 清理：{cleanup:?}");
    }

    let started = Instant::now();
    let deadline = started + timeout;
    let outcome = loop {
        match tree.root_exited() {
            Ok(true) => break Ok(false),
            Ok(false) => {}
            Err(failure) => break Err(failure),
        }
        if Instant::now() >= deadline {
            break Ok(true);
        }
        thread::sleep(Duration::from_millis(20));
    };
    // 即使调试事件链挂住，也须终止专属 Job、等待测试根进程并确认 ActiveProcesses=0。
    drop(held_input);
    let status = tree.terminate_and_confirm(Duration::from_secs(5)).unwrap();
    eprintln!("atomic_windows_strict_job_cleanup_confirmed=true");
    let timed_out = outcome.unwrap();
    eprintln!(
        "Windows 调试夹具：测试={test_name}，外层耗时={}ms，超时={timed_out}，严格 Job 已清空",
        started.elapsed().as_millis()
    );
    assert!(
        !timed_out,
        "调试链 {test_name} 在 {} 秒内没有原生退出；严格 Job 已清空，测试根进程：{status}",
        timeout.as_secs()
    );
    assert!(
        status.success(),
        "调试链 {test_name} 失败；严格 Job 已清空，测试根进程：{status}"
    );
    assert_eq!(fs::read_to_string(receipt).unwrap(), test_name);
}

fn run_debug_stdio_fixture_in_strict_job(test_name: &str, case: &str, expected_line: &str) {
    const OUTPUT_LIMIT: usize = 64 * 1024;
    let (_, test_module) = module_path!().split_once("::").unwrap();
    let exact_name = format!("{test_module}::{test_name}");
    let directory = tempfile::tempdir().unwrap();
    let receipt = directory.path().join("native-exit.txt");
    let stderr_path = directory.path().join("driver.stderr");
    let stderr_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&stderr_path)
        .unwrap();
    let output_root = std::env::var_os(NODE_CONTROL_OUTPUT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            tempfile::Builder::new()
                .prefix("infinishell-node-control-evidence-")
                .tempdir()
                .unwrap()
                .keep()
        });
    fs::create_dir_all(&output_root).unwrap();
    let output = output_root.join(case);
    fs::create_dir(&output).expect("stdio 对照证据目录必须全新，不能覆盖旧证据");
    let stderr_kind = debug_stdio_kind(HANDLE(stderr_file.as_raw_handle()));
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            &exact_name,
            "--nocapture",
            "--test-threads=1",
            "--format=terse",
        ])
        .env(DEBUG_DRIVER_ENV, "1")
        .env(DEBUG_DRIVER_RECEIPT_ENV, &receipt)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr_file))
        .inherit_managed_job();
    let mut tree = ManagedTree::claim(command.spawn().unwrap()).unwrap();
    assert_eq!(tree.containment(), Containment::WindowsJob);
    let mut input = tree.child_mut().stdin.take().unwrap();
    let mut stdout = tree.child_mut().stdout.take().unwrap();
    let stdin_kind = debug_stdio_kind(HANDLE(input.as_raw_handle()));
    let stdout_kind = debug_stdio_kind(HANDLE(stdout.as_raw_handle()));
    let stdout_exceeded = Arc::new(AtomicBool::new(false));
    let reader_exceeded = stdout_exceeded.clone();
    let (capture_sender, capture_receiver) = mpsc::channel();
    // 始终排空管道，只保留前 64 KiB；超量同时通知父线程终止本次严格 Job。
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 4096];
        let failure = loop {
            match stdout.read(&mut buffer) {
                Ok(0) => break None,
                Ok(count) => {
                    total = total.saturating_add(count as u64);
                    let keep = count.min(OUTPUT_LIMIT.saturating_sub(bytes.len()));
                    bytes.extend_from_slice(&buffer[..keep]);
                    if keep != count {
                        reader_exceeded.store(true, Ordering::Release);
                    }
                }
                Err(failure) if failure.kind() == io::ErrorKind::Interrupted => {}
                Err(failure) => break Some(format!("{:?}", failure.kind())),
            }
        };
        let _ = capture_sender.send((bytes, total, failure));
    });
    let mut stderr_exceeded = false;
    let deadline = Instant::now() + DEBUG_LARGE_IMAGE_TIMEOUT;
    let outcome = input.write_all(b"1").and_then(|()| {
        loop {
            stderr_exceeded = fs::metadata(&stderr_path)?.len() > OUTPUT_LIMIT as u64;
            if stderr_exceeded || stdout_exceeded.load(Ordering::Acquire) {
                break Ok(false);
            }
            if tree.root_exited()? {
                break Ok(false);
            }
            if Instant::now() >= deadline {
                break Ok(true);
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    // stdin 写端一直保留到 Job 收尾，不能用提前 EOF 掩盖原生进程挂起。
    let status = tree.terminate_and_confirm(Duration::from_secs(5));
    drop(input);
    let captured = capture_receiver.recv_timeout(Duration::from_secs(5));
    let reader_completed = captured.is_ok() && reader.join().is_ok();
    let (stdout_bytes, stdout_total, stdout_failure) =
        captured.unwrap_or_else(|failure| (Vec::new(), 0, Some(format!("{failure:?}"))));
    let stderr_total = fs::metadata(&stderr_path).unwrap().len();
    stderr_exceeded |= stderr_total > OUTPUT_LIMIT as u64;
    let mut stderr_bytes = Vec::new();
    File::open(&stderr_path)
        .unwrap()
        .take(OUTPUT_LIMIT as u64)
        .read_to_end(&mut stderr_bytes)
        .unwrap();
    let expected_line_count = String::from_utf8_lossy(&stdout_bytes)
        .lines()
        .filter(|line| *line == expected_line)
        .count();
    let native_receipt_matches = fs::read_to_string(&receipt).is_ok_and(|value| value == test_name);
    let summary = serde_json::json!({
        "case": case,
        "stdio_scope": "npm_acceptance_fixture",
        "stdin_kind": stdin_kind,
        "stdout_kind": stdout_kind,
        "stderr_kind": stderr_kind,
        "stdin_held_until_job_cleanup": true,
        "timed_out": outcome.as_ref().ok().copied(),
        "driver_failure_kind": outcome.as_ref().err().map(|failure| format!("{:?}", failure.kind())),
        "driver_exit_code": status.as_ref().ok().and_then(|status| status.code()),
        "job_cleanup_confirmed": status.is_ok(),
        "native_receipt_matches": native_receipt_matches,
        "stdout_total_bytes": stdout_total,
        "stderr_total_bytes": stderr_total,
        "stdout_truncated": stdout_exceeded.load(Ordering::Acquire),
        "stderr_truncated": stderr_exceeded,
        "stdout_read_failure": stdout_failure,
        "stdout_reader_completed": reader_completed,
        "expected_stdout_line": expected_line,
        "expected_stdout_line_count": expected_line_count,
    });
    eprintln!("atomic_windows_cmd_stdio_driver={summary}");
    eprintln!(
        "atomic_windows_cmd_stdio_stderr_begin case={case}\n{}\natomic_windows_cmd_stdio_stderr_end case={case}",
        String::from_utf8_lossy(&stderr_bytes)
    );
    let summary_bytes = serde_json::to_vec_pretty(&summary).unwrap();
    for (name, bytes) in [
        ("control.stdout", stdout_bytes.as_slice()),
        ("control.stderr", stderr_bytes.as_slice()),
        ("control.safe.json", summary_bytes.as_slice()),
    ] {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(name))
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
    assert_eq!(
        (stdin_kind, stdout_kind, stderr_kind),
        ("pipe", "pipe", "disk")
    );
    assert!(
        status.unwrap().success(),
        "stdio 对照子测试失败，证据已保留"
    );
    assert!(!outcome.unwrap(), "stdio 对照超时，证据已保留");
    assert!(!stdout_exceeded.load(Ordering::Acquire));
    assert!(!stderr_exceeded);
    assert!(reader_completed);
    assert!(stdout_failure.is_none());
    assert!(native_receipt_matches);
    assert_eq!(expected_line_count, 1);
}

fn debug_stdio_kind(handle: HANDLE) -> &'static str {
    match unsafe { GetFileType(handle) } {
        FILE_TYPE_PIPE => "pipe",
        FILE_TYPE_DISK => "disk",
        FILE_TYPE_CHAR => "char",
        _kind => "unknown",
    }
}

fn await_debug_driver_authorization() {
    let mut authorization = [0];
    std::io::stdin().read_exact(&mut authorization).unwrap();
    assert_eq!(authorization, [b'1']);
}

fn spawn_debuggee(command: &mut Command) -> SuspendedChild {
    match command.spawn_suspended_with_child_on_error() {
        Ok(suspended) => suspended,
        Err((failure, child)) => {
            if let Some(mut child) = child {
                let _ = child.kill();
            }
            panic!("调试夹具挂起派生失败，外层严格 Job 负责整树清理：{failure}");
        }
    }
}

fn resume_debuggee(suspended: SuspendedChild) -> std::process::Child {
    match suspended.resume_with_child_on_error() {
        Ok(child) => child,
        Err((failure, mut child)) => {
            let _ = child.kill();
            panic!("调试夹具恢复主线程失败，外层严格 Job 负责整树清理：{failure}");
        }
    }
}

fn record_debug_native_exit(test_name: &str) {
    let receipt = std::env::var_os(DEBUG_DRIVER_RECEIPT_ENV).unwrap();
    fs::write(receipt, test_name).unwrap();
}

fn minimal_pe() -> Vec<u8> {
    let (machine, magic, optional_size, directory_offset) = if cfg!(target_arch = "x86") {
        (0x014c_u16, 0x010b_u16, 224_u16, 96_usize)
    } else if cfg!(target_arch = "x86_64") {
        (0x8664_u16, 0x020b_u16, 240_u16, 112_usize)
    } else {
        (0xaa64_u16, 0x020b_u16, 240_u16, 112_usize)
    };
    minimal_pe_for(machine, magic, optional_size, directory_offset)
}

fn minimal_pe_for(
    machine: u16,
    magic: u16,
    optional_size: u16,
    directory_offset: usize,
) -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x400];
    bytes[..2].copy_from_slice(b"MZ");
    put_u32(
        &mut bytes,
        DOS_HEADER_PE_OFFSET as usize,
        TEST_PE_OFFSET as u32,
    );
    bytes[TEST_PE_OFFSET..TEST_PE_OFFSET + 4].copy_from_slice(b"PE\0\0");
    put_u16(&mut bytes, TEST_PE_OFFSET + 4, machine);
    put_u16(&mut bytes, TEST_PE_OFFSET + 6, 1);
    put_u16(&mut bytes, TEST_PE_OFFSET + 20, optional_size);
    put_u16(&mut bytes, TEST_PE_OFFSET + 22, 0x0002);

    let optional = TEST_PE_OFFSET + PE_SIGNATURE_AND_FILE_HEADER_BYTES as usize;
    put_u16(&mut bytes, optional, magic);
    put_u32(&mut bytes, optional + 16, TEST_SECTION_RVA);
    put_u32(&mut bytes, optional + 32, 0x1000);
    put_u32(&mut bytes, optional + 36, 0x200);
    put_u32(&mut bytes, optional + 56, 0x2000);
    put_u32(&mut bytes, optional + 60, TEST_HEADERS_SIZE as u32);
    put_u16(&mut bytes, optional + 68, 3);
    put_u32(&mut bytes, optional + directory_offset - 4, 16);

    let section = optional + usize::from(optional_size);
    bytes[section..section + 5].copy_from_slice(b".text");
    put_u32(&mut bytes, section + 8, 0x200);
    put_u32(&mut bytes, section + 12, TEST_SECTION_RVA);
    put_u32(&mut bytes, section + 16, 0x200);
    put_u32(&mut bytes, section + 20, TEST_SECTION_OFFSET as u32);
    put_u32(&mut bytes, section + 36, 0x6000_0020);
    bytes
}

fn set_directory(bytes: &mut [u8], index: u32, rva: u32, size: u32) {
    let optional = TEST_PE_OFFSET + PE_SIGNATURE_AND_FILE_HEADER_BYTES as usize;
    let magic = u16::from_le_bytes(bytes[optional..optional + 2].try_into().unwrap());
    let directory_offset = if magic == 0x010b { 96 } else { 112 };
    let entry = optional + directory_offset + index as usize * 8;
    put_u32(bytes, entry, rva);
    put_u32(bytes, entry + 4, size);
}

fn set_import_name(bytes: &mut [u8], name: &[u8]) {
    let offset = TEST_SECTION_OFFSET + 0x140;
    bytes[offset..offset + name.len()].copy_from_slice(name);
    bytes[offset + name.len()] = 0;
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn verify_pe_bytes(bytes: &[u8]) -> io::Result<()> {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(bytes).unwrap();
    file.rewind().unwrap();
    require_pe(&mut file, bytes.len() as u64)
}

fn create_directory_junction(target: &Path, link: &Path) {
    let command_processor = std::env::var_os("COMSPEC").expect("Windows 必须提供 COMSPEC");
    let output = BlockingCommand::new(command_processor)
        .arg("/D")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(link)
        .arg(target)
        .output()
        .expect("Windows 验收机必须能运行 mklink /J");
    assert!(
        output.status.success(),
        "Windows 验收机必须允许创建测试 junction：{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reparse_program_is_rejected() {
    let fixture = Fixture::new();
    let link = fixture.bin.join("linked-updater.exe");
    create_directory_junction(&fixture.bin, &link);
    let mut expected = fixture.expected();
    expected.path = link.clone();
    expected.canonical_path = link;

    assert!(prepare(&expected).is_err());
}

#[test]
fn reparse_ancestor_is_rejected() {
    let fixture = Fixture::new();
    let linked_bin = fixture.directory.path().join("linked-bin");
    create_directory_junction(&fixture.bin, &linked_bin);
    let linked_program = linked_bin.join("updater.exe");
    let mut expected = fixture.expected();
    expected.path = linked_program.clone();
    expected.canonical_path = linked_program;

    assert!(prepare(&expected).is_err());
}

#[test]
fn same_content_replacement_with_new_file_id_is_rejected() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let old = fixture.bin.join("old.exe");
    fs::rename(&fixture.program, &old).unwrap();
    fs::copy(&old, &fixture.program).unwrap();

    assert!(prepare(&expected).is_err());
}

#[test]
fn lease_rejects_program_write_delete_and_rename() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let renamed = fixture.bin.join("renamed.exe");

    assert!(
        OpenOptions::new()
            .write(true)
            .open(&fixture.program)
            .is_err()
    );
    assert!(fs::remove_file(&fixture.program).is_err());
    assert!(fs::rename(&fixture.program, &renamed).is_err());

    drop(lease);
}

#[test]
fn lease_rejects_ancestor_rename_until_release_then_restores_operations() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let moved_install = fixture.directory.path().join("moved-install");

    assert!(fs::rename(&fixture.install, &moved_install).is_err());
    drop(lease);
    OpenOptions::new()
        .write(true)
        .open(&fixture.program)
        .unwrap()
        .write_all(b"")
        .unwrap();
    let renamed_program = fixture.bin.join("renamed.exe");
    fs::rename(&fixture.program, &renamed_program).unwrap();
    fs::rename(&renamed_program, &fixture.program).unwrap();
    fs::rename(&fixture.install, &moved_install).unwrap();
    let moved_program = moved_install.join("bin/updater.exe");
    fs::remove_file(&moved_program).unwrap();
}

#[test]
fn mismatched_command_is_rejected_before_suspended_spawn() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let marker = fixture.directory.path().join("started.txt");
    let command_processor = std::env::var_os("COMSPEC").expect("Windows 必须提供 COMSPEC");
    let mut command = Command::new(command_processor);
    command
        .arg("/C")
        .arg(format!("type nul > \"{}\"", marker.display()))
        .inherit_managed_job();

    let mut lease = lease;
    assert!(lease.verify_command_for_suspended_spawn(&command).is_err());
    assert!(!marker.exists());
}

#[test]
fn matching_project_command_is_ready_for_suspended_spawn_without_starting() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let mut lease = prepare(&expected).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("started.txt");
    let mut command = Command::new(lease.execution_path());
    command
        .arg("/D")
        .arg("/C")
        .arg(format!("type nul > \"{}\"", marker.display()))
        .inherit_managed_job();

    lease.verify_command_for_suspended_spawn(&command).unwrap();

    assert!(!marker.exists());
}

#[test]
fn windows_program_with_bounded_imports_is_structurally_accepted() {
    let command_processor =
        PathBuf::from(std::env::var_os("COMSPEC").expect("Windows 必须提供带系统依赖的 COMSPEC"));
    let expected = ExpectedFileIdentity::capture(&command_processor).unwrap();

    prepare(&expected).unwrap();
}

#[test]
#[ignore = "Windows 实机退出闭包必须通过显式诊断入口验证"]
fn debug_session_runs_system_only_process_to_native_exit() {
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(
            "debug_session_runs_system_only_process_to_native_exit",
            DEBUG_DRIVER_TIMEOUT,
        );
        return;
    }
    await_debug_driver_authorization();
    let command_processor =
        PathBuf::from(std::env::var_os("COMSPEC").expect("Windows 必须提供 COMSPEC"));
    let expected = ExpectedFileIdentity::capture(&command_processor).unwrap();
    let mut executable = prepare(&expected).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let cwd_identity = AtomicDirectoryIdentity::capture(directory.path()).unwrap();
    let cwd = prepare_directory(&cwd_identity).unwrap();
    let mut command = Command::new(executable.execution_path());
    command
        .arg("/D")
        .arg("/C")
        .arg("exit 0")
        .current_dir(cwd.execution_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .inherit_managed_job()
        .creation_flags(DEBUG_PROCESS.0);
    executable
        .verify_command_for_suspended_spawn(&command)
        .unwrap();
    let suspended = spawn_debuggee(&mut command);
    let root_process_id = suspended.id();
    let mut child = resume_debuggee(suspended);
    eprintln!("Windows 调试夹具：系统命令主线程已恢复，等待根映像初始事件");
    let mut debug = executable
        .begin_image_debug_session(root_process_id)
        .unwrap();
    eprintln!("Windows 调试夹具：根映像初始事件已确认并继续");
    drop(cwd);

    let status = debug.wait_for_exit(&mut child).unwrap();
    eprintln!("Windows 调试夹具：根进程及调试子树已退出，原生状态：{status}");

    assert!(status.success());
    record_debug_native_exit("debug_session_runs_system_only_process_to_native_exit");
}

#[test]
#[ignore = "Windows 实机调试事件退出闭包尚未收敛，仅可按显式收据流程运行"]
fn debug_session_runs_fixed_real_cli_to_native_exit() {
    let Some(executable_path) = std::env::var_os(REAL_CLI_FIXTURE_ENV).map(PathBuf::from) else {
        return;
    };
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(
            "debug_session_runs_fixed_real_cli_to_native_exit",
            Duration::from_secs(6 * 60),
        );
        return;
    }
    await_debug_driver_authorization();
    let requested_args = std::env::var(REAL_CLI_ARGS_ENV).ok();
    let args: Vec<String> = requested_args
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .unwrap()
        .unwrap_or_else(|| vec!["--version".to_owned()]);
    let environment: std::collections::BTreeMap<String, String> = std::env::var(REAL_CLI_ENV_ENV)
        .ok()
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .unwrap()
        .unwrap_or_default();
    let expected = ExpectedFileIdentity::capture(&executable_path).unwrap();
    let mut executable = prepare(&expected).unwrap();
    if let Some(target) = std::env::var(REAL_CLI_TARGET_ENV).ok() {
        executable
            .set_child_image(Some(serde_json::from_str(&target).unwrap()))
            .unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    for relative in [
        "home",
        "home/AppData/Roaming",
        "home/AppData/Local",
        "home/.claude",
        "home/.codex",
        "grok",
        "tmp",
    ] {
        fs::create_dir_all(directory.path().join(relative)).unwrap();
    }
    let cwd_identity = AtomicDirectoryIdentity::capture(directory.path()).unwrap();
    let cwd = prepare_directory(&cwd_identity).unwrap();
    let mut command = Command::new(executable.execution_path());
    command
        .args(args)
        .current_dir(cwd.execution_path())
        .env_clear()
        .envs(super::super::isolated_environment(directory.path()))
        .envs(environment)
        .stdin(Stdio::null())
        // 固定 CLI 运行器将此 ignored case 的输出写入私有日志，只导出安全标记。
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .inherit_managed_job()
        .creation_flags(DEBUG_PROCESS.0);
    executable
        .verify_command_for_suspended_spawn(&command)
        .unwrap();
    let suspended = spawn_debuggee(&mut command);
    let root_process_id = suspended.id();
    let mut child = resume_debuggee(suspended);
    eprintln!("Windows 调试夹具：真实 CLI 主线程已恢复，等待根映像初始事件");
    let mut debug = executable
        .begin_image_debug_session(root_process_id)
        .unwrap_or_else(|failure| {
            eprintln!(
                "atomic_windows_debug_failure={}",
                debug_failure_fields("begin_session", &failure)
            );
            panic!("Windows 原子调试失败：begin_session");
        });
    eprintln!("Windows 调试夹具：真实 CLI 根映像初始事件已确认并继续");
    drop(cwd);

    let status = debug.wait_for_exit(&mut child).unwrap_or_else(|failure| {
        eprintln!(
            "atomic_windows_debug_failure={}",
            debug_failure_fields("wait_for_exit", &failure)
        );
        panic!("Windows 原子调试失败：wait_for_exit");
    });
    eprintln!("Windows 调试夹具：真实 CLI 根进程及调试子树已退出，原生状态：{status}");

    if requested_args.is_none() {
        assert!(status.success());
    } else {
        eprintln!("真实更新参数的原生退出状态：{status}");
    }
    record_debug_native_exit("debug_session_runs_fixed_real_cli_to_native_exit");
}

fn debug_failure_fields(stage: &'static str, failure: &io::Error) -> serde_json::Value {
    // 只保存固定错误标识及系统数值，不输出可能包含路径的错误正文。
    let description = failure.to_string();
    let code = description.split(':').next().filter(|code| {
        code.starts_with("managed_process.atomic_windows_")
            && code.len() <= 128
            && code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || matches!(byte, b'_' | b'.'))
    });
    let hresult = failure
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<WindowsError>())
        .map(|error| error.code().0);
    serde_json::json!({"stage": stage, "os_error": failure.raw_os_error(),
        "hresult": hresult, "failure_code": code})
}

#[test]
fn debug_failure_fields_keep_os_code_without_private_error_text() {
    let native = debug_failure_fields("begin_session", &io::Error::from_raw_os_error(5));
    assert_eq!(native["os_error"], 5);
    let private = debug_failure_fields(
        "wait_for_exit",
        &io::Error::other("C:\\private\\config.json"),
    );
    assert!(private["os_error"].is_null());
    assert!(private["failure_code"].is_null());
    assert!(!private.to_string().contains("private"));
    let code = "managed_process.atomic_windows_loaded_image_outside_system_directory";
    assert_eq!(
        debug_failure_fields("wait_for_exit", &io::Error::other(code))["failure_code"],
        code
    );
}

#[test]
fn non_system_dynamic_image_fixture() {
    let Some(path) = std::env::var_os(DYNAMIC_IMAGE_FIXTURE_ENV) else {
        return;
    };
    let marker = PathBuf::from(
        std::env::var_os(DYNAMIC_IMAGE_MARKER_ENV).expect("动态映像夹具必须提供 marker"),
    );
    let mut wide: Vec<_> = Path::new(&path).as_os_str().encode_wide().collect();
    wide.push(0);
    unsafe { LoadLibraryW(PCWSTR(wide.as_ptr())) }.expect("未受监督运行时应能加载测试映像");
    fs::write(marker, b"loaded").unwrap();
}

#[test]
#[ignore = "Windows 实机拒绝路径必须通过显式诊断入口验证"]
fn debug_session_rejects_non_system_dynamic_image_before_continue() {
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(
            "debug_session_rejects_non_system_dynamic_image_before_continue",
            DEBUG_LARGE_IMAGE_TIMEOUT,
        );
        return;
    }
    await_debug_driver_authorization();
    let executable_path = std::env::current_exe().unwrap();
    let executable_bytes = fs::metadata(&executable_path).unwrap().len();
    eprintln!("Windows DLL 拒绝诊断：测试二进制大小={executable_bytes} 字节，开始身份捕获");
    let mut stage_started = Instant::now();
    let expected = ExpectedFileIdentity::capture(&executable_path).unwrap();
    eprintln!(
        "Windows DLL 拒绝诊断：身份捕获耗时={}ms，开始 PE 与租约准备",
        stage_started.elapsed().as_millis()
    );
    stage_started = Instant::now();
    let mut executable = prepare(&expected).unwrap();
    eprintln!(
        "Windows DLL 拒绝诊断：PE 与租约准备耗时={}ms，开始夹具准备",
        stage_started.elapsed().as_millis()
    );
    stage_started = Instant::now();
    let directory = tempfile::tempdir().unwrap();
    let system_directory = prepare_system_directory().unwrap();
    let system_path = final_path_from_handle(&system_directory.file).unwrap();
    let copied_dll = directory.path().join("copied-version.dll");
    let copied_dll_bytes = fs::copy(system_path.join("version.dll"), &copied_dll).unwrap();
    let marker = directory.path().join("loaded.txt");
    let cwd_identity = AtomicDirectoryIdentity::capture(directory.path()).unwrap();
    let cwd = prepare_directory(&cwd_identity).unwrap();
    eprintln!(
        "Windows DLL 拒绝诊断：夹具准备耗时={}ms，测试 DLL 大小={copied_dll_bytes} 字节，开始命令核验",
        stage_started.elapsed().as_millis()
    );
    let mut command = Command::new(executable.execution_path());
    command
        .arg("non_system_dynamic_image_fixture")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(DYNAMIC_IMAGE_FIXTURE_ENV, &copied_dll)
        .env(DYNAMIC_IMAGE_MARKER_ENV, &marker)
        .current_dir(cwd.execution_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .inherit_managed_job()
        .creation_flags(DEBUG_PROCESS.0);
    stage_started = Instant::now();
    executable
        .verify_command_for_suspended_spawn(&command)
        .unwrap();
    eprintln!(
        "Windows DLL 拒绝诊断：命令核验耗时={}ms，开始挂起派生",
        stage_started.elapsed().as_millis()
    );
    stage_started = Instant::now();
    let suspended = spawn_debuggee(&mut command);
    eprintln!(
        "Windows DLL 拒绝诊断：挂起派生耗时={}ms，开始恢复主线程",
        stage_started.elapsed().as_millis()
    );
    let root_process_id = suspended.id();
    stage_started = Instant::now();
    let mut child = resume_debuggee(suspended);
    eprintln!(
        "Windows DLL 拒绝诊断：恢复主线程耗时={}ms，开始根映像初始事件接管",
        stage_started.elapsed().as_millis()
    );
    stage_started = Instant::now();
    let mut debug = executable
        .begin_image_debug_session(root_process_id)
        .unwrap();
    eprintln!(
        "Windows DLL 拒绝诊断：根映像初始事件接管耗时={}ms，开始非系统 DLL 拒绝等待",
        stage_started.elapsed().as_millis()
    );
    drop(cwd);

    stage_started = Instant::now();
    let failure = debug.wait_for_exit(&mut child).unwrap_err();
    eprintln!(
        "Windows DLL 拒绝诊断：非系统 DLL 拒绝等待耗时={}ms，已返回拒绝结果",
        stage_started.elapsed().as_millis()
    );
    stage_started = Instant::now();
    let status = child.wait().unwrap();
    eprintln!(
        "Windows DLL 拒绝诊断：根进程退出等待耗时={}ms，原生状态：{status}",
        stage_started.elapsed().as_millis()
    );

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_loaded_image_outside_system_directory"
    );
    assert!(!marker.exists());
    record_debug_native_exit("debug_session_rejects_non_system_dynamic_image_before_continue");
}

#[test]
fn cwd_lease_blocks_leaf_rename_until_released() {
    let directory = tempfile::tempdir().unwrap();
    let cwd_path = directory.path().join("cwd");
    let moved_cwd = directory.path().join("moved-cwd");
    fs::create_dir(&cwd_path).unwrap();
    let cwd_identity = AtomicDirectoryIdentity::capture(&cwd_path).unwrap();
    let cwd = prepare_directory(&cwd_identity).unwrap();

    cwd.verify_for_spawn().unwrap();
    assert!(fs::rename(&cwd_path, &moved_cwd).is_err());
    assert!(fs::remove_dir(&cwd_path).is_err());
    cwd.verify_for_spawn().unwrap();
    drop(cwd);
    fs::rename(&cwd_path, &moved_cwd).unwrap();
    fs::remove_dir(&moved_cwd).unwrap();
}

#[test]
fn protected_component_pe_accepts_anycpu_import_directory_with_bounded_tail() {
    let bytes = component_pe_with_import_tail();
    verify_component_pe_bytes(&bytes).unwrap();

    // 同一导入目录不能改变主程序的严格长度规则；DLL 也不能直接成为更新主程序。
    assert!(verify_pe_bytes(&bytes).is_err());
    let mut program = bytes;
    put_u16(&mut program, TEST_PE_OFFSET + 22, 0x0022);
    // 系统 CodeDOM 编译器也可能是 PE32 AnyCPU；只有受保护组件路径可以复用该解析规则。
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(&program).unwrap();
    require_pe_image(&mut file, program.len() as u64, false, true).unwrap();
    assert_eq!(
        verify_pe_bytes(&program).unwrap_err().to_string(),
        "managed_process.atomic_windows_program_not_pe"
    );
}

fn component_pe_with_import_tail() -> Vec<u8> {
    // 模拟 AnyCPU PE32 的一项普通导入、完整零终止项和目录内名称尾部（总长 79）。
    let mut bytes = minimal_pe_for(0x014c, 0x010b, 224, 96);
    put_u16(&mut bytes, TEST_PE_OFFSET + 22, 0x2022);
    set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, TEST_SECTION_RVA, 79);
    put_u32(&mut bytes, TEST_SECTION_OFFSET + 12, TEST_SECTION_RVA + 40);
    bytes[TEST_SECTION_OFFSET + 40..TEST_SECTION_OFFSET + 52].copy_from_slice(b"mscoree.dll\0");
    bytes
}

fn verify_component_pe_bytes(bytes: &[u8]) -> io::Result<()> {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(bytes).unwrap();
    require_pe_kind(&mut file, bytes.len() as u64, true)
}

#[test]
fn protected_component_import_tail_cannot_hide_invalid_descriptors_or_image_headers() {
    for mutation in [
        "partial_terminator",
        "outside_section",
        "path_import",
        "unknown_machine",
        "not_dll",
        "partial_delay_import",
    ] {
        let mut bytes = component_pe_with_import_tail();
        match mutation {
            "partial_terminator" => {
                set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, TEST_SECTION_RVA, 39);
            }
            "outside_section" => {
                set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, TEST_SECTION_RVA, 0x201);
            }
            "path_import" => bytes[TEST_SECTION_OFFSET + 40] = b'/',
            "unknown_machine" => put_u16(&mut bytes, TEST_PE_OFFSET + 4, 0),
            "not_dll" => put_u16(&mut bytes, TEST_PE_OFFSET + 22, 0x0022),
            "partial_delay_import" => {
                set_directory(
                    &mut bytes,
                    DELAY_IMPORT_DIRECTORY_INDEX,
                    TEST_SECTION_RVA + 0x80,
                    33,
                );
            }
            _ => unreachable!(),
        }
        assert!(verify_component_pe_bytes(&bytes).is_err(), "{mutation}");
    }
}

#[test]
fn pe_with_empty_declared_import_table_is_accepted() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, 0x1100, 20);

    verify_pe_bytes(&bytes).unwrap();
}

#[test]
fn pe_with_regular_import_is_accepted_for_runtime_binding() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, 0x1100, 40);
    put_u32(&mut bytes, 0x300 + 12, 0x1140);
    set_import_name(&mut bytes, b"kernel32.dll");

    verify_pe_bytes(&bytes).unwrap();
}

#[test]
fn pe_with_rva_delay_import_is_accepted_for_runtime_binding() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, DELAY_IMPORT_DIRECTORY_INDEX, 0x1100, 64);
    put_u32(&mut bytes, 0x300, 1);
    put_u32(&mut bytes, 0x300 + 4, 0x1140);
    set_import_name(&mut bytes, b"projectedfslib.dll");

    verify_pe_bytes(&bytes).unwrap();
}

#[test]
fn pe_with_path_import_name_is_rejected() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, 0x1100, 40);
    put_u32(&mut bytes, 0x300 + 12, 0x1140);
    set_import_name(&mut bytes, b"C:\\temp\\evil.dll");

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_import_name_invalid"
    );
}

#[test]
fn pe_with_va_delay_descriptor_is_rejected() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, DELAY_IMPORT_DIRECTORY_INDEX, 0x1100, 64);
    put_u32(&mut bytes, 0x300 + 4, 0x1140);
    set_import_name(&mut bytes, b"projectedfslib.dll");

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_delay_import_not_rva_based"
    );
}

#[test]
fn pe_with_unterminated_import_table_is_rejected() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, 0x1100, 20);
    put_u32(&mut bytes, 0x300 + 12, 0x1140);
    set_import_name(&mut bytes, b"kernel32.dll");

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_import_table_unterminated"
    );
}

#[test]
fn pe_with_bound_import_directory_is_rejected() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, BOUND_IMPORT_DIRECTORY_INDEX, 0x1100, 8);

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_bound_import_unsupported"
    );
}

#[test]
fn pe_with_import_directory_outside_file_is_rejected() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, 0x1300, 20);

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_program_not_pe"
    );
}

#[test]
fn pe_with_import_directory_inside_headers_is_rejected() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, 0x40, 20);

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_program_not_pe"
    );
}

#[test]
fn pe_with_partial_delay_descriptor_is_rejected() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, DELAY_IMPORT_DIRECTORY_INDEX, 0x1100, 31);

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_program_not_pe"
    );
}

#[test]
fn pe_header_cannot_overlap_the_dos_header() {
    let mut bytes = minimal_pe();
    put_u32(&mut bytes, DOS_HEADER_PE_OFFSET as usize, 0x20);

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_program_not_pe"
    );
}

#[test]
fn pe_section_count_has_a_fixed_oom_limit() {
    let mut bytes = minimal_pe();
    put_u16(&mut bytes, TEST_PE_OFFSET + 6, u16::MAX);

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_program_not_pe"
    );
}

#[test]
fn pe_import_directory_has_a_fixed_oom_limit() {
    let mut bytes = minimal_pe();
    set_directory(&mut bytes, IMPORT_DIRECTORY_INDEX, 0x1100, u32::MAX);

    let failure = verify_pe_bytes(&bytes).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_program_not_pe"
    );
}

#[test]
fn fixed_system_powershell_helper_is_leased_and_rejects_an_identical_copy() {
    let directory = tempfile::tempdir().unwrap();
    let system = directory.path().join("System32");
    let parent = system.join("WindowsPowerShell/v1.0");
    fs::create_dir_all(&parent).unwrap();
    let program = parent.join("powershell.exe");
    fs::write(&program, minimal_pe()).unwrap();
    let file = open_ancestor(&system).unwrap();
    let system = AncestorLease {
        identity: inspect_handle(&file).unwrap(),
        file,
    };
    let helper = prepare_powershell(&system).unwrap().unwrap();
    helper.verify_image(&File::open(&program).unwrap()).unwrap();

    let shadow = directory.path().join("powershell.exe");
    fs::copy(&program, &shadow).unwrap();
    assert!(helper.verify_image(&File::open(&shadow).unwrap()).is_err());
    assert!(OpenOptions::new().write(true).open(&program).is_err());
    assert!(fs::rename(&parent, parent.with_file_name("replaced")).is_err());
    drop(helper);
    fs::rename(&parent, parent.with_file_name("replaced")).unwrap();
}

#[test]
fn fixed_system_powershell_helper_does_not_search_other_locations() {
    let directory = tempfile::tempdir().unwrap();
    let system = directory.path().join("System32");
    fs::create_dir(&system).unwrap();
    fs::write(system.join("powershell.exe"), minimal_pe()).unwrap();
    let file = open_ancestor(&system).unwrap();
    let system = AncestorLease {
        identity: inspect_handle(&file).unwrap(),
        file,
    };
    assert!(prepare_powershell(&system).unwrap().is_none());
}

fn component_acl(aces: &[(u8, u8, u32, Vec<u8>)]) -> Vec<u8> {
    let mut acl = vec![2, 0, 0, 0, aces.len() as u8, 0, 0, 0];
    for (kind, flags, mask, sid) in aces {
        let size = (8 + sid.len()) as u16;
        acl.extend_from_slice(&[*kind, *flags]);
        acl.extend_from_slice(&size.to_le_bytes());
        acl.extend_from_slice(&mask.to_le_bytes());
        acl.extend_from_slice(sid);
    }
    let size = acl.len() as u16;
    acl[2..4].copy_from_slice(&size.to_le_bytes());
    acl
}

#[test]
fn protected_components_require_trusted_owner_and_all_effective_write_grants() {
    let system = system_sid(&[18]);
    let users = system_sid(&[32, 545]);
    let mut aces = vec![
        (0, 0, 0x001f01ff, system.clone()),
        (0, 0, 0x001200a9, users.clone()),
    ];
    assert!(protected_security(&system, &component_acl(&aces)));
    let mut padded = component_acl(&aces);
    padded.extend_from_slice(&[0xa5; 4]);
    let size = padded.len() as u16;
    padded[2..4].copy_from_slice(&size.to_le_bytes());
    assert!(protected_security(&system, &padded));
    assert!(!protected_security(&users, &component_acl(&aces)));
    // 显式 deny 不能用来掩盖后面的未知主体写授权；宁可拒绝，不求解复杂访问令牌。
    aces.push((1, 0, 0x001f01ff, users.clone()));
    aces.push((0, 0x10, 2, users.clone()));
    assert!(!protected_security(&system, &component_acl(&aces)));
    aces.pop();
    aces.push((0, 0x08, 0x10000000, users));
    assert!(protected_security(&system, &component_acl(&aces)));
    for kind in [5, 6, 9, 10, 0xff] {
        aces[0].0 = kind;
        assert!(!protected_security(&system, &component_acl(&aces)));
    }
    assert!(!protected_security(&system, &[]));
    assert!(!protected_security(&system, &[0; 8]));
}

#[test]
fn component_paths_are_limited_to_actual_windows_component_roots_and_dlls() {
    let windows = Path::new(r"C:\Windows");
    for relative in [
        r"Microsoft.NET\Framework64\v4.0.30319\mscoreei.dll",
        r"assembly\thing.dll",
        r"assembly\NativeImages_v4.0.30319_64\System\hash\System.ni.dll",
        r"WinSxS\component\runtime.DLL",
        r"System32\WindowsPowerShell\v1.0\pwrshplugin.dll",
    ] {
        assert_eq!(
            is_component_path(&windows.join(relative), windows),
            relative != r"assembly\thing.dll"
        );
    }
    for path in [
        r"C:\Windows-other\Microsoft.NET\x.dll",
        r"C:\private\Windows\Microsoft.NET\x.dll",
        r"C:\Windows\Temp\x.dll",
        r"C:\Windows\Microsoft.NET\x.exe",
    ] {
        assert!(!is_component_path(Path::new(path), windows));
    }
}

#[test]
fn component_image_rejects_fake_system_anchor() {
    let fixture = Fixture::new();
    let fake = open_ancestor(&fixture.bin).unwrap();
    let system = AncestorLease {
        identity: inspect_handle(&fake).unwrap(),
        file: fake,
    };
    for dll in [true, false] {
        assert!(
            prepare_component_image(&File::open(&fixture.program).unwrap(), &system, dll).is_err()
        );
    }
    assert!(!is_plain_kind(
        FILE_ATTRIBUTE_REPARSE_POINT.0 | FILE_ATTRIBUTE_DIRECTORY.0,
        true
    ));
}

#[test]
fn framework_compilers_require_exact_system_runtime_paths() {
    let windows = Path::new(r"C:\Windows");
    for framework in ["Framework", "Framework64"] {
        for name in ["csc.exe", "cvtres.exe", "CSC.EXE"] {
            assert!(is_framework_compiler_path(
                &windows
                    .join("Microsoft.NET")
                    .join(framework)
                    .join("v4.0.30319")
                    .join(name),
                windows,
            ));
        }
    }
    for path in [
        r"C:\private\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe",
        r"C:\Windows-other\Microsoft.NET\Framework64\v4.0.30319\csc.exe",
        r"C:\Windows\Microsoft.NET\Framework64\v4.0.30319\unknown.exe",
        r"C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.dll",
        r"C:\Windows\Microsoft.NET\Framework64\v4.0.30319\nested\csc.exe",
        r"C:\Windows\Microsoft.NET\Framework64\v2.0.50727\csc.exe",
        r"C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe:payload",
        r"C:\Windows\Temp\csc.exe",
    ] {
        assert!(!is_framework_compiler_path(Path::new(path), windows));
    }
}

#[test]
fn child_image_requires_authoritative_digest_and_holds_identity_until_exit() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let file = File::open(&fixture.program).unwrap();
    let target = WindowsChildImage {
        size: expected.size,
        sha256: expected.sha256.clone(),
    };
    let lease = prepare_child_image(&file, &target).unwrap();
    assert!(
        OpenOptions::new()
            .write(true)
            .open(&fixture.program)
            .is_err()
    );
    assert!(fs::rename(&fixture.bin, fixture.bin.with_file_name("changed")).is_err());
    let mut wrong = target.clone();
    wrong.sha256 = "0".repeat(64);
    assert!(prepare_child_image(&file, &wrong).is_err());
    wrong = target;
    wrong.size += 1;
    assert!(prepare_child_image(&file, &wrong).is_err());
    drop(lease);
    drop(file);
    fs::rename(&fixture.bin, fixture.bin.with_file_name("changed")).unwrap();
}

#[test]
fn repeated_root_image_requires_same_file_identity_and_contents() {
    let fixture = Fixture::new();
    let expected = fixture.expected();
    let file = File::open(&fixture.program).unwrap();
    let session = WindowsImageDebugSession {
        root_process_id: 0,
        expected_program_id: inspect_handle(&file).unwrap().id,
        expected_program_size: expected.size,
        expected_program_sha256: expected.sha256,
        system_directory: prepare_system_directory().unwrap(),
        powershell: None,
        child_image: None,
        package_images: None,
        npm_console_host: None,
        child_images: HashMap::new(),
        component_images: HashMap::new(),
        processes: HashMap::new(),
        held_package_processes: Vec::new(),
        initial_breakpoints: HashSet::new(),
        pending_event: None,
        station_debugger: None,
        root_exit_observed: false,
        cancellation: None,
        npm_diagnostics: None,
        #[cfg(all(feature = "cli-agent-native-witness", target_arch = "x86_64"))]
        native_witness: None,
        loader_trace: None,
    };
    assert!(session.verify_root_image(&file).is_ok());
    let copy = fixture.bin.join("same-name-copy.exe");
    fs::copy(&fixture.program, &copy).unwrap();
    assert!(
        session
            .verify_root_image(&File::open(copy).unwrap())
            .is_err()
    );
    fs::write(&fixture.program, vec![0; expected.size as usize]).unwrap();
    assert!(session.verify_root_image(&file).is_err());
}

#[test]
fn package_image_rejection_diagnostics_hide_unknown_paths() {
    let fixture = Fixture::new();
    let mut lease = prepare(&fixture.expected()).unwrap();
    lease.set_package_images(vec![fixture.expected()]).unwrap();
    let session = lease.prepare_image_debug_session().unwrap();
    let fields = session.package_image_rejection_fields(&File::open(&fixture.program).unwrap());

    assert_eq!(fields["basename_class"], "unknown");
    assert_eq!(fields["directory_class"], "outside_windows");
    assert_eq!(fields["package_identity_match"], true);
    assert_eq!(fields["package_digest_match"], true);
    assert_eq!(fields["handle_identity_unchanged"], true);
    assert!(!fields.to_string().contains("updater"));
    assert!(
        !fields
            .to_string()
            .contains(&fixture.directory.path().display().to_string())
    );
    assert_eq!(
        rejected_image_basename(Some(Path::new(r"C:\private\NODE.EXE"))),
        "node.exe"
    );
    assert_eq!(
        rejected_image_basename(Some(Path::new(r"C:\private\secret.exe"))),
        "unknown"
    );
}

#[test]
fn debug_backend_cannot_change_while_an_event_is_pending() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let mut session = lease.prepare_image_debug_session().unwrap();
    session.pending_event = Some((41, 43, CREATE_PROCESS_DEBUG_EVENT));

    assert!(session.bind_station_debugger(None).is_err());
    assert_eq!(
        session.pending_event,
        Some((41, 43, CREATE_PROCESS_DEBUG_EVENT))
    );
    assert!(session.station_debugger.is_none());
}

#[test]
fn debug_backend_cannot_change_after_the_root_was_bound() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let mut session = lease.prepare_image_debug_session().unwrap();
    session.root_process_id = 41;

    assert!(session.bind_station_debugger(None).is_err());
    assert_eq!(session.root_process_id, 41);
    assert!(session.station_debugger.is_none());
}

#[test]
fn initial_clr_failure_survives_later_debug_events() {
    let mut trace = NpmDebugTrace::new(Uuid::nil(), 43);
    let mut event = DEBUG_EVENT {
        dwDebugEventCode: EXCEPTION_DEBUG_EVENT,
        dwProcessId: 41,
        dwThreadId: 43,
        ..Default::default()
    };
    event.u.Exception.ExceptionRecord.ExceptionCode = NTSTATUS(0xe0434352_u32 as i32);
    event.u.Exception.ExceptionRecord.NumberParameters = 1;
    event.u.Exception.ExceptionRecord.ExceptionInformation =
        [0x80131534, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    event.u.Exception.dwFirstChance = 1;
    trace.received(&event, 10, 43, false);
    trace.validated(NpmDebugResult::from_io(Ok(()), Duration::ZERO, 43), "root");
    trace.continued(
        NpmDebugResult::from_io(Ok(()), Duration::ZERO, 43),
        DBG_EXCEPTION_NOT_HANDLED.0,
    );
    event.dwDebugEventCode = EXIT_THREAD_DEBUG_EVENT;
    for _ in 0..MAX_NPM_DEBUG_EVENTS {
        trace.received(&event, 20, 43, true);
        trace.validated(
            NpmDebugResult::from_io(Ok(()), Duration::ZERO, 43),
            "console",
        );
        trace.continued(
            NpmDebugResult::from_io(Ok(()), Duration::ZERO, 43),
            DBG_CONTINUE.0,
        );
    }

    assert_eq!(trace.dropped_events, 1);
    assert_eq!(trace.initial_clr_exceptions.len(), 1);
    let failure = &trace.initial_clr_exceptions[0];
    assert_eq!(failure.sequence, 1);
    assert_eq!(failure.clr_hresult, Some(0x80131534));
    assert_eq!(failure.first_chance, Some(1));
    assert_eq!(failure.role, "root");
    assert_eq!(failure.mode, "normal");
    assert!(failure.validation.as_ref().unwrap().ok);
    assert!(failure.continuation.as_ref().unwrap().ok);
    assert_eq!(failure.continue_status, Some(DBG_EXCEPTION_NOT_HANDLED.0));
}

#[test]
fn clr_failure_history_is_bounded_without_replacing_the_first_exception() {
    let mut trace = NpmDebugTrace::new(Uuid::nil(), 43);
    let mut event = DEBUG_EVENT {
        dwDebugEventCode: EXCEPTION_DEBUG_EVENT,
        ..Default::default()
    };
    event.u.Exception.ExceptionRecord.ExceptionCode = NTSTATUS(0xe0434352_u32 as i32);
    event.u.Exception.ExceptionRecord.NumberParameters = 1;
    event.u.Exception.ExceptionRecord.ExceptionInformation =
        [0x80070005, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    for _ in 0..MAX_NPM_DEBUG_EVENTS {
        trace.received(&event, 10, 43, false);
    }
    event.u.Exception.ExceptionRecord.ExceptionInformation =
        [0x80131534, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    trace.received(&event, 20, 43, true);
    trace.validated(NpmDebugResult::from_io(Ok(()), Duration::ZERO, 43), "root");

    assert_eq!(trace.initial_clr_exceptions.len(), MAX_NPM_DEBUG_EVENTS);
    assert_eq!(trace.dropped_clr_exceptions, 1);
    assert_eq!(trace.initial_clr_exceptions[0].sequence, 1);
    assert_eq!(
        trace.initial_clr_exceptions[0].clr_hresult,
        Some(0x80070005)
    );
    assert!(
        trace
            .initial_clr_exceptions
            .last()
            .unwrap()
            .validation
            .is_none()
    );
    assert_eq!(trace.recent.back().unwrap().clr_hresult, Some(0x80131534));
}

#[test]
fn clr_hresult_requires_a_declared_parameter_and_does_not_record_addresses() {
    let mut trace = NpmDebugTrace::new(Uuid::nil(), 43);
    let mut event = DEBUG_EVENT {
        dwDebugEventCode: EXCEPTION_DEBUG_EVENT,
        ..Default::default()
    };
    event.u.Exception.ExceptionRecord.ExceptionCode = NTSTATUS(0xe0434352_u32 as i32);
    event.u.Exception.ExceptionRecord.ExceptionInformation = [
        0x80131534, 0x12345678, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    trace.received(&event, 10, 43, false);
    event.u.Exception.ExceptionRecord.NumberParameters = 16;
    trace.received(&event, 20, 43, false);
    event.u.Exception.ExceptionRecord.ExceptionCode = EXCEPTION_BREAKPOINT;
    event.u.Exception.ExceptionRecord.NumberParameters = 1;
    trace.received(&event, 30, 43, false);

    assert_eq!(trace.initial_clr_exceptions.len(), 2);
    assert_eq!(trace.initial_clr_exceptions[0].clr_hresult, None);
    assert_eq!(trace.initial_clr_exceptions[1].clr_hresult, None);
    assert_eq!(trace.recent.back().unwrap().clr_hresult, None);
    let recorded = serde_json::to_string(&trace.initial_clr_exceptions).unwrap();
    assert!(!recorded.contains("305419896"));
    assert!(!recorded.contains("12345678"));
}

#[test]
fn debug_wait_cancellation_precedes_the_deadline_without_consuming_an_event() {
    let mut trace = NpmDebugTrace::new(Uuid::nil(), 43);
    let failure = wait_for_cancellable_debug_event(
        Instant::now(),
        "已到原截止时间",
        &AtomicBool::new(true),
        Some(&mut trace),
        None,
    )
    .err()
    .expect("取消后不应返回原生事件");

    assert_eq!(failure.kind(), io::ErrorKind::Interrupted);
    assert_eq!(trace.last_boundary, "wait_cancelled");
    assert_eq!(trace.wait_calls, 0);
    assert_eq!(trace.received, 0);
}

#[test]
fn debug_wait_deadline_does_not_start_another_wait() {
    let mut trace = NpmDebugTrace::new(Uuid::nil(), 43);
    let failure = wait_for_cancellable_debug_event(
        Instant::now(),
        "已到原截止时间",
        &AtomicBool::new(false),
        Some(&mut trace),
        None,
    )
    .err()
    .expect("截止后不应返回原生事件");

    assert_eq!(failure.kind(), io::ErrorKind::TimedOut);
    assert_eq!(trace.last_boundary, "wait_deadline");
    assert_eq!(trace.wait_calls, 0);
    assert_eq!(trace.received, 0);
}

#[test]
fn debug_wait_preserves_native_errors_without_treating_transport_failure_as_timeout() {
    let timeout = io::Error::other(WindowsError::from_hresult(HRESULT::from_win32(121)));
    let denied = io::Error::from_raw_os_error(5);
    let transport = io::Error::new(io::ErrorKind::TimedOut, "有界协议未确认");

    assert_eq!(debug_error_code(&timeout), Some(HRESULT::from_win32(121)));
    assert_eq!(debug_error_code(&denied), Some(HRESULT::from_win32(5)));
    assert_eq!(debug_error_code(&transport), None);
}

#[test]
fn failed_exit_continue_keeps_process_and_pending_event() {
    let fixture = Fixture::new();
    let lease = prepare(&fixture.expected()).unwrap();
    let mut session = lease.prepare_image_debug_session().unwrap();
    let process_id = unsafe { GetCurrentProcessId() };
    // 调试事件提供真实句柄；避免把 GetCurrentProcess 的 -1 伪句柄交给生产校验。
    let original =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.unwrap();
    let original = unsafe { OwnedHandle::from_raw_handle(original.0) };
    session.root_process_id = process_id;
    session.processes.insert(
        process_id,
        duplicate_process_handle(HANDLE(original.as_raw_handle())).unwrap(),
    );
    // 当前测试进程不是 debuggee，且线程 0 无效；真实 ContinueDebugEvent 必须失败。
    session.pending_event = Some((process_id, 0, EXIT_PROCESS_DEBUG_EVENT));
    let mut diagnostics = NpmProcessDiagnostics::new();
    diagnostics.roles.insert(process_id, NpmProcessRole::Root);
    diagnostics.received(process_id, EXIT_PROCESS_DEBUG_EVENT, Some(17));
    session.npm_diagnostics = Some(diagnostics);
    session.bind_package_diagnostics(Uuid::new_v4());
    session
        .npm_diagnostics
        .as_mut()
        .unwrap()
        .trace
        .as_mut()
        .unwrap()
        .received(
            &DEBUG_EVENT {
                dwDebugEventCode: EXIT_PROCESS_DEBUG_EVENT,
                dwProcessId: process_id,
                ..Default::default()
            },
            0,
            unsafe { GetCurrentThreadId() },
            false,
        );

    let failure = session
        .continue_pending(DBG_CONTINUE, Instant::now() + DEBUG_DRAIN_TIMEOUT)
        .unwrap_err();

    assert_eq!(
        session.pending_event,
        Some((process_id, 0, EXIT_PROCESS_DEBUG_EVENT))
    );
    assert!(session.processes.contains_key(&process_id));
    assert!(!session.root_exit_observed);
    let diagnostics = session.npm_diagnostics.as_ref().unwrap();
    assert_eq!(
        diagnostics.roles.get(&process_id),
        Some(&NpmProcessRole::Root)
    );
    assert_eq!(diagnostics.pending_exit_code, Some(17));
    let fields = session.package_debug_summary(Err(&failure)).unwrap();
    assert_eq!(fields["last_boundary"], "continue_failed");
    assert_eq!(fields["continued"], 0);
    assert_eq!(fields["recent"][0]["continuation"]["ok"], false);
    assert!(fields["recent"][0]["continuation"]["hresult"].is_i64());
    assert_eq!(
        fields["pending_event"],
        serde_json::json!([process_id, 0, EXIT_PROCESS_DEBUG_EVENT.0])
    );
}

struct PackageProbeFixture {
    process: command::windows::AppContainerProbe,
    debugger: WindowsImageDebugSession,
    _lease: WindowsReplacementLease,
    marker: PathBuf,
    receipt: PathBuf,
    rejected_identity: ExpectedFileId,
    _directory: tempfile::TempDir,
}

impl PackageProbeFixture {
    fn new() -> Self {
        Self::with_bound_child(false)
    }

    fn with_bound_child(bind_child: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        for name in ["home", "config", "cache", "data", "tmp"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        let system = prepare_system_directory().unwrap();
        let program = final_path_from_handle(&system.file)
            .unwrap()
            .join("cmd.exe");
        let allowed = root.join("allowed.exe");
        let rejected = root.join("unbound.exe");
        fs::copy(&program, &allowed).unwrap();
        fs::copy(&program, &rejected).unwrap();
        let mut lease = prepare(&ExpectedFileIdentity::capture(&program).unwrap()).unwrap();
        let mut images = vec![ExpectedFileIdentity::capture(&allowed).unwrap()];
        if bind_child {
            images.push(ExpectedFileIdentity::capture(&rejected).unwrap());
        }
        lease.set_package_images(images).unwrap();
        lease.enable_npm_console_host().unwrap();
        let debugger = lease.prepare_image_debug_session().unwrap();
        let marker = root.join("tmp").join("unverified-image-ran.txt");
        // 使用真正的 Windows cmd 子映像；拒绝时其用户态重定向不应执行。
        let command_path = rejected.to_str().unwrap().strip_prefix(r"\\?\").unwrap();
        let marker_path = marker.to_str().unwrap().strip_prefix(r"\\?\").unwrap();
        let arguments =
            format!(r#"/d /v:off /s /c ""{command_path}" /d /c "echo forbidden>"{marker_path}"""#);
        let environment = super::super::version_probe::resolved_environment(&root).unwrap();
        let process = command::windows::AppContainerProbe::spawn_package_suspended(
            lease.execution_path(),
            arguments.as_ref(),
            &root,
            &environment,
            &format!("InfiniShell.Version.{}", uuid::Uuid::new_v4()),
            &[allowed, rejected.clone()],
        )
        .unwrap();
        Self {
            process,
            debugger,
            _lease: lease,
            marker,
            receipt: root.join("appcontainer-cleanup-v1"),
            rejected_identity: ExpectedFileIdentity::capture(&rejected)
                .unwrap()
                .file_id
                .unwrap(),
            _directory: directory,
        }
    }

    fn finish(&mut self) {
        self.process.terminate_job().unwrap();
        self.debugger
            .drain_terminated_package_in_container(&self.process)
            .unwrap();
        self.process.write_cleanup_receipt(&self.receipt).unwrap();
        assert_eq!(
            fs::read(&self.receipt).unwrap(),
            b"appcontainer-no-capabilities-job-empty-profile-deleted-v1\n"
        );
        assert!(!self.marker.exists());
    }
}

#[test]
#[ignore = "Windows AppContainer 实际映像拒绝与整树退出须显式运行"]
fn package_probe_rejects_unbound_child_and_confirms_cleanup() {
    let name = "package_probe_rejects_unbound_child_and_confirms_cleanup";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let mut fixture = PackageProbeFixture::new();
    fixture.process.resume().unwrap();
    fixture
        .debugger
        .verify_package_initial_image_in_container(&fixture.process)
        .unwrap();
    let deadline = Instant::now() + DEBUG_DRIVER_TIMEOUT;
    let (failure, rejected_identity) = loop {
        let event = fixture
            .debugger
            .next_event(
                deadline,
                "测试未收到子映像事件",
                #[cfg(all(feature = "cli-agent-native-witness", target_arch = "x86_64"))]
                None,
            )
            .unwrap();
        let image = (event.dwDebugEventCode == CREATE_PROCESS_DEBUG_EVENT).then(|| {
            // 复制 hFile，仅供断言；实际事件所有权仍由 validate_event 接管。
            let handle = unsafe { BorrowedHandle::borrow_raw(event.u.CreateProcessInfo.hFile.0) };
            let file = File::from(handle.try_clone_to_owned().unwrap());
            let identity = inspect_handle(&file).unwrap().id;
            identity
        });
        match fixture
            .debugger
            .validate_event_in_container(&event, Some(&fixture.process))
        {
            Ok(status) => fixture.debugger.continue_pending(status, deadline).unwrap(),
            Err(failure) => break (failure, image),
        }
    };
    assert_eq!(failure.to_string(), "npm 包探针拒绝未绑定的子进程映像");
    assert_eq!(rejected_identity, Some(fixture.rejected_identity));
    assert!(!fixture.marker.exists());
    assert!(!fixture.receipt.exists());
    fixture.finish();
    record_debug_native_exit(name);
}

#[test]
#[ignore = "Windows AppContainer 首事件前的终止闭包须显式运行"]
fn package_probe_cleans_up_before_initial_debug_event() {
    let name = "package_probe_cleans_up_before_initial_debug_event";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let mut fixture = PackageProbeFixture::new();
    // 主线程未恢复、会话尚未领到首事件，仍须用本次已持有根 PID 完成显式清理。
    assert_eq!(fixture.debugger.root_process_id, 0);
    assert!(fixture.debugger.pending_event.is_none());
    fixture.finish();
    record_debug_native_exit(name);
}

#[test]
#[ignore = "Windows AppContainer Continue 失败后的事件保留须显式运行"]
fn package_probe_continue_failure_preserves_pending_cleanup() {
    let name = "package_probe_continue_failure_preserves_pending_cleanup";
    if std::env::var_os(DEBUG_DRIVER_ENV).is_none() {
        run_debug_fixture_in_strict_job(name, DEBUG_DRIVER_TIMEOUT);
        return;
    }
    await_debug_driver_authorization();
    let mut fixture = PackageProbeFixture::new();
    fixture.process.resume().unwrap();
    fixture
        .debugger
        .verify_package_initial_image_in_container(&fixture.process)
        .unwrap();
    let event = fixture
        .debugger
        .next_event(
            Instant::now() + DEBUG_DRIVER_TIMEOUT,
            "测试未收到 loader 事件",
            #[cfg(all(feature = "cli-agent-native-witness", target_arch = "x86_64"))]
            None,
        )
        .unwrap();
    fixture
        .debugger
        .validate_event_in_container(&event, Some(&fixture.process))
        .unwrap();
    let pending = fixture.debugger.pending_event.unwrap();
    fixture.debugger.pending_event = Some((pending.0, 0, pending.2));
    fixture.process.terminate_job().unwrap();
    assert!(
        fixture
            .debugger
            .drain_terminated_package_in_container(&fixture.process)
            .is_err()
    );
    assert_eq!(
        fixture.debugger.pending_event,
        Some((pending.0, 0, pending.2))
    );
    assert!(!fixture.debugger.root_exit_observed);
    assert!(!fixture.receipt.exists());
    // 恢复仍未继续的同一真实事件，清理私有夹具；不是把失败当作确认退出。
    fixture.debugger.pending_event = Some(pending);
    fixture.finish();
    record_debug_native_exit(name);
}

#[cfg(target_arch = "x86_64")]
#[test]
fn protected_component_accepts_system_powershell_dotnet_runtime() {
    let system = prepare_system_directory().unwrap();
    let system_path = final_path_from_handle(&system.file).unwrap();
    let runtime = system_path
        .parent()
        .unwrap()
        .join(r"Microsoft.NET\Framework64\v4.0.30319\mscoreei.dll");
    let file = File::open(runtime).unwrap();
    let lease = prepare_component_image(&file, &system, true).unwrap();
    lease.verify_image(&file).unwrap();
    for name in ["csc.exe", "cvtres.exe"] {
        let path = system_path
            .parent()
            .unwrap()
            .join("Microsoft.NET")
            .join("Framework64")
            .join("v4.0.30319")
            .join(name);
        let compiler = File::open(path).unwrap();
        let compiler_lease = prepare_component_image(&compiler, &system, false).unwrap();
        compiler_lease.verify_image(&compiler).unwrap();
        assert!(prepare_component_image(&compiler, &system, true).is_err());
    }
}

#[test]
fn debug_process_duplicate_does_not_own_or_close_the_original_handle() {
    let process_id = unsafe { GetCurrentProcessId() };
    let original =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.unwrap();
    let original = unsafe { OwnedHandle::from_raw_handle(original.0) };
    let original_raw = HANDLE(original.as_raw_handle());
    let duplicate = duplicate_process_handle(original_raw).unwrap();
    assert_ne!(duplicate.as_raw_handle(), original.as_raw_handle());
    assert_eq!(
        unsafe { GetProcessId(HANDLE(duplicate.as_raw_handle())) },
        process_id
    );
    drop(duplicate);
    assert_eq!(unsafe { GetProcessId(original_raw) }, process_id);
    let duplicate = duplicate_process_handle(original_raw).unwrap();
    drop(original);
    assert_eq!(
        unsafe { GetProcessId(HANDLE(duplicate.as_raw_handle())) },
        process_id
    );
}

#[path = "managed_process_atomic_windows_console_tests.rs"]
mod console_binding;
