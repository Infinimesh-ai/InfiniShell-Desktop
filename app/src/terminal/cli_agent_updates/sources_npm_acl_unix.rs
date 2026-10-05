//! Unix 自有包对象的有界 ACL 原语与新节点权限计划。
//! 仅使用调用方持有的原 fd；调用方仍须绑定祖先、内容、硬链接和其他安全属性。

use std::collections::BTreeSet;
use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

const MAX_ENTRIES: usize = 128;
#[cfg(any(target_os = "macos", test))]
const MAC_HEADER: usize = 44;
#[cfg(any(target_os = "macos", test))]
const MAC_ACE: usize = 24;
#[cfg(any(target_os = "macos", test))]
const MAC_ATTRIBUTE_HEADER: usize = 12;
#[cfg(any(target_os = "macos", test))]
const MAC_MAGIC: u32 = 0x012c_c16d;
const MAC_NO_INHERIT: u32 = 1 << 17;
const MAC_ENTRY_FLAGS: u32 = 0x1f0;
const MAC_RIGHTS: u32 = 0x0010_3ffe;
const MAC_READ_RIGHTS: u32 = (1 << 1) | (1 << 3) | (1 << 7) | (1 << 9) | (1 << 11) | (1 << 20);
const UNDEFINED_ID: u32 = u32::MAX;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(super) struct Entries<T>(Vec<T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Entries<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Bounded<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for Bounded<T> {
            type Value = Entries<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("最多 128 条 ACL 项")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
                let mut entries = Vec::new();
                while let Some(entry) = input.next_element()? {
                    if entries.len() == MAX_ENTRIES {
                        return Err(de::Error::custom("ACL 项超限"));
                    }
                    entries.push(entry);
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_seq(Bounded(std::marker::PhantomData))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MacAce {
    uuid: [u8; 16],
    tag: u32,
    flags: u32,
    rights: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MacAcl {
    flags: u32,
    entries: Entries<MacAce>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PosixEntry {
    tag: u16,
    permissions: u16,
    id: u32,
}

/// 平台和格式版本进入描述；Mac 空 ACL 不等价于原生属性缺失。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", deny_unknown_fields)]
pub(super) enum Acl {
    MacV1 {
        extended: Option<MacAcl>,
    },
    LinuxV1 {
        access: Option<Entries<PosixEntry>>,
        default: Option<Entries<PosixEntry>>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FailureKind {
    Unsupported,
    Invalid,
    TooLarge,
    Changed,
    Native,
    ReadbackMismatch,
}

/// 无路径或 ACL 主体文本；errno 必须保留，不能映射为“没有 ACL”。
#[derive(Debug)]
pub(super) struct Failure {
    pub(super) operation: &'static str,
    pub(super) kind: FailureKind,
    pub(super) errno: Option<i32>,
    pub(super) write_attempted: bool,
}

fn fail(operation: &'static str, kind: FailureKind) -> Failure {
    Failure {
        operation,
        kind,
        errno: None,
        write_attempted: false,
    }
}

fn native(operation: &'static str) -> Failure {
    Failure {
        operation,
        kind: FailureKind::Native,
        errno: io::Error::last_os_error().raw_os_error(),
        write_attempted: false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Identity {
    device: u64,
    inode: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    links: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Identity {
    fn read(file: &File) -> Result<Self, Failure> {
        let value = file.metadata().map_err(|error| Failure {
            operation: "fstat",
            kind: FailureKind::Native,
            errno: error.raw_os_error(),
            write_attempted: false,
        })?;
        if value.uid() != unsafe { libc::geteuid() }
            || value.mode() & 0o7022 != 0
            || !(value.is_file() || value.is_dir())
        {
            return Err(fail("owned_object", FailureKind::Unsupported));
        }
        Ok(Self {
            device: value.dev(),
            inode: value.ino(),
            uid: value.uid(),
            gid: value.gid(),
            mode: value.mode(),
            links: value.nlink(),
            length: value.len(),
            modified: (value.mtime(), value.mtime_nsec()),
            changed: (value.ctime(), value.ctime_nsec()),
        })
    }
    fn directory(&self) -> bool {
        self.mode & libc::S_IFMT as u32 == libc::S_IFDIR as u32
    }
    fn unchanged_except_ctime(&self, other: &Self) -> bool {
        let mut expected = self.clone();
        expected.changed = other.changed;
        expected == *other
    }
}

/// 不序列化原 fd 状态；账本只保存 acl，现有对象身份仍由父树负责。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Captured {
    identity: Identity,
    pub(super) acl: Acl,
}

impl Acl {
    pub(super) fn absent() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacV1 { extended: None }
        }
        #[cfg(target_os = "linux")]
        {
            Self::LinuxV1 {
                access: None,
                default: None,
            }
        }
    }

    pub(super) fn is_absent(&self) -> bool {
        match self {
            Self::MacV1 { extended } => extended.is_none(),
            Self::LinuxV1 { access, default } => access.is_none() && default.is_none(),
        }
    }

    pub(super) fn into_optional(self) -> Option<Self> {
        if self.is_absent() { None } else { Some(self) }
    }

    /// requested_mode 是调用方计划的权限，不能取临时目录的偶然继承结果。
    /// 没有继承 ACL 时保持该模式；调用方负责既有的创建掩码策略。
    pub(super) fn inherit(
        &self,
        directory: bool,
        requested_mode: u32,
    ) -> Result<(Self, u32), Failure> {
        if requested_mode & !0o777 != 0 || requested_mode & 0o022 != 0 {
            return Err(fail("inherit_mode", FailureKind::Unsupported));
        }
        match self {
            Self::MacV1 { extended } => {
                let mut entries = Vec::new();
                if let Some(parent) = extended {
                    validate_mac(parent)?;
                    // XNU kauth_acl_inherit：父 NO_INHERIT 不抹除其可传播 ACE。
                    for entry in &parent.entries.0 {
                        if entry.flags & (if directory { 0x40 } else { 0x20 }) == 0 {
                            continue;
                        }
                        let mut child = entry.clone();
                        child.flags = (child.flags | 0x10) & !0x100;
                        if !directory || child.flags & 0x80 != 0 {
                            child.flags &= !0x1e0;
                        }
                        entries.push(child);
                    }
                }
                let extended = if entries.is_empty() {
                    None
                } else {
                    Some(MacAcl {
                        flags: 0,
                        entries: Entries(entries),
                    })
                };
                Ok((Self::MacV1 { extended }, requested_mode))
            }
            Self::LinuxV1 { access, default } => {
                if let Some(access) = access {
                    validate_posix(access)?;
                }
                let Some(parent) = default else {
                    return Ok((
                        Self::LinuxV1 {
                            access: None,
                            default: None,
                        },
                        requested_mode,
                    ));
                };
                validate_posix(parent)?;
                let mut child = parent.clone();
                let has_mask = child.0.iter().any(|entry| entry.tag == 16);
                for entry in &mut child.0 {
                    // Linux posix_acl_create_masq：保留 named 项和原 mask，不重算 mask。
                    let shift = match entry.tag {
                        1 => Some(6),
                        16 => Some(3),
                        4 if !has_mask => Some(3),
                        32 => Some(0),
                        2 | 4 | 8 => None,
                        _ => return Err(fail("inherit_tag", FailureKind::Invalid)),
                    };
                    if let Some(shift) = shift {
                        entry.permissions &= ((requested_mode >> shift) & 7) as u16;
                    }
                }
                let mode = posix_mode(&child);
                // 内核只把恰为 owner/group/other 的 access ACL 规范为 mode。
                let access = if child.0.len() == 3 {
                    None
                } else {
                    Some(child)
                };
                let result = Self::LinuxV1 {
                    access,
                    default: directory.then(|| parent.clone()),
                };
                result.validate(directory, mode)?;
                Ok((result, mode))
            }
        }
    }

    pub(super) fn validate(&self, directory: bool, mode: u32) -> Result<(), Failure> {
        match self {
            Self::MacV1 { extended } => {
                if let Some(value) = extended {
                    validate_mac(value)?;
                }
                Ok(())
            }
            Self::LinuxV1 { access, default } => {
                if let Some(value) = access {
                    validate_posix(value)?;
                    let acl_mode = posix_mode(value);
                    if acl_mode != mode & 0o777 {
                        return Err(fail("access_mode", FailureKind::Invalid));
                    }
                }
                if let Some(value) = default {
                    if !directory {
                        return Err(fail("default_on_file", FailureKind::Invalid));
                    }
                    validate_posix(value)?;
                }
                Ok(())
            }
        }
    }
}

// 仅在 validate_posix 已确认必需项后调用。
fn posix_mode(value: &Entries<PosixEntry>) -> u32 {
    let rights = |tag| {
        value
            .0
            .iter()
            .find(|entry| entry.tag == tag)
            .map(|entry| u32::from(entry.permissions))
    };
    (rights(1).unwrap() << 6)
        | (rights(16).or_else(|| rights(4)).unwrap() << 3)
        | rights(32).unwrap()
}

fn validate_mac(value: &MacAcl) -> Result<(), Failure> {
    if value.entries.0.len() > MAX_ENTRIES {
        return Err(fail("mac_count", FailureKind::TooLarge));
    }
    // 私有或 deferred 标志没有本增量语义，拒绝而非清零。
    if value.flags & !MAC_NO_INHERIT != 0 {
        return Err(fail("mac_acl_flags", FailureKind::Unsupported));
    }
    for entry in &value.entries.0 {
        if !matches!(entry.tag, 1 | 2)
            || entry.flags & !MAC_ENTRY_FLAGS != 0
            || entry.rights & !MAC_RIGHTS != 0
        {
            return Err(fail("mac_ace", FailureKind::Unsupported));
        }
        // 不解析 UUID 成用户名；即使是当前用户的附加 allow，也不接纳写授权。
        if entry.tag == 1 && entry.rights & !MAC_READ_RIGHTS != 0 {
            return Err(fail("mac_allow_write", FailureKind::Unsupported));
        }
    }
    Ok(())
}

fn validate_posix(value: &Entries<PosixEntry>) -> Result<(), Failure> {
    if value.0.len() > MAX_ENTRIES {
        return Err(fail("posix_count", FailureKind::TooLarge));
    }
    let mut seen = BTreeSet::new();
    let mut previous = None;
    let mut named = false;
    for entry in &value.0 {
        if !matches!(entry.tag, 1 | 2 | 4 | 8 | 16 | 32) || entry.permissions & !7 != 0 {
            return Err(fail("posix_entry", FailureKind::Invalid));
        }
        let is_named = matches!(entry.tag, 2 | 8);
        if is_named == (entry.id == UNDEFINED_ID) {
            return Err(fail("posix_id", FailureKind::Invalid));
        }
        let key = (entry.tag, entry.id);
        if !seen.insert(key) || previous.is_some_and(|old| old >= key) {
            return Err(fail("posix_order_or_duplicate", FailureKind::Invalid));
        }
        previous = Some(key);
        named |= is_named;
        // 保守拒绝被 mask 暂时遮蔽的原始写位，避免后续 chmod/继承重新放行。
        if entry.tag != 1 && entry.permissions & 2 != 0 {
            return Err(fail("posix_non_owner_write", FailureKind::Unsupported));
        }
    }
    if ![1, 4, 32]
        .iter()
        .all(|tag| seen.contains(&(*tag, UNDEFINED_ID)))
        || (named && !seen.contains(&(16, UNDEFINED_ID)))
    {
        return Err(fail("posix_required_entries", FailureKind::Invalid));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn decode_mac(bytes: &[u8]) -> Result<MacAcl, Failure> {
    if bytes.len() < MAC_HEADER {
        return Err(fail("mac_header", FailureKind::Invalid));
    }
    let word = |at| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    let count = word(36) as usize;
    if count > MAX_ENTRIES {
        return Err(fail("mac_count", FailureKind::TooLarge));
    }
    // acl_copy_ext 的公开 portable 表示；owner/group 保留区按 libc 为零。
    if word(0) != MAC_MAGIC
        || bytes[4..36].iter().any(|byte| *byte != 0)
        || bytes.len() != MAC_HEADER + count * MAC_ACE
    {
        return Err(fail("mac_external_format", FailureKind::Invalid));
    }
    let mut entries = Vec::with_capacity(count);
    for raw in bytes[MAC_HEADER..].chunks_exact(MAC_ACE) {
        let flags = u32::from_be_bytes(raw[16..20].try_into().unwrap());
        entries.push(MacAce {
            uuid: raw[..16].try_into().unwrap(),
            tag: flags & 0xf,
            flags: flags & !0xf,
            rights: u32::from_be_bytes(raw[20..24].try_into().unwrap()),
        });
    }
    let result = MacAcl {
        flags: word(40),
        entries: Entries(entries),
    };
    validate_mac(&result)?;
    Ok(result)
}

#[cfg(any(target_os = "macos", test))]
fn decode_mac_attributes(bytes: &[u8]) -> Result<Option<MacAcl>, Failure> {
    if bytes.len() < MAC_ATTRIBUTE_HEADER {
        return Err(fail("mac_attribute_header", FailureKind::Invalid));
    }
    let word = |at| u32::from_ne_bytes(bytes[at..at + 4].try_into().unwrap());
    let total = word(0) as usize;
    if total > MAC_ATTRIBUTE_HEADER + MAC_HEADER + MAX_ENTRIES * MAC_ACE {
        return Err(fail("mac_attribute_size", FailureKind::TooLarge));
    }
    if total < MAC_ATTRIBUTE_HEADER || total > bytes.len() {
        return Err(fail("mac_attribute_size", FailureKind::Invalid));
    }
    // 只请求一个可变属性：长度后紧接 attrreference，偏移从该引用起算，不能指向头部或填充区。
    let length = word(8) as usize;
    if word(4) != 8 || length != total - MAC_ATTRIBUTE_HEADER {
        return Err(fail("mac_attribute_reference", FailureKind::Invalid));
    }
    if length == 0 {
        return Ok(None);
    }
    let native = &bytes[MAC_ATTRIBUTE_HEADER..total];
    if native.len() < MAC_HEADER {
        return Err(fail("mac_attribute_acl", FailureKind::Invalid));
    }
    let count = u32::from_ne_bytes(native[36..40].try_into().unwrap()) as usize;
    if count > MAX_ENTRIES {
        return Err(fail("mac_count", FailureKind::TooLarge));
    }
    if native.len() != MAC_HEADER + count * MAC_ACE {
        return Err(fail("mac_attribute_acl", FailureKind::Invalid));
    }
    // XNU 的 ATTR_CMN_EXTENDED_SECURITY 返回本机字节序；仅转换整数，UUID 和条目顺序保持原样。
    let mut portable = native.to_vec();
    for at in [0, 36, 40] {
        let value = u32::from_ne_bytes(portable[at..at + 4].try_into().unwrap());
        portable[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    for entry in portable[MAC_HEADER..].chunks_exact_mut(MAC_ACE) {
        for at in [16, 20] {
            let value = u32::from_ne_bytes(entry[at..at + 4].try_into().unwrap());
            entry[at..at + 4].copy_from_slice(&value.to_be_bytes());
        }
    }
    // 复用既有完整校验；44 字节、零条目仍是 Some，不能折叠成属性缺失。
    decode_mac(&portable).map(Some)
}

#[cfg(any(target_os = "macos", test))]
fn encode_mac(value: &MacAcl) -> Result<Vec<u8>, Failure> {
    validate_mac(value)?;
    let mut bytes = vec![0; MAC_HEADER];
    bytes[..4].copy_from_slice(&MAC_MAGIC.to_be_bytes());
    bytes[36..40].copy_from_slice(&(value.entries.0.len() as u32).to_be_bytes());
    bytes[40..44].copy_from_slice(&value.flags.to_be_bytes());
    for entry in &value.entries.0 {
        bytes.extend_from_slice(&entry.uuid);
        bytes.extend_from_slice(&(entry.tag | entry.flags).to_be_bytes());
        bytes.extend_from_slice(&entry.rights.to_be_bytes());
    }
    Ok(bytes)
}

#[cfg(any(target_os = "linux", test))]
fn decode_posix(bytes: &[u8]) -> Result<Entries<PosixEntry>, Failure> {
    if bytes.len() > 4 + MAX_ENTRIES * 8 {
        return Err(fail("posix_bytes", FailureKind::TooLarge));
    }
    if bytes.len() < 4 || (bytes.len() - 4) % 8 != 0 || bytes[..4] != 2_u32.to_le_bytes() {
        return Err(fail("posix_xattr_format", FailureKind::Invalid));
    }
    let value = Entries(
        bytes[4..]
            .chunks_exact(8)
            .map(|raw| PosixEntry {
                tag: u16::from_le_bytes(raw[..2].try_into().unwrap()),
                permissions: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
                id: u32::from_le_bytes(raw[4..8].try_into().unwrap()),
            })
            .collect(),
    );
    validate_posix(&value)?;
    Ok(value)
}

#[cfg(any(target_os = "linux", test))]
fn encode_posix(value: &Entries<PosixEntry>) -> Result<Vec<u8>, Failure> {
    validate_posix(value)?;
    let mut bytes = 2_u32.to_le_bytes().to_vec();
    for entry in &value.0 {
        bytes.extend_from_slice(&entry.tag.to_le_bytes());
        bytes.extend_from_slice(&entry.permissions.to_le_bytes());
        bytes.extend_from_slice(&entry.id.to_le_bytes());
    }
    Ok(bytes)
}

pub(super) fn capture(file: &File) -> Result<Captured, Failure> {
    let before = Identity::read(file)?;
    let acl = platform::read(file)?;
    acl.validate(before.directory(), before.mode)?;
    if Identity::read(file)? != before {
        return Err(fail("capture_identity", FailureKind::Changed));
    }
    // 第二次同 fd 读取避免把访问 ACL 与默认 ACL 的两次独立读取冒充原子快照。
    if platform::read(file)? != acl || Identity::read(file)? != before {
        return Err(fail("capture_acl", FailureKind::Changed));
    }
    Ok(Captured {
        identity: before,
        acl,
    })
}

/// 未跟踪 ACL 的消费者只判缺失；原 fd 的对象类型、所有权和前后身份仍由调用方校验。
#[cfg(target_os = "macos")]
pub(super) fn is_absent_on_fd(file: &File) -> Result<bool, Failure> {
    platform::read(file).map(|value| value.is_absent())
}

pub(super) fn verify(file: &File, expected: &Captured) -> Result<(), Failure> {
    if &capture(file)? != expected {
        return Err(fail("verify_acl", FailureKind::Changed));
    }
    Ok(())
}

/// 仅供父事务已确认的本轮新对象。失败可能已部分写入，绝不静默回滚或当作成功。
/// 硬链接多名权限计划尚未接线；这里只允许单链接文件或目录。
pub(super) fn apply_to_new(
    file: &File,
    expected: &Captured,
    desired: &Acl,
) -> Result<Captured, Failure> {
    verify(file, expected)?;
    desired.validate(expected.identity.directory(), expected.identity.mode)?;
    if !expected.identity.directory() && expected.identity.links != 1 {
        return Err(fail("apply_hardlink", FailureKind::Unsupported));
    }
    if &expected.acl == desired {
        return Ok(expected.clone());
    }
    let result: Result<Captured, Failure> = (|| {
        platform::write(file, desired)?;
        file.sync_all().map_err(|error| Failure {
            operation: "acl_fsync",
            kind: FailureKind::Native,
            errno: error.raw_os_error(),
            write_attempted: true,
        })?;
        let actual = capture(file)?;
        if !expected.identity.unchanged_except_ctime(&actual.identity) || &actual.acl != desired {
            return Err(fail("acl_readback", FailureKind::ReadbackMismatch));
        }
        Ok(actual)
    })();
    result.map_err(|mut error| {
        error.write_attempted = true;
        error
    })
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::ffi::CStr;

    fn get(file: &File, name: &CStr) -> Result<Option<Entries<PosixEntry>>, Failure> {
        // 一次有界读取；ERANGE、权限错误和不支持均不是缺失。
        let mut bytes = [0_u8; 4 + MAX_ENTRIES * 8];
        let size = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if size < 0 {
            let error = native("fgetxattr_acl");
            return if error.errno == Some(libc::ENODATA) {
                Ok(None)
            } else {
                Err(error)
            };
        }
        if size as usize > bytes.len() {
            return Err(fail("fgetxattr_size", FailureKind::TooLarge));
        }
        decode_posix(&bytes[..size as usize]).map(Some)
    }
    pub(super) fn read(file: &File) -> Result<Acl, Failure> {
        Ok(Acl::LinuxV1 {
            access: get(file, c"system.posix_acl_access")?,
            default: get(file, c"system.posix_acl_default")?,
        })
    }
    fn set(file: &File, name: &CStr, value: &Option<Entries<PosixEntry>>) -> Result<(), Failure> {
        let result = match value {
            Some(value) => {
                let bytes = encode_posix(value)?;
                unsafe {
                    libc::fsetxattr(
                        file.as_raw_fd(),
                        name.as_ptr(),
                        bytes.as_ptr().cast(),
                        bytes.len(),
                        0,
                    )
                }
            }
            None => {
                let result = unsafe { libc::fremovexattr(file.as_raw_fd(), name.as_ptr()) };
                if result < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ENODATA) {
                    return Ok(());
                }
                result
            }
        };
        if result < 0 {
            return Err(native("set_or_remove_acl_xattr"));
        }
        Ok(())
    }
    pub(super) fn write(file: &File, value: &Acl) -> Result<(), Failure> {
        match value {
            Acl::LinuxV1 { access, default } => {
                // 只有这两个固定属性可写；不接受调用方自带名称。
                set(file, c"system.posix_acl_access", access)?;
                set(file, c"system.posix_acl_default", default)
            }
            Acl::MacV1 { .. } => Err(fail("acl_platform", FailureKind::Unsupported)),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::c_void;

    unsafe extern "C" {
        fn acl_set_fd_np(fd: libc::c_int, acl: *mut c_void, kind: libc::c_int) -> libc::c_int;
        fn acl_free(acl: *mut c_void) -> libc::c_int;
        fn acl_copy_int(bytes: *const c_void) -> *mut c_void;
    }
    struct OwnedAcl(*mut c_void);
    impl Drop for OwnedAcl {
        fn drop(&mut self) {
            unsafe { acl_free(self.0) };
        }
    }

    pub(super) fn read(file: &File) -> Result<Acl, Failure> {
        let mut attributes = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: libc::ATTR_CMN_EXTENDED_SECURITY,
            volattr: 0,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };
        // acl_get_fd_np 的 libc 路径会漏掉 44 字节空 ACL；直接从原 fd 读取公开属性，不按路径重开。
        let mut aligned = [0_u32; (MAC_ATTRIBUTE_HEADER + MAC_HEADER + MAX_ENTRIES * MAC_ACE) / 4];
        if unsafe {
            libc::fgetattrlist(
                file.as_raw_fd(),
                (&mut attributes as *mut libc::attrlist).cast(),
                aligned.as_mut_ptr().cast(),
                std::mem::size_of_val(&aligned),
                libc::FSOPT_REPORT_FULLSIZE,
            )
        } != 0
        {
            return Err(native("fgetattrlist_acl"));
        }
        let bytes = unsafe {
            std::slice::from_raw_parts(aligned.as_ptr().cast(), std::mem::size_of_val(&aligned))
        };
        Ok(Acl::MacV1 {
            extended: decode_mac_attributes(bytes)?,
        })
    }
    pub(super) fn write(file: &File, value: &Acl) -> Result<(), Failure> {
        match value {
            Acl::MacV1 { extended: None } => {
                // SDK sys/fcntl.h 的 _FILESEC_REMOVE_ACL；不是分配对象，不能 acl_free。
                if unsafe { acl_set_fd_np(file.as_raw_fd(), 1_usize as *mut c_void, 0x100) } != 0 {
                    return Err(native("acl_remove_fd_np"));
                }
                Ok(())
            }
            Acl::MacV1 {
                extended: Some(value),
            } => {
                let bytes = encode_mac(value)?;
                let mut aligned = [0_u32; (MAC_HEADER + MAX_ENTRIES * MAC_ACE) / 4];
                let target = unsafe {
                    std::slice::from_raw_parts_mut(aligned.as_mut_ptr().cast(), bytes.len())
                };
                target.copy_from_slice(&bytes);
                // acl_copy_int 没有长度参数，只有已验证 count/总长度的编码能进入这里。
                let raw = unsafe { acl_copy_int(aligned.as_ptr().cast()) };
                if raw.is_null() {
                    return Err(native("acl_copy_int"));
                }
                let owned = OwnedAcl(raw);
                // Apple 的 acl_valid_fd_np 为 ENOTSUP 占位；完整纯校验已在 encode_mac 中执行。
                if unsafe { acl_set_fd_np(file.as_raw_fd(), owned.0, 0x100) } != 0 {
                    return Err(native("acl_set_fd_np"));
                }
                Ok(())
            }
            Acl::LinuxV1 { .. } => Err(fail("acl_platform", FailureKind::Unsupported)),
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
fn uid_uuid_for_test(uid: u32) -> [u8; 16] {
    unsafe extern "C" {
        fn mbr_uid_to_uuid(uid: libc::uid_t, uuid: *mut u8) -> libc::c_int;
    }
    let mut uuid = [0; 16];
    assert_eq!(unsafe { mbr_uid_to_uuid(uid, uuid.as_mut_ptr()) }, 0);
    uuid
}

/// 只读 ACL 的真实平台夹具；Linux 非继承型 access 与 0644 对应。
#[cfg(test)]
pub(super) fn readonly_test_acl(inherit: bool) -> Acl {
    #[cfg(target_os = "macos")]
    {
        Acl::MacV1 {
            extended: Some(MacAcl {
                flags: 0,
                entries: Entries(vec![
                    MacAce {
                        uuid: uid_uuid_for_test(unsafe { libc::geteuid() }),
                        tag: 1,
                        flags: if inherit { 0x60 } else { 0 },
                        rights: MAC_READ_RIGHTS,
                    },
                    MacAce {
                        uuid: uid_uuid_for_test(0),
                        tag: 1,
                        flags: if inherit { 0x60 } else { 0 },
                        rights: MAC_READ_RIGHTS,
                    },
                ]),
            }),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let entries = Entries(vec![
            PosixEntry {
                tag: 1,
                permissions: if inherit { 7 } else { 6 },
                id: UNDEFINED_ID,
            },
            PosixEntry {
                tag: 2,
                permissions: 4,
                id: if unsafe { libc::geteuid() } == 0 {
                    1
                } else {
                    0
                },
            },
            PosixEntry {
                tag: 4,
                permissions: if inherit { 5 } else { 4 },
                id: UNDEFINED_ID,
            },
            PosixEntry {
                tag: 16,
                permissions: if inherit { 5 } else { 4 },
                id: UNDEFINED_ID,
            },
            PosixEntry {
                tag: 32,
                permissions: if inherit { 5 } else { 4 },
                id: UNDEFINED_ID,
            },
        ]);
        if inherit {
            Acl::LinuxV1 {
                access: None,
                default: Some(entries),
            }
        } else {
            Acl::LinuxV1 {
                access: Some(entries),
                default: None,
            }
        }
    }
}

/// 真实安装夹具沿用文件已有 mode；Linux access ACL 不能把 0750 原生入口改成 0644。
#[cfg(test)]
pub(super) fn readonly_file_test_acl(mode: u32) -> Result<Acl, Failure> {
    if mode & !0o777 != 0 || mode & 0o022 != 0 {
        return Err(fail("fixture_mode", FailureKind::Unsupported));
    }
    #[allow(unused_mut)]
    let mut value = readonly_test_acl(false);
    #[cfg(target_os = "linux")]
    if let Acl::LinuxV1 {
        access: Some(entries),
        ..
    } = &mut value
    {
        for entry in &mut entries.0 {
            match entry.tag {
                1 => entry.permissions = ((mode >> 6) & 7) as u16,
                4 | 16 => entry.permissions = ((mode >> 3) & 7) as u16,
                32 => entry.permissions = (mode & 7) as u16,
                2 | 8 => {}
                _ => return Err(fail("fixture_tag", FailureKind::Invalid)),
            }
        }
    }
    value.validate(false, mode)?;
    Ok(value)
}

/// 仅新版 Mac 真实事务夹具使用；显式空 ACL 仍要求原有普通文件 mode 合同。
#[cfg(test)]
pub(super) fn empty_file_test_acl(mode: u32) -> Result<Acl, Failure> {
    if mode & !0o777 != 0 || mode & 0o022 != 0 {
        return Err(fail("fixture_mode", FailureKind::Unsupported));
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Acl::MacV1 {
            extended: Some(MacAcl {
                flags: 0,
                entries: Entries(vec![]),
            }),
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(fail("fixture_empty_acl_platform", FailureKind::Unsupported))
    }
}

#[cfg(test)]
#[path = "sources_npm_acl_unix_tests.rs"]
mod tests;
