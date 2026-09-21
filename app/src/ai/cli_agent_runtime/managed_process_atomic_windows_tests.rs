use std::fs::{self, OpenOptions};
use std::io::{Seek as _, Write as _};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::{symlink_dir, symlink_file};
use std::process::Stdio;

use windows::Win32::System::LibraryLoader::LoadLibraryW;
use windows::Win32::System::Threading::DEBUG_PROCESS;
use windows::core::PCWSTR;

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

fn minimal_pe() -> Vec<u8> {
    let (machine, magic, optional_size, directory_offset) = if cfg!(target_arch = "x86") {
        (0x014c_u16, 0x010b_u16, 224_u16, 96_usize)
    } else if cfg!(target_arch = "x86_64") {
        (0x8664_u16, 0x020b_u16, 240_u16, 112_usize)
    } else {
        (0xaa64_u16, 0x020b_u16, 240_u16, 112_usize)
    };
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
    let directory_offset = if cfg!(target_arch = "x86") { 96 } else { 112 };
    let optional = TEST_PE_OFFSET + PE_SIGNATURE_AND_FILE_HEADER_BYTES as usize;
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

#[test]
fn reparse_program_is_rejected() {
    let fixture = Fixture::new();
    let link = fixture.bin.join("linked-updater.exe");
    symlink_file(&fixture.program, &link).expect("Windows 验收机必须允许创建测试符号链接");
    let mut expected = fixture.expected();
    expected.path = link.clone();
    expected.canonical_path = link;

    assert!(prepare(&expected).is_err());
}

#[test]
fn reparse_ancestor_is_rejected() {
    let fixture = Fixture::new();
    let linked_bin = fixture.directory.path().join("linked-bin");
    symlink_dir(&fixture.bin, &linked_bin).expect("Windows 验收机必须允许创建测试目录链接");
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
#[ignore = "Windows 实机调试事件退出闭包尚未收敛，产品入口继续保持 ManualOnly"]
fn debug_session_runs_system_only_process_to_native_exit() {
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
    let suspended = command.spawn_suspended().unwrap();
    let mut debug = executable
        .begin_image_debug_session(suspended.id())
        .unwrap();
    drop(cwd);
    let mut child = suspended.resume().unwrap();

    let status = debug.wait_for_exit(&mut child).unwrap();

    assert!(status.success());
}

#[test]
#[ignore = "Windows 实机调试事件退出闭包尚未收敛，仅可按显式收据流程运行"]
fn debug_session_runs_fixed_real_cli_to_native_exit() {
    let Some(executable_path) = std::env::var_os(REAL_CLI_FIXTURE_ENV).map(PathBuf::from) else {
        return;
    };
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .inherit_managed_job()
        .creation_flags(DEBUG_PROCESS.0);
    executable
        .verify_command_for_suspended_spawn(&command)
        .unwrap();
    let suspended = command.spawn_suspended().unwrap();
    let mut debug = executable
        .begin_image_debug_session(suspended.id())
        .unwrap();
    drop(cwd);
    let mut child = suspended.resume().unwrap();

    let status = debug.wait_for_exit(&mut child).unwrap();

    if requested_args.is_none() {
        assert!(status.success());
    } else {
        eprintln!("真实更新参数的原生退出状态：{status}");
    }
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
#[ignore = "Windows 实机调试事件拒绝路径尚未收敛，产品入口继续保持 ManualOnly"]
fn debug_session_rejects_non_system_dynamic_image_before_continue() {
    let executable_path = std::env::current_exe().unwrap();
    let expected = ExpectedFileIdentity::capture(&executable_path).unwrap();
    let mut executable = prepare(&expected).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let system_directory = prepare_system_directory().unwrap();
    let system_path = final_path_from_handle(&system_directory.file).unwrap();
    let copied_dll = directory.path().join("copied-version.dll");
    fs::copy(system_path.join("version.dll"), &copied_dll).unwrap();
    let marker = directory.path().join("loaded.txt");
    let cwd_identity = AtomicDirectoryIdentity::capture(directory.path()).unwrap();
    let cwd = prepare_directory(&cwd_identity).unwrap();
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
    executable
        .verify_command_for_suspended_spawn(&command)
        .unwrap();
    let suspended = command.spawn_suspended().unwrap();
    let mut debug = executable
        .begin_image_debug_session(suspended.id())
        .unwrap();
    drop(cwd);
    let mut child = suspended.resume().unwrap();

    let failure = debug.wait_for_exit(&mut child).unwrap_err();

    assert_eq!(
        failure.to_string(),
        "managed_process.atomic_windows_loaded_image_outside_system_directory"
    );
    assert!(!marker.exists());
}

#[test]
fn cwd_lease_does_not_claim_atomic_replacement_protection() {
    let directory = tempfile::tempdir().unwrap();
    let cwd_path = directory.path().join("cwd");
    let moved_cwd = directory.path().join("moved-cwd");
    fs::create_dir(&cwd_path).unwrap();
    let cwd_identity = AtomicDirectoryIdentity::capture(&cwd_path).unwrap();
    let cwd = prepare_directory(&cwd_identity).unwrap();

    cwd.verify_for_spawn().unwrap();
    fs::rename(&cwd_path, &moved_cwd).unwrap();
    assert!(cwd.verify_for_spawn().is_err());
    drop(cwd);
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
