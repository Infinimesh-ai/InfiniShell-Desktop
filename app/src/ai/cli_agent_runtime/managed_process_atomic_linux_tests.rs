use super::*;
use std::fs;
use std::os::unix::ffi::OsStringExt as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::{env, mem::ManuallyDrop, path::PathBuf};

use command::blocking::Command;

const PROCESS_HELPER_TEST: &str =
    "ai::cli_agent_runtime::managed_process::atomic_linux::tests::execveat_process_helper";
const PROCESS_ROLE_ENV: &str = "INFINISHELL_ATOMIC_LINUX_TEST_ROLE";
const PROCESS_EXECUTABLE_ENV: &str = "INFINISHELL_ATOMIC_LINUX_TEST_EXECUTABLE";
const EXECVEAT_FAILURE_MARKER: &str = "INFINISHELL_EXECVEAT_FAILURE_RETURNED";

fn expected(path: &Path) -> ExpectedFileIdentity {
    let canonical_path = path.canonicalize().unwrap();
    let mut file = open_expected_file(&canonical_path).unwrap();
    let metadata = file.metadata().unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    let mut digest = Sha256::new();
    digest.update(bytes);
    ExpectedFileIdentity {
        path: path.to_owned(),
        canonical_path,
        sha256: format!("{:x}", digest.finalize()),
        size: metadata.len(),
        file_id: Some(ExpectedFileId {
            volume: metadata.dev(),
            index: metadata.ino(),
        }),
    }
}

fn minimal_elf() -> Vec<u8> {
    let mut bytes = vec![0_u8; 120];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&3_u16.to_le_bytes());
    bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
    bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1_u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
    bytes
}

fn elf_with_segment(segment_type: u32, flags: u32) -> Vec<u8> {
    let mut bytes = minimal_elf();
    bytes[64..68].copy_from_slice(&segment_type.to_le_bytes());
    bytes[68..72].copy_from_slice(&flags.to_le_bytes());
    bytes
}

fn static_pie_with_dynamic_tag(tag: u64) -> Vec<u8> {
    let mut bytes = vec![0_u8; 136];
    bytes[..120].copy_from_slice(&minimal_elf());
    bytes[64..68].copy_from_slice(&2_u32.to_le_bytes());
    bytes[72..80].copy_from_slice(&120_u64.to_le_bytes());
    bytes[96..104].copy_from_slice(&16_u64.to_le_bytes());
    bytes[120..128].copy_from_slice(&tag.to_le_bytes());
    bytes
}

fn elf_with_program_header_count(count: u16) -> Vec<u8> {
    let table_size = usize::from(count) * 56;
    let mut bytes = vec![0_u8; 64 + table_size];
    bytes[..64].copy_from_slice(&minimal_elf()[..64]);
    bytes[56..58].copy_from_slice(&count.to_le_bytes());
    bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
    bytes
}

fn static_pie_with_dynamic_table_size(size: u64) -> Vec<u8> {
    let offset = 120_u64;
    let mut bytes = vec![0_u8; usize::try_from(offset + size).unwrap()];
    bytes[..120].copy_from_slice(&minimal_elf());
    bytes[64..68].copy_from_slice(&2_u32.to_le_bytes());
    bytes[72..80].copy_from_slice(&offset.to_le_bytes());
    bytes[96..104].copy_from_slice(&size.to_le_bytes());
    bytes
}

#[cfg(target_arch = "x86_64")]
fn native_probe_machine_and_code() -> (u16, Vec<u8>) {
    // 无 libc 的 `_start`：依次 write argv[0]、argv[1]、envp[0] 和 getcwd，再 exit。
    let code = [
        0x49, 0x89, 0xe4, 0x48, 0x81, 0xec, 0x00, 0x02, 0x00, 0x00, 0x49, 0x8b, 0x74, 0x24, 0x08,
        0xb8, 0x01, 0x00, 0x00, 0x00, 0xbf, 0x01, 0x00, 0x00, 0x00, 0xba, 0x0c, 0x00, 0x00, 0x00,
        0x0f, 0x05, 0x48, 0x8d, 0x35, 0xc3, 0x00, 0x00, 0x00, 0xb8, 0x01, 0x00, 0x00, 0x00, 0xbf,
        0x01, 0x00, 0x00, 0x00, 0xba, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x49, 0x8b, 0x74, 0x24,
        0x10, 0xb8, 0x01, 0x00, 0x00, 0x00, 0xbf, 0x01, 0x00, 0x00, 0x00, 0xba, 0x0e, 0x00, 0x00,
        0x00, 0x0f, 0x05, 0x48, 0x8d, 0x35, 0x95, 0x00, 0x00, 0x00, 0xb8, 0x01, 0x00, 0x00, 0x00,
        0xbf, 0x01, 0x00, 0x00, 0x00, 0xba, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x49, 0x8b, 0x74,
        0x24, 0x20, 0xb8, 0x01, 0x00, 0x00, 0x00, 0xbf, 0x01, 0x00, 0x00, 0x00, 0xba, 0x21, 0x00,
        0x00, 0x00, 0x0f, 0x05, 0x48, 0x8d, 0x35, 0x67, 0x00, 0x00, 0x00, 0xb8, 0x01, 0x00, 0x00,
        0x00, 0xbf, 0x01, 0x00, 0x00, 0x00, 0xba, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05, 0xb8, 0x4f,
        0x00, 0x00, 0x00, 0x48, 0x89, 0xe7, 0xbe, 0x00, 0x02, 0x00, 0x00, 0x0f, 0x05, 0x48, 0x85,
        0xc0, 0x78, 0x36, 0x48, 0xff, 0xc8, 0x48, 0x89, 0xc2, 0x48, 0x89, 0xe6, 0xb8, 0x01, 0x00,
        0x00, 0x00, 0xbf, 0x01, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x48, 0x8d, 0x35, 0x26, 0x00, 0x00,
        0x00, 0xb8, 0x01, 0x00, 0x00, 0x00, 0xbf, 0x01, 0x00, 0x00, 0x00, 0xba, 0x01, 0x00, 0x00,
        0x00, 0x0f, 0x05, 0xb8, 0x3c, 0x00, 0x00, 0x00, 0x31, 0xff, 0x0f, 0x05, 0xb8, 0x3c, 0x00,
        0x00, 0x00, 0xbf, 0x46, 0x00, 0x00, 0x00, 0x0f, 0x05, 0x0a,
    ];
    (62, code.to_vec())
}

#[cfg(target_arch = "aarch64")]
fn native_probe_machine_and_code() -> (u16, Vec<u8>) {
    // 与 x86_64 固件相同，只使用 aarch64 Linux write/getcwd/exit 系统调用。
    let instructions = [
        0x9100_03f4_u32,
        0xd108_03ff,
        0xf940_0681,
        0xd280_0020,
        0xd280_0182,
        0xd280_0808,
        0xd400_0001,
        0x1000_05c1,
        0xd280_0020,
        0xd280_0022,
        0xd280_0808,
        0xd400_0001,
        0xf940_0a81,
        0xd280_0020,
        0xd280_01c2,
        0xd280_0808,
        0xd400_0001,
        0x1000_0481,
        0xd280_0020,
        0xd280_0022,
        0xd280_0808,
        0xd400_0001,
        0xf940_1281,
        0xd280_0020,
        0xd280_0422,
        0xd280_0808,
        0xd400_0001,
        0x1000_0341,
        0xd280_0020,
        0xd280_0022,
        0xd280_0808,
        0xd400_0001,
        0x9100_03e0,
        0xd280_4001,
        0xd280_0228,
        0xd400_0001,
        0xb7f8_01c0,
        0xd100_0402,
        0x9100_03e1,
        0xd280_0020,
        0xd280_0808,
        0xd400_0001,
        0x1000_0161,
        0xd280_0020,
        0xd280_0022,
        0xd280_0808,
        0xd400_0001,
        0xd280_0000,
        0xd280_0ba8,
        0xd400_0001,
        0xd280_08c0,
        0xd280_0ba8,
        0xd400_0001,
    ];
    let mut code = instructions
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    code.push(b'\n');
    (183, code)
}

fn native_static_probe() -> Vec<u8> {
    const ELF_HEADER_SIZE: usize = 64;
    const PROGRAM_HEADER_SIZE: usize = 56;
    const PROGRAM_HEADER_COUNT: usize = 2;
    const IMAGE_BASE: u64 = 0x40_0000;

    let (machine, code) = native_probe_machine_and_code();
    let code_offset = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE * PROGRAM_HEADER_COUNT;
    let file_size = u64::try_from(code_offset + code.len()).unwrap();
    let mut bytes = vec![0_u8; code_offset];
    bytes.extend_from_slice(&code);

    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&2_u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&machine.to_le_bytes());
    bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&(IMAGE_BASE + code_offset as u64).to_le_bytes());
    bytes[32..40].copy_from_slice(&(ELF_HEADER_SIZE as u64).to_le_bytes());
    bytes[52..54].copy_from_slice(&(ELF_HEADER_SIZE as u16).to_le_bytes());
    bytes[54..56].copy_from_slice(&(PROGRAM_HEADER_SIZE as u16).to_le_bytes());
    bytes[56..58].copy_from_slice(&(PROGRAM_HEADER_COUNT as u16).to_le_bytes());

    let load = ELF_HEADER_SIZE;
    bytes[load..load + 4].copy_from_slice(&1_u32.to_le_bytes());
    bytes[load + 4..load + 8].copy_from_slice(&5_u32.to_le_bytes());
    bytes[load + 16..load + 24].copy_from_slice(&IMAGE_BASE.to_le_bytes());
    bytes[load + 24..load + 32].copy_from_slice(&IMAGE_BASE.to_le_bytes());
    bytes[load + 32..load + 40].copy_from_slice(&file_size.to_le_bytes());
    bytes[load + 40..load + 48].copy_from_slice(&file_size.to_le_bytes());
    bytes[load + 48..load + 56].copy_from_slice(&0x1000_u64.to_le_bytes());

    let stack = load + PROGRAM_HEADER_SIZE;
    bytes[stack..stack + 4].copy_from_slice(&0x6474_e551_u32.to_le_bytes());
    bytes[stack + 4..stack + 8].copy_from_slice(&6_u32.to_le_bytes());
    bytes[stack + 48..stack + 56].copy_from_slice(&16_u64.to_le_bytes());
    bytes
}

fn execveat_helper(role: &str, executable: &Path, current_dir: &Path) -> command::Output {
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args([
            "--exact",
            PROCESS_HELPER_TEST,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(current_dir)
        .env(PROCESS_ROLE_ENV, role)
        .env(PROCESS_EXECUTABLE_ENV, executable);
    command.output().unwrap()
}

fn write_executable(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn sealed_snapshot_matches_expected_bytes_and_rejects_writes() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(&program, &minimal_elf());
    let expected = expected(&program);

    let snapshot = prepare(&expected).unwrap();

    assert_eq!(snapshot.sha256(), expected.sha256);
    assert_eq!(snapshot.size(), expected.size);
    assert_eq!(snapshot.seals().unwrap() & REQUIRED_SEALS, REQUIRED_SEALS);
    assert!(snapshot.try_write_at(b"x", 0).is_err());
}

#[test]
fn sealed_snapshot_file_descriptor_is_close_on_exec() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(&program, &minimal_elf());

    let snapshot = prepare(&expected(&program)).unwrap();

    assert_ne!(snapshot.fd_flags().unwrap() & libc::FD_CLOEXEC, 0);
}

#[test]
fn script_is_rejected_before_it_can_be_executed() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(&program, b"#!/bin/sh\nexit 0\n");

    assert_eq!(
        prepare(&expected(&program)).unwrap_err().to_string(),
        "managed_process.linux_atomic_not_elf"
    );
}

#[test]
fn interpreter_dependency_closure_is_rejected() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(&program, &elf_with_segment(3, 0));

    assert_eq!(
        prepare(&expected(&program)).unwrap_err().to_string(),
        "managed_process.linux_atomic_dependency_closure_unbound"
    );
}

#[test]
fn static_pie_without_needed_dependency_is_accepted() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(&program, &static_pie_with_dynamic_tag(0));

    prepare(&expected(&program)).unwrap();
}

#[test]
fn dynamic_needed_dependency_is_rejected() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(&program, &static_pie_with_dynamic_tag(1));

    assert_eq!(
        prepare(&expected(&program)).unwrap_err().to_string(),
        "managed_process.linux_atomic_dependency_closure_unbound"
    );
}

#[test]
fn executable_stack_is_rejected() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(&program, &elf_with_segment(0x6474_e551, 1));

    assert_eq!(
        prepare(&expected(&program)).unwrap_err().to_string(),
        "managed_process.linux_atomic_executable_stack"
    );
}

#[test]
fn same_content_new_inode_is_rejected() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    let previous = state.path().join("previous");
    let bytes = minimal_elf();
    write_executable(&program, &bytes);
    let expected = expected(&program);
    fs::rename(&program, previous).unwrap();
    write_executable(&program, &bytes);

    assert_eq!(
        prepare(&expected).unwrap_err().to_string(),
        "managed_process.linux_atomic_source_changed"
    );
}

#[test]
fn same_inode_same_size_content_change_is_rejected() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    let bytes = minimal_elf();
    write_executable(&program, &bytes);
    let expected = expected(&program);
    let before = fs::metadata(&program).unwrap();
    let mut changed = bytes;
    let last = changed.len() - 1;
    changed[last] = 1;
    fs::write(&program, changed).unwrap();
    let after = fs::metadata(&program).unwrap();

    assert_eq!(after.dev(), before.dev());
    assert_eq!(after.ino(), before.ino());
    assert_eq!(after.len(), before.len());

    assert_eq!(
        prepare(&expected).unwrap_err().to_string(),
        "managed_process.linux_atomic_source_changed"
    );
}

#[test]
fn maximum_program_header_table_is_accepted() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");

    write_executable(
        &program,
        &elf_with_program_header_count(u16::try_from(MAX_PROGRAM_HEADERS).unwrap()),
    );

    prepare(&expected(&program)).unwrap();
}

#[test]
fn program_header_count_above_limit_is_rejected_when_table_fits_file() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(
        &program,
        &elf_with_program_header_count(u16::try_from(MAX_PROGRAM_HEADERS + 1).unwrap()),
    );

    assert_eq!(
        prepare(&expected(&program)).unwrap_err().to_string(),
        "managed_process.linux_atomic_program_table_invalid"
    );
}

#[test]
fn maximum_dynamic_table_is_accepted() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(
        &program,
        &static_pie_with_dynamic_table_size(MAX_DYNAMIC_TABLE_BYTES),
    );

    prepare(&expected(&program)).unwrap();
}

#[test]
fn dynamic_table_above_limit_is_rejected_when_table_fits_file() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(
        &program,
        &static_pie_with_dynamic_table_size(MAX_DYNAMIC_TABLE_BYTES + 16),
    );

    assert_eq!(
        prepare(&expected(&program)).unwrap_err().to_string(),
        "managed_process.linux_atomic_dynamic_table_invalid"
    );
}

#[test]
fn opened_source_survives_path_replacement_without_reading_replacement() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    let previous = state.path().join("previous");
    let bytes = minimal_elf();
    write_executable(&program, &bytes);
    let expected = expected(&program);

    // `prepare` 自身在打开后不再读取 pathname；这里用相同底层合同直接证明打开对象稳定。
    let source = open_expected_file(&expected.canonical_path).unwrap();
    fs::rename(&program, &previous).unwrap();
    write_executable(&program, b"replacement");
    assert_eq!(
        source.metadata().unwrap().ino(),
        expected.file_id.unwrap().index
    );
    assert_ne!(
        fs::metadata(&program).unwrap().ino(),
        expected.file_id.unwrap().index
    );
}

#[test]
fn invalid_argv_or_environment_is_rejected_without_exec() {
    let state = tempfile::tempdir().unwrap();
    let program = state.path().join("program");
    write_executable(&program, &minimal_elf());
    let snapshot = prepare(&expected(&program)).unwrap();

    let error = execute(
        &snapshot,
        &program,
        &[OsString::from_vec(b"bad\0argument".to_vec())],
        &[],
    );
    assert_eq!(
        error.to_string(),
        "managed_process.linux_atomic_argument_invalid"
    );

    let error = execute(
        &snapshot,
        &program,
        &[],
        &[(OsString::from("BAD=NAME"), OsString::from("value"))],
    );
    assert_eq!(
        error.to_string(),
        "managed_process.linux_atomic_environment_invalid"
    );
}

#[test]
fn sealed_memfd_execveat_forwards_argv_environment_and_cwd() {
    let state = tempfile::tempdir().unwrap();
    let executable = state.path().join("static-probe");
    let working_directory = state.path().join("working-directory");
    write_executable(&executable, &native_static_probe());
    fs::create_dir(&working_directory).unwrap();
    let working_directory = working_directory.canonicalize().unwrap();

    let output = execveat_helper("forward", &executable, &working_directory);

    assert!(
        output.status.success(),
        "execveat 子进程失败：stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let evidence = format!(
        "atomic-argv0\nargument-value\nATOMIC_EXEC_ENV=environment-value\n{}\n",
        working_directory.display()
    );
    assert!(
        stdout.contains(&evidence),
        "缺少 argv/env/cwd 执行证据：{stdout:?}"
    );
}

#[test]
fn execveat_failure_does_not_fall_back_to_pathname() {
    let state = tempfile::tempdir().unwrap();
    let executable = state.path().join("pathname-fallback-probe");
    write_executable(&executable, &native_static_probe());

    let output = execveat_helper("closed-fd", &executable, state.path());

    assert!(
        output.status.success(),
        "失败路径子进程异常：stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains(EXECVEAT_FAILURE_MARKER),
        "execveat 失败后没有返回，可能执行了 pathname：{stdout:?}"
    );
}

#[test]
#[ignore = "仅由 Linux execveat 多进程测试派生"]
fn execveat_process_helper() {
    let executable = PathBuf::from(env::var_os(PROCESS_EXECUTABLE_ENV).unwrap())
        .canonicalize()
        .unwrap();
    let role = env::var(PROCESS_ROLE_ENV).unwrap();
    if role == "layout" {
        let snapshot = prepare_layout_snapshot(
            executable.parent().unwrap(),
            uuid::Uuid::new_v4(),
            &"a".repeat(64),
            &expected(&executable),
        )
        .unwrap();
        let error = execute_layout(
            &snapshot,
            Path::new("atomic-argv0"),
            &[OsString::from("argument-value")],
            &[(
                OsString::from("ATOMIC_EXEC_ENV"),
                OsString::from("environment-value"),
            )],
        );
        panic!("布局快照 execveat 没有替换辅助进程：{error}");
    }
    let snapshot = ManuallyDrop::new(prepare(&expected(&executable)).unwrap());
    match role.as_str() {
        "forward" => {
            let error = execute(
                &snapshot,
                Path::new("atomic-argv0"),
                &[OsString::from("argument-value")],
                &[(
                    OsString::from("ATOMIC_EXEC_ENV"),
                    OsString::from("environment-value"),
                )],
            );
            panic!("execveat 没有替换辅助进程：{error}");
        }
        "closed-fd" => {
            let result = unsafe { libc::close(snapshot.file.as_raw_fd()) };
            assert_eq!(result, 0);
            let error = execute(&snapshot, &executable, &[], &[]);
            assert_eq!(error.raw_os_error(), Some(libc::EBADF));
            println!("{EXECVEAT_FAILURE_MARKER}");
        }
        role => panic!("未知 Linux execveat 测试角色：{role}"),
    }
}

struct LayoutFixture(tempfile::TempDir);

impl LayoutFixture {
    fn path(&self) -> &Path {
        self.0.path()
    }
}

impl Drop for LayoutFixture {
    fn drop(&mut self) {
        let snapshots = self.path().join("cli-agent-executable-snapshots");
        if let Ok(generations) = fs::read_dir(snapshots) {
            for entry in generations.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    let _ = fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o700));
                }
            }
        }
    }
}

fn private_layout_fixture() -> LayoutFixture {
    // 与发布合同一致，祖先不可由其他用户写入；不在共享 /tmp 中建立可执行快照。
    LayoutFixture(
        tempfile::tempdir_in(env::var_os("HOME").expect("Linux 验证需要私有 HOME")).unwrap(),
    )
}

#[test]
fn layout_snapshot_preserves_an_actual_file_path_and_binding_receipt() {
    let state = private_layout_fixture();
    let releases = state.path().canonicalize().unwrap();
    let program = releases.join("source");
    write_executable(&program, &minimal_elf());
    let generation = uuid::Uuid::new_v4();
    let binding = "a".repeat(64);
    let snapshot =
        prepare_layout_snapshot(&releases, generation, &binding, &expected(&program)).unwrap();

    assert_eq!(
        fs::read_link(format!("/proc/self/fd/{}", snapshot.file.as_raw_fd())).unwrap(),
        snapshot.path
    );
    assert_eq!(snapshot.file.metadata().unwrap().mode() & 0o7777, 0o500);
    assert_ne!(
        unsafe { libc::fcntl(snapshot.file.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(snapshot.path.parent().unwrap().join("snapshot.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(receipt["binding_digest"], binding);
    assert_eq!(receipt["generation"], generation.to_string());
    snapshot.verify_for_execution().unwrap();
}

#[test]
fn layout_snapshot_does_not_overwrite_an_existing_generation() {
    let state = private_layout_fixture();
    let releases = state.path().canonicalize().unwrap();
    let program = releases.join("source");
    write_executable(&program, &minimal_elf());
    let generation = uuid::Uuid::new_v4();
    let snapshot =
        prepare_layout_snapshot(&releases, generation, &"a".repeat(64), &expected(&program))
            .unwrap();

    assert!(
        prepare_layout_snapshot(&releases, generation, &"b".repeat(64), &expected(&program))
            .is_err()
    );
    snapshot.verify_for_execution().unwrap();
}

#[test]
fn layout_snapshot_rejects_shared_ancestors_and_directory_links() {
    let state = private_layout_fixture();
    let root = state.path().canonicalize().unwrap();
    let program = root.join("source");
    write_executable(&program, &minimal_elf());
    let releases = root.join("releases");
    fs::create_dir(&releases).unwrap();
    fs::set_permissions(&releases, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        prepare_layout_snapshot(
            &releases,
            uuid::Uuid::new_v4(),
            &"a".repeat(64),
            &expected(&program)
        )
        .is_err()
    );
    fs::set_permissions(&releases, fs::Permissions::from_mode(0o700)).unwrap();
    let redirected = root.join("redirected");
    std::os::unix::fs::symlink(&releases, &redirected).unwrap();
    assert!(
        prepare_layout_snapshot(
            &redirected,
            uuid::Uuid::new_v4(),
            &"a".repeat(64),
            &expected(&program)
        )
        .is_err()
    );
}

#[test]
fn layout_snapshot_rejects_replaced_program_before_exec() {
    let state = private_layout_fixture();
    let releases = state.path().canonicalize().unwrap();
    let program = releases.join("source");
    write_executable(&program, &minimal_elf());
    let snapshot = prepare_layout_snapshot(
        &releases,
        uuid::Uuid::new_v4(),
        &"a".repeat(64),
        &expected(&program),
    )
    .unwrap();
    fs::set_permissions(
        snapshot.path.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::rename(&snapshot.path, snapshot.path.with_file_name("previous")).unwrap();
    write_executable(&snapshot.path, &minimal_elf());
    fs::set_permissions(&snapshot.path, fs::Permissions::from_mode(0o500)).unwrap();
    fs::set_permissions(
        snapshot.path.parent().unwrap(),
        fs::Permissions::from_mode(0o500),
    )
    .unwrap();

    assert_eq!(
        execute_layout(&snapshot, &program, &[], &[]).to_string(),
        "managed_process.linux_layout_snapshot_changed"
    );
}

#[test]
fn layout_snapshot_execveat_preserves_the_original_source() {
    let state = private_layout_fixture();
    let executable = state.path().join("static-probe");
    write_executable(&executable, &native_static_probe());
    let identity = expected(&executable);

    let output = execveat_helper("layout", &executable, state.path());

    assert!(
        output.status.success(),
        "布局快照 execveat 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("atomic-argv0\nargument-value\nATOMIC_EXEC_ENV=environment-value\n")
    );
    assert_eq!(expected(&executable).sha256, identity.sha256);
}

#[test]
fn layout_snapshot_cleanup_is_bound_and_preserves_other_generations() {
    let state = private_layout_fixture();
    let releases = state.path().canonicalize().unwrap();
    let program = releases.join("source");
    write_executable(&program, &minimal_elf());
    let identity = expected(&program);
    let generation = uuid::Uuid::new_v4();
    let binding = "a".repeat(64);
    let snapshot = prepare_layout_snapshot(&releases, generation, &binding, &identity).unwrap();
    let other =
        prepare_layout_snapshot(&releases, uuid::Uuid::new_v4(), &binding, &identity).unwrap();

    assert!(cleanup_layout_snapshot(&releases, generation, &"b".repeat(64), &identity).is_err());
    assert!(snapshot.path.exists());
    cleanup_layout_snapshot(&releases, generation, &binding, &identity).unwrap();
    cleanup_layout_snapshot(&releases, generation, &binding, &identity).unwrap();
    assert!(!snapshot.path.parent().unwrap().exists());
    other.verify_for_execution().unwrap();
    assert_eq!(expected(&program), identity);
}

#[test]
fn layout_snapshot_cleanup_recovers_interrupted_copy_and_unlink() {
    let state = private_layout_fixture();
    let releases = state.path().canonicalize().unwrap();
    let program = releases.join("source");
    write_executable(&program, &minimal_elf());
    let identity = expected(&program);
    for removed_program in [false, true] {
        let generation = uuid::Uuid::new_v4();
        let binding = "a".repeat(64);
        let snapshot = prepare_layout_snapshot(&releases, generation, &binding, &identity).unwrap();
        fs::set_permissions(
            snapshot.path.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        if removed_program {
            fs::remove_file(&snapshot.path).unwrap();
        } else {
            fs::set_permissions(&snapshot.path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(&snapshot.path)
                .unwrap()
                .set_len(3)
                .unwrap();
        }
        cleanup_layout_snapshot(&releases, generation, &binding, &identity).unwrap();
        assert!(!snapshot.path.parent().unwrap().exists());
    }
}

#[test]
fn layout_snapshot_cleanup_rejects_unknown_files_and_replaced_identity() {
    let state = private_layout_fixture();
    let releases = state.path().canonicalize().unwrap();
    let program = releases.join("source");
    write_executable(&program, &minimal_elf());
    let identity = expected(&program);
    let generation = uuid::Uuid::new_v4();
    let binding = "a".repeat(64);
    let snapshot = prepare_layout_snapshot(&releases, generation, &binding, &identity).unwrap();
    let directory = snapshot.path.parent().unwrap();
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(directory.join("unrelated"), b"preserve").unwrap();
    assert!(cleanup_layout_snapshot(&releases, generation, &binding, &identity).is_err());
    assert_eq!(fs::read(directory.join("unrelated")).unwrap(), b"preserve");
    fs::remove_file(directory.join("unrelated")).unwrap();
    // 旧 fd 仍持有旧 inode，保证替换文件不会巧合重用相同身份。
    fs::remove_file(&snapshot.path).unwrap();
    write_executable(&snapshot.path, &minimal_elf());
    fs::set_permissions(&snapshot.path, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(cleanup_layout_snapshot(&releases, generation, &binding, &identity).is_err());
    assert!(snapshot.path.exists());
    assert!(directory.join("snapshot.json").exists());
}
