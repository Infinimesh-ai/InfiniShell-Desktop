//! 有界的 x86_64 glibc 启动依赖闭包。主程序仍由父模块的 sealed memfd 执行。
//! 系统 loader/cache/库信任 root 管理的 OS；打开 fd 不阻止 root 替换系统文件。
//! 每个依赖必须有 cache 基线，不能回退未绑定的默认搜索目录。

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::{CStr, CString};
use std::fs::{File, Metadata};
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::unix::fs::{FileExt as _, MetadataExt as _};
use std::path::{Component, Path, PathBuf};

use super::{MAX_NATIVE_EXECUTABLE_BYTES, open_at, sha256_file};

const INTERPRETER: &str = "/lib64/ld-linux-x86-64.so.2";
const CACHE: &str = "/etc/ld.so.cache";
const MAX_TABLE: usize = 1024 * 1024;
const MAX_CACHE: u64 = 16 * 1024 * 1024;
const MAX_OBJECTS: usize = 128;
const HWCAP_EXTENSION: u64 = 1 << 62;

fn invalid() -> io::Error {
    failure("managed_process.linux_glibc_format_invalid")
}

fn failure(code: &'static str) -> io::Error {
    io::Error::other(code)
}

#[derive(Debug)]
struct BoundFile {
    path: PathBuf,
    canonical: PathBuf,
    file: File,
    identity: (u64, u64, u64, u32),
    sha256: String,
}

impl BoundFile {
    fn capture(path: &Path, limit: u64) -> io::Result<Self> {
        let (mut file, canonical) = open_system_file(path)?;
        let metadata = file.metadata()?;
        if metadata.len() == 0 || metadata.len() > limit {
            return Err(invalid());
        }
        let identity = identity(&metadata);
        let sha256 = sha256_file(&mut file)?;
        if identity != self::identity(&file.metadata()?) {
            return Err(invalid());
        }
        Ok(Self {
            path: path.to_owned(),
            canonical,
            file,
            identity,
            sha256,
        })
    }

    fn verify(&self) -> io::Result<()> {
        let (mut current, canonical) = open_system_file(&self.path)?;
        if canonical != self.canonical
            || identity(&current.metadata()?) != self.identity
            || identity(&self.file.metadata()?) != self.identity
            || sha256_file(&mut current)? != self.sha256
        {
            return Err(invalid());
        }
        Ok(())
    }
}

fn identity(metadata: &Metadata) -> (u64, u64, u64, u32) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mode(),
    )
}

#[derive(Debug)]
pub(super) struct SystemClosure {
    files: Vec<BoundFile>,
}

impl SystemClosure {
    pub(super) fn verify(&self) -> io::Result<()> {
        require_no_preload()?;
        for file in &self.files {
            file.verify()
                .map_err(|_| failure("managed_process.linux_glibc_binding_changed"))?;
        }
        Ok(())
    }
}

pub(super) fn prepare(main: &File, size: u64) -> io::Result<SystemClosure> {
    let image =
        parse_elf(main, size).map_err(|_| failure("managed_process.linux_glibc_elf_invalid"))?;
    if image.interpreter.as_deref() != Some(INTERPRETER) {
        return Err(failure("managed_process.linux_glibc_elf_invalid"));
    }
    require_no_preload()?;
    let cache = capture_system(Path::new(CACHE), MAX_CACHE)?;
    let cache_bytes = read(&cache.file, 0, cache.identity.2 as usize, cache.identity.2)?;
    let candidates = parse_cache(&cache_bytes)
        .map_err(|_| failure("managed_process.linux_glibc_cache_invalid"))?;
    let loader = capture_system(Path::new(INTERPRETER), MAX_NATIVE_EXECUTABLE_BYTES)?;
    if !system_library_path(&loader.canonical) {
        return Err(failure("managed_process.linux_glibc_system_file_untrusted"));
    }
    let loader_image = parse_elf(&loader.file, loader.identity.2)
        .map_err(|_| failure("managed_process.linux_glibc_elf_invalid"))?;
    if loader_image.soname.as_deref() != Some("ld-linux-x86-64.so.2")
        || loader_image.interpreter.is_some()
        || !loader_image.needed.is_empty()
    {
        return Err(failure("managed_process.linux_glibc_soname_mismatch"));
    }
    let kernel_version = kernel_version()?;
    let mut files = vec![cache, loader];
    let mut pending = VecDeque::from(image.needed);
    // 内核直接装入的解释器也要绑定 cache 映射，不能因已装入而略过身份校验。
    pending.push_back("ld-linux-x86-64.so.2".to_owned());
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop_front() {
        if !visited.insert(name.clone()) {
            continue;
        }
        if visited.len() > MAX_OBJECTS {
            return Err(invalid());
        }
        for entry in dependency_candidates(&candidates, &name, kernel_version)? {
            let file = capture_system(&entry.path, MAX_NATIVE_EXECUTABLE_BYTES)?;
            if !system_library_path(&file.canonical) {
                return Err(failure("managed_process.linux_glibc_system_file_untrusted"));
            }
            let library = parse_elf(&file.file, file.identity.2)
                .map_err(|_| failure("managed_process.linux_glibc_elf_invalid"))?;
            if library.soname.as_deref() != Some(name.as_str()) {
                return Err(failure("managed_process.linux_glibc_soname_mismatch"));
            }
            pending.extend(library.needed);
            // 即使不同 cache 路径指向同一 inode，也保留路径绑定，防止漏复核别名。
            if !files.iter().any(|bound| bound.path == file.path) {
                files.push(file);
            }
            if files.len() > MAX_OBJECTS {
                return Err(invalid());
            }
        }
    }
    let closure = SystemClosure { files };
    closure.verify()?;
    Ok(closure)
}

fn capture_system(path: &Path, limit: u64) -> io::Result<BoundFile> {
    BoundFile::capture(path, limit)
        .map_err(|_| failure("managed_process.linux_glibc_system_file_untrusted"))
}

fn dependency_candidates<'a>(
    candidates: &'a BTreeMap<String, Vec<Candidate>>,
    name: &str,
    kernel_version: u32,
) -> io::Result<&'a [Candidate]> {
    let entries = candidates
        .get(name)
        .ok_or_else(|| failure("managed_process.linux_glibc_cache_baseline_missing"))?;
    if !entries
        .iter()
        .any(|entry| entry.baseline && entry.minimum_kernel <= kernel_version)
    {
        return Err(failure(
            "managed_process.linux_glibc_cache_baseline_missing",
        ));
    }
    for entry in entries {
        if !system_library_path(&entry.path) || normalize(&entry.path)? != entry.path {
            return Err(failure("managed_process.linux_glibc_system_file_untrusted"));
        }
    }
    Ok(entries)
}

fn kernel_version() -> io::Result<u32> {
    let mut value = std::mem::MaybeUninit::<libc::utsname>::uninit();
    if unsafe { libc::uname(value.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let value = unsafe { value.assume_init() };
    let release = unsafe { CStr::from_ptr(value.release.as_ptr()) }
        .to_str()
        .map_err(|_| invalid())?;
    let mut result = 0;
    let mut parts = release.split('.');
    for shift in [16, 8, 0] {
        let part = parts.next().ok_or_else(invalid)?;
        let digits = part.bytes().take_while(u8::is_ascii_digit).count();
        let number = part[..digits].parse::<u32>().map_err(|_| invalid())?;
        if number > 255 {
            return Err(invalid());
        }
        result |= number << shift;
    }
    Ok(result)
}

fn require_no_preload() -> io::Result<()> {
    match open_system_file(Path::new("/etc/ld.so.preload")) {
        Err(failure) if failure.kind() == io::ErrorKind::NotFound => Ok(()),
        // 即使空文件也拒绝：此支持域不接受系统预加载配置。
        Ok(_) => Err(failure("managed_process.linux_glibc_preload_present")),
        Err(failure) => Err(failure),
    }
}

fn system_library_path(path: &Path) -> bool {
    ["/lib", "/lib64", "/usr/lib", "/usr/lib64"]
        .iter()
        .any(|root| path.starts_with(root))
}

/// 按 fd 遍历每个系统目录；支持发行版 usr-merge 链接，但不接受用户拥有的链接/祖先。
fn open_system_file(path: &Path) -> io::Result<(File, PathBuf)> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    let mut names = path_components(path)?;
    let mut directory = open_at(libc::AT_FDCWD, c"/", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    require_root(&directory.metadata()?, true)?;
    let mut resolved = PathBuf::from("/");
    let mut links = 0;
    while let Some(name) = names.pop_front() {
        // 必须在已解析的真实目录上处理 ..，不能在解析 symlink 前词法折叠。
        if name == ".." {
            if !resolved.pop() {
                return Err(invalid());
            }
            directory = open_at(
                directory.as_raw_fd(),
                c"..",
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )?;
            require_root(&directory.metadata()?, true)?;
            continue;
        }
        let c_name = CString::new(name.as_bytes()).map_err(|_| invalid())?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                c_name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let stat = unsafe { stat.assume_init() };
        if stat.st_uid != 0 {
            return Err(invalid());
        }
        if stat.st_mode & libc::S_IFMT == libc::S_IFLNK {
            links += 1;
            if links > 40 {
                return Err(invalid());
            }
            let mut buffer = [0_u8; 4096];
            let count = unsafe {
                libc::readlinkat(
                    directory.as_raw_fd(),
                    c_name.as_ptr(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if count < 0 {
                return Err(io::Error::last_os_error());
            }
            if count == 0 || count as usize == buffer.len() {
                return Err(invalid());
            }
            let target = PathBuf::from(std::ffi::OsString::from_vec(
                buffer[..count as usize].to_vec(),
            ));
            if target.is_absolute() {
                directory = open_at(libc::AT_FDCWD, c"/", libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                require_root(&directory.metadata()?, true)?;
                resolved = PathBuf::from("/");
            }
            let mut target_names = path_components(&target)?;
            target_names.append(&mut names);
            names = target_names;
            continue;
        }
        let last = names.is_empty();
        let kind = if last { libc::S_IFREG } else { libc::S_IFDIR };
        if stat.st_mode & libc::S_IFMT != kind || stat.st_mode & 0o022 != 0 {
            return Err(invalid());
        }
        let file = open_at(
            directory.as_raw_fd(),
            &c_name,
            libc::O_RDONLY | if last { 0 } else { libc::O_DIRECTORY },
            0,
        )?;
        let metadata = file.metadata()?;
        require_root(&metadata, !last)?;
        if metadata.dev() != stat.st_dev || metadata.ino() != stat.st_ino {
            return Err(invalid());
        }
        resolved.push(name);
        if last {
            return Ok((file, resolved));
        }
        directory = file;
    }
    Err(invalid())
}

fn path_components(path: &Path) -> io::Result<VecDeque<std::ffi::OsString>> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(Ok(name.to_owned())),
            Component::ParentDir => Some(Ok("..".into())),
            Component::RootDir | Component::CurDir => None,
            Component::Prefix(_) => Some(Err(invalid())),
        })
        .collect()
}

fn require_root(metadata: &Metadata, directory: bool) -> io::Result<()> {
    if metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(invalid());
    }
    Ok(())
}

fn normalize(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid());
    }
    let mut result = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => result.push(name),
            Component::ParentDir => {
                if !result.pop() {
                    return Err(invalid());
                }
            }
            Component::Prefix(_) => return Err(invalid()),
        }
    }
    Ok(result)
}

#[derive(Debug)]
struct ElfImage {
    interpreter: Option<String>,
    soname: Option<String>,
    needed: Vec<String>,
}

fn parse_elf(file: &File, size: u64) -> io::Result<ElfImage> {
    let header = read(file, 0, 64, size)?;
    if &header[..7] != b"\x7fELF\x02\x01\x01"
        || u16_at(&header, 18)? != 62
        || !matches!(header[7], 0 | 3)
        || header[8] != 0
        || u32_at(&header, 20)? != 1
        || !matches!(u16_at(&header, 16)?, 2 | 3)
    {
        return Err(invalid());
    }
    let offset = u64_at(&header, 32)?;
    let count = usize::from(u16_at(&header, 56)?);
    if u16_at(&header, 54)? != 56 || count == 0 || count > 4096 {
        return Err(invalid());
    }
    let headers = read(file, offset, count * 56, size)?;
    let mut loads = Vec::new();
    let mut load_pages = Vec::new();
    let mut dynamic = None;
    let mut interpreter = None;
    for header in headers.chunks_exact(56) {
        let offset = u64_at(header, 8)?;
        let vaddr = u64_at(header, 16)?;
        let bytes = u64_at(header, 32)?;
        if offset.checked_add(bytes).is_none_or(|end| end > size) {
            return Err(invalid());
        }
        match u32_at(header, 0)? {
            1 => {
                let memory = u64_at(header, 40)?;
                let alignment = u64_at(header, 48)?;
                if memory < bytes
                    || offset % 4096 != vaddr % 4096
                    || (alignment > 1
                        && (!alignment.is_power_of_two()
                            || offset % alignment != vaddr % alignment))
                {
                    return Err(invalid());
                }
                let start = vaddr & !4095;
                let end = vaddr
                    .checked_add(memory)
                    .and_then(|value| value.checked_add(4095))
                    .ok_or_else(invalid)?
                    & !4095;
                if load_pages
                    .iter()
                    .any(|(base, limit)| start < *limit && *base < end)
                {
                    return Err(invalid());
                }
                load_pages.push((start, end));
                loads.push((vaddr, offset, bytes));
            }
            2 => {
                if dynamic.replace((vaddr, offset, bytes)).is_some() {
                    return Err(invalid());
                }
            }
            3 => {
                if interpreter.is_some() || bytes == 0 || bytes > 4096 {
                    return Err(invalid());
                }
                let value = read(file, offset, bytes as usize, size)?;
                if value != b"/lib64/ld-linux-x86-64.so.2\0" {
                    return Err(invalid());
                }
                interpreter = Some(INTERPRETER.to_owned());
            }
            0x6474_e551 if u32_at(header, 4)? & 1 != 0 => return Err(invalid()),
            _ => {}
        }
    }
    let (dynamic_vaddr, offset, bytes) = dynamic.ok_or_else(invalid)?;
    if mapped_offset(&loads, dynamic_vaddr, bytes)? != offset {
        return Err(invalid());
    }
    if bytes == 0 || bytes > MAX_TABLE as u64 || bytes % 16 != 0 {
        return Err(invalid());
    }
    let table = read(file, offset, bytes as usize, size)?;
    let mut strtab = None;
    let mut strsz = None;
    let mut needed = Vec::new();
    let mut soname = None;
    let mut terminated = false;
    for entry in table.chunks_exact(16) {
        let tag = u64_at(entry, 0)?;
        let value = u64_at(entry, 8)?;
        match tag {
            0 => {
                terminated = true;
                break;
            }
            1 => needed.push(value),
            5 => {
                if strtab.replace(value).is_some() {
                    return Err(invalid());
                }
            }
            10 => {
                if strsz.replace(value).is_some() {
                    return Err(invalid());
                }
            }
            14 => {
                if soname.replace(value).is_some() {
                    return Err(invalid());
                }
            }
            // 路径、审计、过滤器会增加未绑定对象或改变解析来源。
            15 | 29 | 0x6fff_fefa | 0x6fff_fefb | 0x6fff_fefc | 0x7fff_fffd | 0x7fff_ffff => {
                return Err(invalid());
            }
            // 禁止 NODEFLIB 改变缓存/默认目录语义；其余标志不新增路径来源。
            0x6fff_fffb if value & 0x800 != 0 => return Err(invalid()),
            _ => {}
        }
    }
    let (vaddr, length) = (strtab.ok_or_else(invalid)?, strsz.ok_or_else(invalid)?);
    if !terminated || needed.len() > MAX_OBJECTS || length == 0 || length > MAX_TABLE as u64 {
        return Err(invalid());
    }
    let offset = mapped_offset(&loads, vaddr, length)?;
    let strings = read(file, offset, length as usize, size)?;
    let needed = needed
        .into_iter()
        .map(|offset| soname_at(&strings, offset))
        .collect::<io::Result<_>>()?;
    let soname = soname
        .map(|offset| soname_at(&strings, offset))
        .transpose()?;
    Ok(ElfImage {
        interpreter,
        soname,
        needed,
    })
}

fn mapped_offset(loads: &[(u64, u64, u64)], vaddr: u64, length: u64) -> io::Result<u64> {
    let mut mappings = loads.iter().filter_map(|(base, offset, bytes)| {
        let relative = vaddr.checked_sub(*base)?;
        (relative.checked_add(length)? <= *bytes)
            .then(|| offset.checked_add(relative))
            .flatten()
    });
    let offset = mappings.next().ok_or_else(invalid)?;
    if mappings.next().is_some() {
        return Err(invalid());
    }
    Ok(offset)
}

fn soname_at(bytes: &[u8], offset: u64) -> io::Result<String> {
    let tail = bytes
        .get(usize::try_from(offset).map_err(|_| invalid())?..)
        .ok_or_else(invalid)?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(invalid)?;
    let name = std::str::from_utf8(&tail[..length]).map_err(|_| invalid())?;
    if name.is_empty()
        || name.len() > 255
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
    {
        return Err(invalid());
    }
    Ok(name.to_owned())
}

fn read(file: &File, offset: u64, length: usize, size: u64) -> io::Result<Vec<u8>> {
    if offset
        .checked_add(length as u64)
        .is_none_or(|end| end > size)
    {
        return Err(invalid());
    }
    let mut bytes = vec![0; length];
    file.read_exact_at(&mut bytes, offset)?;
    Ok(bytes)
}
fn u16_at(bytes: &[u8], at: usize) -> io::Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> io::Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ))
}
fn u64_at(bytes: &[u8], at: usize) -> io::Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(at..at + 8)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ))
}

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    baseline: bool,
    minimum_kernel: u32,
}

fn parse_cache(bytes: &[u8]) -> io::Result<BTreeMap<String, Vec<Candidate>>> {
    if bytes.len() < 48
        || bytes.len() as u64 > MAX_CACHE
        || &bytes[..20] != b"glibc-ld.so.cache1.1"
        || !matches!(bytes[28], 0 | 2)
    {
        return Err(invalid());
    }
    let count = u32_at(bytes, 20)? as usize;
    if count > 65536 {
        return Err(invalid());
    }
    let strings_start = 48 + count * 24;
    let strings_end = strings_start
        .checked_add(u32_at(bytes, 24)? as usize)
        .ok_or_else(invalid)?;
    if strings_end > bytes.len() {
        return Err(invalid());
    }
    let string = |offset: u32| -> io::Result<&str> {
        let offset = offset as usize;
        if offset < strings_start || offset >= strings_end {
            return Err(invalid());
        }
        let value = &bytes[offset..strings_end];
        let length = value
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(invalid)?;
        std::str::from_utf8(&value[..length]).map_err(|_| invalid())
    };
    let extension = u32_at(bytes, 32)? as usize;
    let mut hwcaps = Vec::new();
    if extension != 0 {
        if extension < strings_end || extension % 4 != 0 || u32_at(bytes, extension)? != 0xeaa4_2174
        {
            return Err(invalid());
        }
        let count = u32_at(bytes, extension + 4)? as usize;
        if count > 2
            || extension
                .checked_add(8 + count * 16)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(invalid());
        }
        let mut tags = BTreeSet::new();
        let mut ranges = Vec::new();
        for index in 0..count {
            let at = extension + 8 + index * 16;
            let tag = u32_at(bytes, at)?;
            let offset = u32_at(bytes, at + 8)? as usize;
            let size = u32_at(bytes, at + 12)? as usize;
            if !tags.insert(tag)
                || u32_at(bytes, at + 4)? != 0
                || offset < extension + 8 + count * 16
                || offset.checked_add(size).is_none_or(|end| end > bytes.len())
            {
                return Err(invalid());
            }
            let end = offset + size;
            if ranges
                .iter()
                .any(|(start, stop)| offset < *stop && *start < end)
            {
                return Err(invalid());
            }
            ranges.push((offset, end));
            match tag {
                0 => {
                    if size > 4096 {
                        return Err(invalid());
                    }
                }
                1 => {
                    if offset % 4 != 0 || size == 0 || size % 4 != 0 || size > 3 * 4 {
                        return Err(invalid());
                    }
                    for entry in (offset..offset + size).step_by(4) {
                        let name = string(u32_at(bytes, entry)?)?;
                        if !matches!(name, "x86-64-v2" | "x86-64-v3" | "x86-64-v4")
                            || hwcaps.last().is_some_and(|previous| *previous >= name)
                        {
                            return Err(invalid());
                        }
                        hwcaps.push(name);
                    }
                }
                _ => return Err(invalid()),
            }
        }
    }
    let mut result = BTreeMap::<String, Vec<Candidate>>::new();
    let mut previous = None;
    for entry in bytes
        .get(48..strings_start)
        .ok_or_else(invalid)?
        .chunks_exact(24)
    {
        let name = string(u32_at(entry, 4)?)?;
        // glibc 的缓存按带数字比较的库名降序排列，不能只验证线性扫描能找到键。
        if let Some(previous) = previous {
            let order = cache_name_cmp(previous, name)?;
            if order == std::cmp::Ordering::Less
                || (order == std::cmp::Ordering::Equal && previous != name)
            {
                return Err(invalid());
            }
        }
        previous = Some(name);
        let path = string(u32_at(entry, 8)?)?;
        if u32_at(entry, 0)? != 0x303 {
            continue;
        }
        let hwcap = u64_at(entry, 16)?;
        if hwcap != 0
            && (hwcap & !(HWCAP_EXTENSION | (0x3ff << 32) | 0xffff_ffff) != 0
                || hwcap & HWCAP_EXTENSION == 0
                || hwcap as u32 as usize >= hwcaps.len())
        {
            return Err(invalid());
        }
        if name.is_empty() || name.len() > 255 {
            return Err(invalid());
        }
        result.entry(name.to_owned()).or_default().push(Candidate {
            path: path.into(),
            baseline: hwcap == 0,
            minimum_kernel: u32_at(entry, 12)?,
        });
    }
    Ok(result)
}

fn cache_name_cmp(left: &str, right: &str) -> io::Result<std::cmp::Ordering> {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    let (mut a, mut b) = (0, 0);
    while a < left.len() && b < right.len() {
        if left[a].is_ascii_digit() && right[b].is_ascii_digit() {
            let mut values = [0_u32; 2];
            for (index, (bytes, offset)) in
                [(left, &mut a), (right, &mut b)].into_iter().enumerate()
            {
                while *offset < bytes.len() && bytes[*offset].is_ascii_digit() {
                    values[index] = values[index]
                        .checked_mul(10)
                        .and_then(|value| value.checked_add(u32::from(bytes[*offset] - b'0')))
                        .filter(|value| *value <= i32::MAX as u32)
                        .ok_or_else(invalid)?;
                    *offset += 1;
                }
            }
            if values[0] != values[1] {
                return Ok(values[0].cmp(&values[1]));
            }
        } else if left[a].is_ascii_digit() != right[b].is_ascii_digit() {
            return Ok(left[a].is_ascii_digit().cmp(&right[b].is_ascii_digit()));
        } else {
            if left[a] != right[b] {
                return Ok(left[a].cmp(&right[b]));
            }
            a += 1;
            b += 1;
        }
    }
    Ok((left.len() - a).cmp(&(right.len() - b)))
}

#[cfg(test)]
#[path = "managed_process_atomic_linux_glibc_tests.rs"]
mod tests;
