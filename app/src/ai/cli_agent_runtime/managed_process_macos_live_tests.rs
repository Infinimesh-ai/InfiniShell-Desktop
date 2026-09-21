//! 只运行私有固定 C 夹具；必须使用新构建的真实应用 worker，不请求模型。

use std::os::fd::{AsRawFd as _, FromRawFd as _};

use command::r#async::Command as AsyncCommand;
use futures::executor::block_on;
use futures_lite::io::{AsyncReadExt as _, AsyncWriteExt as _};
use warpui::r#async::FutureExt as _;

use super::super::{
    ExpectedFileIdentity, ManagedChild, ManagedEnvironment, PreparedLaunchBinding, confirmed_exit,
    confirmed_exit_with_binding, create_generation_directory, create_launch_manifest,
    generation_directory, spawn, spawn_bound_update, supervisor_executable, write_new_record,
    write_spawn_attempt,
};
use super::*;

const FIXTURE_SOURCE: &str = r#"
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static void write_all(int fd, const char *p, size_t n) {
    while (n) { ssize_t k = write(fd, p, n); if (k < 0 && errno == EINTR) continue; if (k <= 0) _exit(84); p += k; n -= (size_t)k; }
}
static void beat(const char *directory, const char *name) {
    char path[4096], identity[64];
    snprintf(path, sizeof(path), "%s/%s.pid", directory, name);
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600); if (fd < 0) _exit(85);
    int n = snprintf(identity, sizeof(identity), "%d\n", getpid()); write_all(fd, identity, (size_t)n); close(fd);
    snprintf(path, sizeof(path), "%s/%s.heartbeat", directory, name);
    fd = open(path, O_WRONLY | O_CREAT | O_APPEND, 0600); if (fd < 0) _exit(86);
    time_t deadline = time(NULL) + 90;
    while (time(NULL) < deadline) { write_all(fd, "x", 1); usleep(50000); }
    close(fd); _exit(0);
}
int main(int argc, char **argv) {
    if (argc < 2) return 80;
    if (!strcmp(argv[1], "atomic-update")) {
        const char *install = getenv("CODEX_INSTALL_DIR");
        const char *release = getenv("CODEX_RELEASE");
        if (!install || !release) return 93;
        char target[PATH_MAX], executed[PATH_MAX], image[PATH_MAX];
        snprintf(target, sizeof(target), "%s/updated-version", install);
        snprintf(executed, sizeof(executed), "%s/executed-path", install);
        uint32_t image_size = sizeof(image);
        if (_NSGetExecutablePath(image, &image_size) != 0) return 94;
        int fd = open(target, O_WRONLY | O_CREAT | O_EXCL, 0600); if (fd < 0) return 95;
        write_all(fd, release, strlen(release)); close(fd);
        fd = open(executed, O_WRONLY | O_CREAT | O_EXCL, 0600); if (fd < 0) return 96;
        write_all(fd, image, strlen(image)); close(fd);
        return 0;
    }
    if (!strcmp(argv[1], "echo")) {
        char b[4096]; ssize_t n;
        while ((n = read(0, b, sizeof(b))) > 0) write_all(1, b, (size_t)n);
        if (n < 0) return 87;
        const char *tail = "\nEOF 完整\nPATH="; write_all(1, tail, strlen(tail));
        const char *path = getenv("PATH"); if (path) write_all(1, path, strlen(path));
        char output[8192], errors[8192]; memset(output, 'O', sizeof(output)); memset(errors, 'E', sizeof(errors));
        for (int i = 0; i < 256; ++i) { write_all(1, output, sizeof(output)); write_all(2, errors, sizeof(errors)); }
        const char *out_end = "\nstdout 尾部 end"; write_all(1, out_end, strlen(out_end));
        const char *err_end = "\nstderr 尾部 end"; write_all(2, err_end, strlen(err_end));
        return 37;
    }
    if (argc < 3) return 81;
    if (!strcmp(argv[1], "control")) beat(argv[2], "outside");
    if (strcmp(argv[1], "tree") || argc != 4) return 82;
    int count = atoi(argv[3]); if (count < 1 || count > 128) return 83;
    for (int i = 0; i < count; ++i) {
        pid_t child = fork(); if (child < 0) return 88;
        if (!child) {
            if (setsid() < 0) _exit(89);
            if (i % 2) { pid_t grandchild = fork(); if (grandchild < 0) _exit(90); if (grandchild) _exit(0); }
            char name[64]; snprintf(name, sizeof(name), "child-%03d", i); beat(argv[2], name);
        }
        if (i % 2) { int status; if (waitpid(child, &status, 0) != child) return 91; }
    }
    beat(argv[2], "root"); return 92;
}
"#;

#[test]
#[ignore = "需要新构建的真实 macOS worker 与本机 C 编译器；无模型"]
fn supervised_macos_atomic_snapshot_executes_snapshot_and_targets_original_install_root() {
    supervisor_executable().unwrap();
    // macOS 的 `/var` 是 `/private/var` 别名；原子合同只接受无链接的规范化路径。
    let directory = directory("infinishell-macos-atomic-update-")
        .canonicalize()
        .unwrap();
    let executable = build_fixture(&directory);
    let installation = directory.join("installation");
    fs::create_dir(&installation).unwrap();
    let generation = Uuid::new_v4();
    let binding = PreparedLaunchBinding::native_file("a".repeat(64)).unwrap();
    let expected = ExpectedFileIdentity::capture(&executable).unwrap();
    let original_sha256 = expected.sha256.clone();
    let environment = ManagedEnvironment {
        values: vec![
            (
                "CODEX_INSTALL_DIR".into(),
                installation.as_os_str().to_owned(),
            ),
            ("CODEX_RELEASE".into(), "0.155.1".into()),
            ("CODEX_NON_INTERACTIVE".into(), "1".into()),
        ],
        remove: Vec::new(),
    };
    let child = block_on(spawn_bound_update(
        &directory,
        generation,
        &executable,
        &[OsString::from("atomic-update")],
        &directory,
        environment,
        vec![expected],
        &binding,
    ))
    .unwrap();
    let receipt = block_on(child.finish_after_stdin_close()).unwrap();

    assert_eq!(receipt.exit_code, Some(0));
    assert_eq!(
        confirmed_exit_with_binding(&directory, generation, &binding).unwrap(),
        Some(receipt)
    );
    assert_eq!(
        fs::read(installation.join("updated-version")).unwrap(),
        b"0.155.1"
    );
    let executed = PathBuf::from(
        String::from_utf8(fs::read(installation.join("executed-path")).unwrap()).unwrap(),
    );
    assert!(
        executed.starts_with(
            directory
                .join("cli-agent-executable-snapshots")
                .join(generation.to_string())
        ),
        "实际执行路径必须是受控快照：{}",
        executed.display()
    );
    assert!(!executed.parent().unwrap().join("updated-version").exists());
    assert_eq!(
        ExpectedFileIdentity::capture(&executable).unwrap().sha256,
        original_sha256
    );
}

fn directory(name: &str) -> PathBuf {
    let directory = tempfile::Builder::new()
        .prefix(name)
        .tempdir()
        .unwrap()
        .keep();
    eprintln!("macOS 监督夹具证据：{}", directory.display());
    directory
}

fn build_fixture(directory: &Path) -> PathBuf {
    let source = directory.join("fixture.c");
    let executable = directory.join("fixture");
    fs::write(&source, FIXTURE_SOURCE).unwrap();
    let mut command = Command::new("/usr/bin/xcrun");
    command
        .args(["clang", "-Wall", "-Wextra", "-Werror", "-O0"])
        .arg(&source)
        .arg("-o")
        .arg(&executable);
    let errors = directory.join("clang.stderr");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(fs::File::create(&errors).unwrap()));
    let mut child = Outside(command.spawn().unwrap());
    wait_until("固定 C 夹具构建超时", || {
        child.0.try_wait().unwrap().is_some()
    });
    assert!(
        child.0.wait().unwrap().success(),
        "固定 C 夹具构建失败：{}",
        fs::read_to_string(errors).unwrap()
    );
    executable
}

fn wait_until(message: &str, mut test: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !test() {
        assert!(Instant::now() < deadline, "{message}");
        thread::sleep(Duration::from_millis(10));
    }
}

struct Outside(std::process::Child);

impl Drop for Outside {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn signal_verified(identity: MacosProcessIdentity, number: i32) {
    assert_eq!(macos_process_identity(identity.pid).unwrap(), identity);
    type Signal = unsafe extern "C" fn(*mut [u32; 8], i32) -> i32;
    let library = unsafe {
        libc::dlopen(
            c"/usr/lib/libproc.dylib".as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_LOCAL,
        )
    };
    assert!(!library.is_null());
    let address = unsafe { libc::dlsym(library, c"proc_signal_with_audittoken".as_ptr()) };
    assert!(!address.is_null());
    let signal: Signal = unsafe { std::mem::transmute(address) };
    let mut token = [0u32; 8];
    token[5] = identity.pid as u32;
    token[7] = identity.pid_version;
    let result = unsafe { signal(&mut token, number) };
    unsafe { libc::dlclose(library) };
    assert_eq!(result, 0);
}

async fn spawn_captured_output(
    directory: &Path,
    generation: Uuid,
    executable: &Path,
    errors: &Path,
) -> ManagedChild {
    // 产品默认丢弃诊断 stderr；这个下层真实 supervisor 验收单独保存它，以核对转发边界。
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let manifest = Manifest {
        isolated_home: None,
        environment: None,
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: listener.local_addr().unwrap(),
        executable: executable.to_owned(),
        arguments: vec![OsString::from("echo")],
        cwd: directory.to_owned(),
        expected_files: Vec::new(),
        atomic_launch_kind: None,
        atomic_cwd: None,
    };
    let (state, manifest_bytes) = create_launch_manifest(directory, &manifest).unwrap();
    write_spawn_attempt(&state, generation, &manifest_bytes).unwrap();
    let path = state.join("manifest.json");
    let mut command = AsyncCommand::new(supervisor_executable().unwrap());
    command
        .arg(WORKER_COMMAND)
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(fs::File::create(errors).unwrap()));
    let mut process = command.spawn().unwrap();
    let control = blocking::unblock(move || {
        let mut control = accept_authorized(&listener, &manifest).unwrap();
        let mut ready = [0];
        control.read_exact(&mut ready).unwrap();
        assert_eq!(ready, [1]);
        control
    })
    .await;
    ManagedChild {
        stdin: process.stdin.take(),
        stdout: process.stdout.take(),
        process: Some(process),
        control: Some(control),
        state_dir: directory.to_owned(),
        generation,
    }
}

#[test]
#[ignore = "需要新构建的真实 macOS worker 与本机 C 编译器；无模型"]
fn supervised_macos_stdin_eof_preserves_output_environment_and_exit_37() {
    supervisor_executable().unwrap();
    let directory = directory("infinishell-macos-eof-");
    let executable = build_fixture(&directory);
    let generation = Uuid::new_v4();
    let errors_path = directory.join("supervisor.stderr");
    let mut child = block_on(spawn_captured_output(
        &directory,
        generation,
        &executable,
        &errors_path,
    ));
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let input = "你好\nsecond line\n".as_bytes();
    block_on(stdin.write_all(input).with_timeout(Duration::from_secs(10)))
        .unwrap()
        .unwrap();
    drop(stdin);
    let mut output = Vec::new();
    block_on(
        stdout
            .read_to_end(&mut output)
            .with_timeout(Duration::from_secs(10)),
    )
    .unwrap()
    .unwrap();
    let mut expected = input.to_vec();
    expected.extend_from_slice("\nEOF 完整\nPATH=".as_bytes());
    expected.extend_from_slice(std::env::var_os("PATH").unwrap_or_default().as_bytes());
    expected.extend(std::iter::repeat_n(b'O', 2 * 1024 * 1024));
    expected.extend_from_slice("\nstdout 尾部 end".as_bytes());
    assert!(
        output == expected,
        "真实 worker 的 EOF 输出或原始 PATH 未完整保持"
    );
    let receipt = block_on(child.finish()).unwrap();
    let mut expected_errors = vec![b'E'; 2 * 1024 * 1024];
    expected_errors.extend_from_slice("\nstderr 尾部 end".as_bytes());
    assert!(
        fs::read(errors_path).unwrap() == expected_errors,
        "stderr 尾部必须完整转发"
    );
    assert_eq!(receipt.exit_code, Some(37));
    assert_eq!(receipt.containment, CONTAINMENT);
    assert!(receipt.cleanup_confirmed);
    assert_eq!(
        confirmed_exit(&directory, generation).unwrap(),
        Some(receipt)
    );
}

#[test]
#[ignore = "需要新构建的真实 macOS worker；90 个受控后代跨 setsid/双 fork，无模型"]
fn supervised_macos_cli_sigkill_cleans_more_than_eighty_detached_descendants() {
    supervisor_executable().unwrap();
    let directory = directory("infinishell-macos-many-");
    let executable = build_fixture(&directory);
    let generation = Uuid::new_v4();
    let mut outside_command = Command::new(&executable);
    outside_command
        .arg("control")
        .arg(&directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut outside = Outside(outside_command.spawn().unwrap());
    let arguments = [
        OsString::from("tree"),
        directory.as_os_str().to_owned(),
        OsString::from("90"),
    ];
    let child = block_on(spawn(
        &directory,
        generation,
        &executable,
        &arguments,
        &directory,
    ))
    .unwrap();
    let claim: Claim = serde_json::from_slice(
        &read_record(&generation_directory(&directory, generation).join(CLAIM_FILE)).unwrap(),
    )
    .unwrap();
    assert_ne!(
        macos_process_identity(outside.0.id() as i32)
            .unwrap()
            .resource_cid,
        claim.wrapper.resource_cid
    );
    let mut identities = Vec::new();
    for name in std::iter::once("root".to_owned()).chain((0..90).map(|i| format!("child-{i:03}"))) {
        let path = directory.join(format!("{name}.pid"));
        wait_until("受控后代没有发布完整身份", || {
            fs::read_to_string(&path).is_ok_and(|text| text.trim().parse::<i32>().is_ok())
        });
        let pid = fs::read_to_string(&path).unwrap().trim().parse().unwrap();
        let identity = macos_process_identity(pid).unwrap();
        assert_eq!(identity.resource_cid, claim.wrapper.resource_cid);
        identities.push(identity);
    }
    let queue = unsafe { libc::kqueue() };
    assert!(queue >= 0);
    let queue = unsafe { std::os::fd::OwnedFd::from_raw_fd(queue) };
    let changes = identities
        .iter()
        .map(|identity| libc::kevent {
            ident: identity.pid as usize,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_CLEAR,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                changes.as_ptr(),
                changes.len() as i32,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        },
        0
    );
    signal_verified(identities[0], libc::SIGKILL);
    let receipt = block_on(child.finish()).unwrap();
    assert!(receipt.cleanup_confirmed);
    assert_eq!(receipt.exit_code, None);
    assert_eq!(receipt.containment, CONTAINMENT);
    let mut exited = std::collections::BTreeSet::new();
    wait_until(
        "已销毁回执前的受控后代缺少真实退出事件",
        || {
            let mut events = vec![unsafe { std::mem::zeroed::<libc::kevent>() }; 128];
            let timeout = libc::timespec {
                tv_sec: 0,
                tv_nsec: 10_000_000,
            };
            let count = unsafe {
                libc::kevent(
                    queue.as_raw_fd(),
                    std::ptr::null(),
                    0,
                    events.as_mut_ptr(),
                    events.len() as i32,
                    &timeout,
                )
            };
            assert!(count >= 0);
            for event in events.into_iter().take(count as usize) {
                if event.fflags & libc::NOTE_EXIT != 0 {
                    exited.insert(event.ident as i32);
                }
            }
            exited.len() == identities.len()
        },
    );
    assert!(outside.0.try_wait().unwrap().is_none());
    let before = fs::metadata(directory.join("outside.heartbeat"))
        .unwrap()
        .len();
    wait_until("域外对照没有继续运行", || {
        fs::metadata(directory.join("outside.heartbeat"))
            .unwrap()
            .len()
            > before
    });
    assert!(MacosCoalition::is_destroyed(claim.wrapper.resource_cid, &claim.boot_session).unwrap());
}

#[test]
#[ignore = "需要真实 macOS worker；仅创建未授权私有 job，验证清理期限失败关闭"]
fn supervised_macos_live_domain_zero_timeout_never_writes_success() {
    let worker = supervisor_executable().unwrap();
    let directory = directory("infinishell-macos-timeout-");
    let generation = Uuid::new_v4();
    let state = create_generation_directory(&directory, generation).unwrap();
    let manifest = Manifest {
        isolated_home: None,
        environment: None,
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:12345".parse().unwrap(),
        executable: worker.clone(),
        arguments: Vec::new(),
        cwd: directory.clone(),
        expected_files: Vec::new(),
        atomic_launch_kind: None,
        atomic_cwd: None,
    };
    let path = state.join("manifest.json");
    let manifest_bytes = encode(&manifest).unwrap();
    write_new_record(&path, &manifest_bytes).unwrap();
    write_spawn_attempt(&state, generation, &manifest_bytes).unwrap();
    let mut job = Job::create(&path, &manifest, &worker).unwrap();
    let (control, identity) = job.accept(&manifest, b'c').unwrap();
    job.coalition = Some(MacosCoalition::claim(identity).unwrap());
    assert_eq!(
        job.coalition
            .as_ref()
            .unwrap()
            .terminate_and_confirm(Duration::ZERO)
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
    assert!(!state.join(PROOF_FILE).exists());
    assert!(confirmed_exit(&directory, generation).unwrap().is_none());
    drop(control);
    job.cleanup().unwrap();
}

#[test]
#[ignore = "需要真实 macOS worker；注入服务移除失败，真实清理未授权 wrapper，无模型"]
fn supervised_macos_job_removal_failure_still_stops_its_claimed_wrapper() {
    let worker = supervisor_executable().unwrap();
    let directory = directory("infinishell-macos-removal-");
    let generation = Uuid::new_v4();
    let state = create_generation_directory(&directory, generation).unwrap();
    let manifest = Manifest {
        isolated_home: None,
        environment: None,
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:12345".parse().unwrap(),
        executable: worker.clone(),
        arguments: Vec::new(),
        cwd: directory.clone(),
        expected_files: Vec::new(),
        atomic_launch_kind: None,
        atomic_cwd: None,
    };
    let path = state.join("manifest.json");
    let manifest_bytes = encode(&manifest).unwrap();
    write_new_record(&path, &manifest_bytes).unwrap();
    write_spawn_attempt(&state, generation, &manifest_bytes).unwrap();
    let mut job = Job::create(&path, &manifest, &worker).unwrap();
    let (control, identity) = job.accept(&manifest, b'c').unwrap();
    job.coalition = Some(MacosCoalition::claim(identity).unwrap());
    let queue = unsafe { libc::kqueue() };
    assert!(queue >= 0);
    let queue = unsafe { std::os::fd::OwnedFd::from_raw_fd(queue) };
    let event = libc::kevent {
        ident: identity.pid as usize,
        filter: libc::EVFILT_PROC,
        flags: libc::EV_ADD | libc::EV_ONESHOT,
        fflags: libc::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    assert_eq!(
        unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                &event,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        },
        0
    );
    assert_eq!(macos_process_identity(identity.pid).unwrap(), identity);
    // 移除操作在本测试中注入失败；域成员发现、身份绑定信号和退出观察均使用真实系统接口。
    let error = cleanup_operations(
        || Err(io::Error::other("固定 bootout 故障注入")),
        || {
            job.coalition
                .as_ref()
                .unwrap()
                .terminate_and_confirm(Duration::from_secs(2))
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("固定 bootout 故障注入"));
    let mut exited = unsafe { std::mem::zeroed::<libc::kevent>() };
    let timeout = libc::timespec {
        tv_sec: 2,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut exited,
                1,
                &timeout,
            )
        },
        1
    );
    let (pid, notes, flags) = (exited.ident, exited.fflags, exited.flags);
    assert_eq!(pid, identity.pid as usize);
    assert_eq!(flags & libc::EV_ERROR, 0);
    assert_ne!(notes & libc::NOTE_EXIT, 0);
    assert!(job.registered, "注入移除失败不能冒充 job 已移除");
    assert!(!state.join(PROOF_FILE).exists());
    assert!(confirmed_exit(&directory, generation).unwrap().is_none());
    drop(control);
    job.cleanup().unwrap();
}
