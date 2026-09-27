use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::os::windows::io::AsHandle as _;

use super::*;

#[test]
fn no_window_mode_preserves_redirected_standard_handles_without_show_flags() {
    // 仅比较启动结构中的哨兵值，不把它们交给任何句柄 API。
    let mut startup = STARTUPINFOW {
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: HANDLE(11_usize as *mut c_void),
        hStdOutput: HANDLE(12_usize as *mut c_void),
        hStdError: HANDLE(13_usize as *mut c_void),
        ..Default::default()
    };

    let flags = ProbeConsoleMode::NoWindow.configure_startup(&mut startup);

    assert_eq!(flags, CREATE_NO_WINDOW);
    assert_eq!(flags & CREATE_NEW_CONSOLE, PROCESS_CREATION_FLAGS(0));
    assert_eq!(startup.dwFlags, STARTF_USESTDHANDLES);
    assert_eq!(startup.wShowWindow, 0);
    assert_eq!(startup.hStdInput, HANDLE(11_usize as *mut c_void));
    assert_eq!(startup.hStdOutput, HANDLE(12_usize as *mut c_void));
    assert_eq!(startup.hStdError, HANDLE(13_usize as *mut c_void));
}

#[test]
fn hidden_new_console_preserves_redirected_standard_handles_and_excludes_no_window() {
    // 隐藏窗口不能把固定标准流重置成新控制台的输入或屏幕缓冲区。
    let mut startup = STARTUPINFOW {
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: HANDLE(21_usize as *mut c_void),
        hStdOutput: HANDLE(22_usize as *mut c_void),
        hStdError: HANDLE(23_usize as *mut c_void),
        ..Default::default()
    };

    let flags = ProbeConsoleMode::HiddenNewConsole.configure_startup(&mut startup);

    assert_eq!(flags, CREATE_NEW_CONSOLE);
    assert_eq!(flags & CREATE_NO_WINDOW, PROCESS_CREATION_FLAGS(0));
    assert_eq!(startup.dwFlags, STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW);
    assert_eq!(startup.wShowWindow, SW_HIDE.0 as u16);
    assert_eq!(startup.hStdInput, HANDLE(21_usize as *mut c_void));
    assert_eq!(startup.hStdOutput, HANDLE(22_usize as *mut c_void));
    assert_eq!(startup.hStdError, HANDLE(23_usize as *mut c_void));
}

#[test]
fn hidden_console_rejects_a_different_execution_directory_before_creating_profile() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let other = tempfile::tempdir().unwrap();

    let result = AppContainerProbe::spawn_package_suspended_with_hidden_console(
        Path::new("unused.exe"),
        "unused".as_ref(),
        &cwd,
        other.path(),
        &[],
        "unused-profile",
        &[],
    );

    assert_eq!(result.err().unwrap().to_string(), "版本探针执行目录不匹配");
}

#[test]
fn hidden_console_rejects_readonly_objects_outside_the_candidate_before_creating_profile() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let other = tempfile::tempdir().unwrap();

    let result = AppContainerProbe::spawn_package_suspended_with_hidden_console(
        Path::new("unused.exe"),
        "unused".as_ref(),
        &cwd,
        &cwd,
        &[],
        "unused-profile",
        &[other.path().canonicalize().unwrap()],
    );

    assert_eq!(
        result.err().unwrap().to_string(),
        "包探针的只读对象范围无效"
    );
}

#[test]
fn package_execution_cwd_rejects_a_different_directory_before_creating_profile() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().canonicalize().unwrap();
    let other = tempfile::tempdir().unwrap();

    let result = AppContainerProbe::spawn_package_suspended_with_execution_cwd(
        Path::new("unused.exe"),
        "unused".as_ref(),
        &cwd,
        other.path(),
        &[],
        "unused-profile",
        &[],
    );

    assert_eq!(result.err().unwrap().to_string(), "版本探针执行目录不匹配");
}

#[test]
fn package_execution_cwd_rejects_relative_paths_before_creating_profile() {
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();

    let result = AppContainerProbe::spawn_package_suspended_with_execution_cwd(
        Path::new("unused.exe"),
        "unused".as_ref(),
        &cwd,
        Path::new("."),
        &[],
        "unused-profile",
        &[],
    );

    assert_eq!(result.err().unwrap().to_string(), "版本探针执行目录不匹配");
}

fn empty_probe() -> (tempfile::TempDir, AppContainerProbe) {
    let directory = tempfile::tempdir().unwrap();
    let mut hash = DefaultHasher::new();
    directory.path().hash(&mut hash);
    let name = format!(
        "InfiniShell.Version.00000000-0000-4001-8000-{:012x}",
        hash.finish() & 0xffff_ffff_ffff
    );
    let profile_name = wide(name.as_ref()).unwrap();
    let sid = unsafe {
        CreateAppContainerProfile(
            PCWSTR(profile_name.as_ptr()),
            PCWSTR(profile_name.as_ptr()),
            PCWSTR(profile_name.as_ptr()),
            None,
        )
    }
    .unwrap();
    let job = owned(unsafe { CreateJobObjectW(None, None) }.unwrap());
    (
        directory,
        AppContainerProbe {
            profile_name,
            sid,
            grants: Vec::new(),
            job,
            process: None,
            thread: None,
            process_id: 0,
            cleaned: false,
        },
    )
}

fn current_process_handle() -> OwnedHandle {
    let current = unsafe { GetCurrentProcess() };
    let mut result = HANDLE::default();
    unsafe {
        DuplicateHandle(
            current,
            current,
            current,
            &mut result,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .unwrap();
    owned(result)
}

#[test]
fn package_process_rejects_a_process_outside_the_exact_job() {
    let (_directory, probe) = empty_probe();
    let current = current_process_handle();

    assert!(!probe.contains_package_process(current.as_handle()).unwrap());
    let failure = probe
        .verify_package_process(current.as_handle())
        .unwrap_err();

    assert_eq!(failure.to_string(), "版本探针子进程不在本次 Job");
}

#[test]
fn package_token_rejects_the_uncontained_test_process() {
    let (_directory, probe) = empty_probe();
    let current = current_process_handle();

    let failure = probe.verify_token_for(handle(&current)).unwrap_err();

    assert_eq!(failure.to_string(), "版本探针不是 AppContainer");
}
