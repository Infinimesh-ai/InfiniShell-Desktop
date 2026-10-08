//! 固定 x86_64 musl ELF 的单加载器启动闭包；主程序仍由密封 memfd 执行。
//! musl 在目录搜索前把 libc.* 解析为自身，不能据此放行其他启动依赖。

use std::ffi::CStr;
use std::fs::File;
use std::io;
use std::path::Path;

use super::MAX_NATIVE_EXECUTABLE_BYTES;
use super::glibc::{BoundFile, parse_elf_with_interpreter, system_library_path};

pub(super) const INTERPRETER: &CStr = c"/lib/ld-musl-x86_64.so.1";
const LIBC: &str = "libc.musl-x86_64.so.1";

#[derive(Debug)]
pub(super) struct SystemClosure {
    loader: BoundFile,
}

impl SystemClosure {
    pub(super) fn verify(&self) -> io::Result<()> {
        self.loader
            .verify()
            .map_err(|_| io::Error::other("managed_process.linux_musl_binding_changed"))
    }
}

fn verify_main(file: &File, size: u64) -> io::Result<()> {
    let interpreter = INTERPRETER.to_str().expect("固定解释器路径为 ASCII");
    let image = parse_elf_with_interpreter(file, size, interpreter)
        .map_err(|_| io::Error::other("managed_process.linux_musl_elf_invalid"))?;
    if image.interpreter.as_deref() != Some(interpreter) || image.needed != [LIBC] {
        return Err(io::Error::other(
            "managed_process.linux_musl_dependency_unbound",
        ));
    }
    Ok(())
}

fn verify_loader(file: &File, size: u64) -> io::Result<()> {
    let image = parse_elf_with_interpreter(
        file,
        size,
        INTERPRETER.to_str().expect("固定解释器路径为 ASCII"),
    )
    .map_err(|_| io::Error::other("managed_process.linux_musl_loader_invalid"))?;
    // 官方 libc.so 同时承担解释器，构建不强制 SONAME；禁止引入第二层解释器或库。
    if image.executable_type != 3
        || image.interpreter.is_some()
        || !image.needed.is_empty()
        || !matches!(image.soname.as_deref(), None | Some("libc.so") | Some(LIBC))
    {
        return Err(io::Error::other(
            "managed_process.linux_musl_loader_invalid",
        ));
    }
    Ok(())
}

pub(super) fn prepare(main: &File, size: u64) -> io::Result<SystemClosure> {
    verify_main(main, size)?;
    let loader = BoundFile::capture(
        Path::new(INTERPRETER.to_str().expect("固定解释器路径为 ASCII")),
        MAX_NATIVE_EXECUTABLE_BYTES,
    )
    .map_err(|_| io::Error::other("managed_process.linux_musl_system_file_untrusted"))?;
    if !system_library_path(&loader.canonical) {
        return Err(io::Error::other(
            "managed_process.linux_musl_system_file_untrusted",
        ));
    }
    verify_loader(&loader.file, loader.identity.2)?;
    let closure = SystemClosure { loader };
    closure.verify()?;
    Ok(closure)
}

#[cfg(test)]
#[path = "managed_process_atomic_linux_musl_tests.rs"]
mod tests;
