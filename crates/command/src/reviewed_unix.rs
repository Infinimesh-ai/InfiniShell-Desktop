//! 受审项目命令的 Unix 执行边界；不创建沙箱，整树回收仍由外层独立监督代负责。

use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

/// 调用方必须持有已核对的映像和 cwd，并在最后回调重新核对获批源文件摘要。
/// Linux 使用原 vnode 的 fd，不搬走 Cargo/Python/Node 的安装位置；动态加载器和
/// 项目代码按当前账号权限运行，不套用更新探针的静态依赖白名单。
/// macOS 在任何用户态指令运行前核对挂起映像的映射 vnode 与 cwd，再恢复本次子进程。
pub fn execute(
    program: &Path,
    image: &File,
    directory: &File,
    argv0: &OsStr,
    arguments: &[OsString],
    environment: &[(OsString, OsString)],
    verify_sources: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    if !program.is_absolute()
        || argv0.is_empty()
        || !image.metadata()?.is_file()
        || !directory.metadata()?.is_dir()
    {
        return Err(io::Error::other("受审命令的映像或工作目录无效"));
    }
    let args = std::iter::once(argv0)
        .chain(arguments.iter().map(OsString::as_os_str))
        .map(|value| CString::new(value.as_bytes()).map_err(io::Error::other))
        .collect::<io::Result<Vec<_>>>()?;
    let env = environment
        .iter()
        .map(|(key, value)| {
            if key.is_empty() || key.as_bytes().contains(&b'=') {
                return Err(io::Error::other("受审命令环境键无效"));
            }
            let mut bytes = key.as_bytes().to_vec();
            bytes.push(b'=');
            bytes.extend_from_slice(value.as_bytes());
            CString::new(bytes).map_err(io::Error::other)
        })
        .collect::<io::Result<Vec<_>>>()?;
    // 此函数仅在单独的 exec worker 中运行，fchdir 不会改变 GUI 或其他任务的 cwd。
    if unsafe { libc::fchdir(directory.as_raw_fd()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        verify_sources()?;
        let null = File::open("/dev/null")?;
        if unsafe { libc::dup2(null.as_raw_fd(), libc::STDIN_FILENO) } < 0
            || unsafe { libc::dup2(libc::STDOUT_FILENO, libc::STDERR_FILENO) } < 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut argv = args.iter().map(|value| value.as_ptr()).collect::<Vec<_>>();
        argv.push(std::ptr::null());
        let mut envp = env.iter().map(|value| value.as_ptr()).collect::<Vec<_>>();
        envp.push(std::ptr::null());
        // 仅接受原生 ELF；不允许 fexecve 对 shebang 的解释器回退，也不按 PATH 重选入口。
        unsafe { libc::fexecve(image.as_raw_fd(), argv.as_ptr(), envp.as_ptr()) };
        return Err(io::Error::last_os_error());
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return macos::execute(program, image, directory, &args, &env, verify_sources);
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64")
    )))]
    {
        let _ = (args, env, verify_sources);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "受审 Unix 命令尚未绑定此平台",
        ))
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos {
    use super::*;
    use crate::managed::{
        MacosProcessIdentity, macos_boot_session, macos_process_identity,
        macos_signal_owned_process,
    };
    use std::mem::{self, MaybeUninit};
    use std::os::unix::fs::MetadataExt as _;

    // 固定 Apple XNU xnu-12377.1.9 的 bsd/sys/proc_info.h；不把 pathname 视为 vnode 身份。
    #[repr(C)]
    struct Region {
        protection: u32,
        max_protection: u32,
        inheritance: u32,
        flags: u32,
        offset: u64,
        behavior: u32,
        user_wired_count: u32,
        user_tag: u32,
        pages_resident: u32,
        pages_shared_now_private: u32,
        pages_swapped_out: u32,
        pages_dirtied: u32,
        ref_count: u32,
        shadow_depth: u32,
        share_mode: u32,
        private_pages_resident: u32,
        shared_pages_resident: u32,
        obj_id: u32,
        depth: u32,
        address: u64,
        size: u64,
    }
    #[repr(C)]
    struct RegionPath {
        region: Region,
        vnode: libc::vnode_info_path,
    }

    fn error(code: libc::c_int) -> io::Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code))
        }
    }
    fn info<T>(pid: i32, flavor: i32, address: u64) -> io::Result<T> {
        let mut value = MaybeUninit::<T>::zeroed();
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                flavor,
                address,
                value.as_mut_ptr().cast(),
                mem::size_of::<T>() as i32,
            )
        };
        if read != mem::size_of::<T>() as i32 {
            return Err(io::Error::other("挂起命令的内核信息不完整"));
        }
        Ok(unsafe { value.assume_init() })
    }
    struct Attributes(libc::posix_spawnattr_t);
    impl Drop for Attributes {
        fn drop(&mut self) {
            unsafe { libc::posix_spawnattr_destroy(&mut self.0) };
        }
    }
    struct Actions(libc::posix_spawn_file_actions_t);
    impl Drop for Actions {
        fn drop(&mut self) {
            unsafe { libc::posix_spawn_file_actions_destroy(&mut self.0) };
        }
    }

    fn wait(pid: i32) -> io::Result<i32> {
        let mut status = 0;
        loop {
            let value = unsafe { libc::waitpid(pid, &mut status, 0) };
            if value == pid {
                return Ok(status);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error);
            }
        }
    }

    fn verify_image(
        identity: MacosProcessIdentity,
        program: &Path,
        image: &File,
        directory: &File,
    ) -> io::Result<()> {
        let before = macos_process_identity(identity.pid)?;
        if before != identity {
            return Err(io::Error::other("挂起命令进程身份已改变"));
        }
        let mut path = [0u8; 4096];
        let count = unsafe {
            libc::proc_pidpath(identity.pid, path.as_mut_ptr().cast(), path.len() as u32)
        };
        if count <= 0 || count as usize >= path.len() {
            return Err(io::Error::other("挂起命令映像路径不可验证"));
        }
        let value = &path[..count as usize];
        let value = value.strip_suffix(&[0]).unwrap_or(value);
        if Path::new(OsStr::from_bytes(value)).canonicalize()? != program {
            return Err(io::Error::other("实际派生映像与审批不一致"));
        }
        let expected = image.metadata()?;
        let cwd = directory.metadata()?;
        let actual_cwd: libc::proc_vnodepathinfo = info(identity.pid, 9, 0)?;
        if u64::from(actual_cwd.pvi_cdir.vip_vi.vi_stat.vst_dev) != cwd.dev()
            || actual_cwd.pvi_cdir.vip_vi.vi_stat.vst_ino != cwd.ino()
        {
            return Err(io::Error::other("实际派生工作目录与审批不一致"));
        }
        let mut address = 0u64;
        let mut found = false;
        // fill_procregioninfo 返回包含地址或之后首个 region；长度与地址必须严格前进。
        for _ in 0..4096 {
            let region: RegionPath = info(identity.pid, 8, address)?;
            let next = region
                .region
                .address
                .checked_add(region.region.size)
                .filter(|next| *next > address)
                .ok_or_else(|| io::Error::other("挂起映像区域没有前进"))?;
            let stat = &region.vnode.vip_vi.vi_stat;
            if region.region.protection & 4 != 0
                && region.vnode.vip_vi.vi_type == 1
                && u64::from(stat.vst_dev) == expected.dev()
                && stat.vst_ino == expected.ino()
                && u64::try_from(stat.vst_size).ok() == Some(expected.len())
            {
                found = true;
                break;
            }
            address = next;
        }
        if !found || macos_process_identity(identity.pid)? != identity {
            return Err(io::Error::other("未找到获批 vnode 的实际可执行映射"));
        }
        Ok(())
    }

    pub(super) fn execute(
        program: &Path,
        image: &File,
        directory: &File,
        args: &[CString],
        env: &[CString],
        verify_sources: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        let boot = macos_boot_session()?;
        let program = CString::new(program.as_os_str().as_bytes()).map_err(io::Error::other)?;
        let mut attrs = MaybeUninit::uninit();
        error(unsafe { libc::posix_spawnattr_init(attrs.as_mut_ptr()) })?;
        let mut attrs = Attributes(unsafe { attrs.assume_init() });
        let mut actions = MaybeUninit::uninit();
        error(unsafe { libc::posix_spawn_file_actions_init(actions.as_mut_ptr()) })?;
        let mut actions = Actions(unsafe { actions.assume_init() });
        error(unsafe {
            libc::posix_spawnattr_setflags(
                &mut attrs.0,
                (libc::POSIX_SPAWN_START_SUSPENDED | libc::POSIX_SPAWN_CLOEXEC_DEFAULT) as i16,
            )
        })?;
        let mut architecture: libc::cpu_type_t = 0x0100_000c;
        let mut accepted = 0;
        error(unsafe {
            libc::posix_spawnattr_setbinpref_np(&mut attrs.0, 1, &mut architecture, &mut accepted)
        })?;
        if accepted != 1 {
            return Err(io::Error::other("挂起命令无法固定 ARM64 映像"));
        }
        error(unsafe {
            libc::posix_spawn_file_actions_addopen(
                &mut actions.0,
                0,
                c"/dev/null".as_ptr(),
                libc::O_RDONLY,
                0,
            )
        })?;
        error(unsafe { libc::posix_spawn_file_actions_adddup2(&mut actions.0, 1, 1) })?;
        error(unsafe { libc::posix_spawn_file_actions_adddup2(&mut actions.0, 1, 2) })?;
        let mut argv = args
            .iter()
            .map(|value| value.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        argv.push(std::ptr::null_mut());
        let mut envp = env
            .iter()
            .map(|value| value.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        envp.push(std::ptr::null_mut());
        let mut pid = 0;
        error(unsafe {
            libc::posix_spawn(
                &mut pid,
                program.as_ptr(),
                &actions.0,
                &attrs.0,
                argv.as_ptr(),
                envp.as_ptr(),
            )
        })?;
        // START_SUSPENDED 由内核在返回用户态前设置；普通 fork 后 SIGSTOP 不能替代。
        let identity = macos_process_identity(pid)?;
        let checked = verify_image(
            identity,
            Path::new(OsStr::from_bytes(program.as_bytes())),
            image,
            directory,
        )
        .and_then(|()| verify_sources())
        .and_then(|()| macos_signal_owned_process(identity, &boot, libc::SIGCONT));
        if let Err(error) = checked {
            // 失败只信号本次已获身份的子进程；未知后代交由外层 coalition 收据判断。
            macos_signal_owned_process(identity, &boot, libc::SIGKILL)?;
            wait(pid)?;
            return Err(error);
        }
        let status = wait(pid)?;
        if libc::WIFEXITED(status) {
            let code = libc::WEXITSTATUS(status);
            if code == 0 {
                return Ok(());
            }
            std::process::exit(code);
        }
        if libc::WIFSIGNALED(status) {
            std::process::exit(128 + libc::WTERMSIG(status));
        }
        Err(io::Error::other("受审命令退出状态不可确认"))
    }
}
