//! 将候选树映射为新登录会话的局部盘根，不向 AppContainer 开放原卷根。

use super::{
    FileIdentity, PathLease, Snapshot, file_identity, invalid, no_impersonation, open_path_locked,
    require, wide,
};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::Storage::FileSystem::{
    DDD_EXACT_MATCH_ON_REMOVE, DDD_NO_BROADCAST_SYSTEM, DDD_RAW_TARGET_PATH, DDD_REMOVE_DEFINITION,
    DefineDosDeviceW, QueryDosDeviceW,
};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::core::PCWSTR;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Target {
    pub(super) path: PathBuf,
    pub(super) identity: FileIdentity,
    nt_path: String,
}

impl Target {
    pub(super) fn capture(lease: &PathLease) -> io::Result<Self> {
        lease.verify()?;
        let path = lease.path();
        require(path.is_dir(), "局部盘映射目标不是目录")?;
        let value = path
            .to_str()
            .ok_or_else(|| invalid("局部盘映射目标编码无效"))?;
        let (drive, suffix) = physical_parts(value)?;
        let targets = query(Some(drive))?;
        require(targets.len() == 1, "局部盘映射原卷目标不唯一")?;
        let volume = &targets[0];
        let number = volume.strip_prefix(r"\Device\HarddiskVolume");
        require(
            number.is_some_and(|value| {
                !value.is_empty() && value.bytes().all(|value| value.is_ascii_digit())
            }),
            "局部盘映射只接受本地物理卷",
        )?;
        Ok(Self {
            path: path.to_owned(),
            identity: lease.entries[0].2.clone(),
            nt_path: format!("{volume}\\{suffix}"),
        })
    }
}

fn physical_parts(path: &str) -> io::Result<(&str, &str)> {
    let path = path
        .strip_prefix(r"\\?\")
        .ok_or_else(|| invalid("局部盘映射需要规范物理路径"))?;
    let bytes = path.as_bytes();
    require(
        bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\',
        "局部盘映射不能指向卷根或非本地路径",
    )?;
    let suffix = &path[3..];
    require(
        suffix
            .split('\\')
            .all(|part| !part.is_empty() && part != "." && part != "..")
            && !suffix.contains(['\0', '/', ':']),
        "局部盘映射目标含不安全路径段",
    )?;
    Ok((&path[..2], suffix))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub(super) target: Target,
    pub(super) mapped_root: PathBuf,
    auth_low: u32,
    auth_high: i32,
    user: String,
    session: u32,
}

impl Binding {
    pub(super) fn validate(&self, target: &Target, owner: &Snapshot) -> io::Result<()> {
        require(
            self.target == *target
                && (self.auth_low, self.auth_high) == (owner.auth_low, owner.auth_high)
                && self.user == owner.user
                && self.user != "S-1-5-18"
                && self.session == owner.session,
            "局部盘映射目标或登录绑定不匹配",
        )?;
        alias(&self.mapped_root)?;
        Ok(())
    }

    pub(super) fn verify_hidden_from_caller(&self) -> io::Result<()> {
        require(
            query(Some(&alias(&self.mapped_root)?))?.is_empty(),
            "局部盘映射意外出现在调用方命名空间",
        )
    }
}

fn alias(root: &Path) -> io::Result<String> {
    let value = root
        .to_str()
        .ok_or_else(|| invalid("局部盘映射别名编码无效"))?;
    let bytes = value.as_bytes();
    require(
        bytes.len() == 3 && (b'D'..=b'Z').contains(&bytes[0]) && bytes[1..] == *b":\\",
        "局部盘映射别名不是独立 D 至 Z 盘根",
    )?;
    Ok(value[..2].to_owned())
}

fn query(name: Option<&str>) -> io::Result<Vec<String>> {
    let name = name.map(|value| wide(value.as_ref())).transpose()?;
    let mut length = 4096;
    loop {
        let mut buffer = vec![0u16; length];
        let used = unsafe {
            QueryDosDeviceW(
                name.as_ref()
                    .map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr())),
                Some(&mut buffer),
            )
        };
        if used != 0 {
            require(used as usize <= buffer.len(), "局部盘映射查询长度无效")?;
            return buffer[..used as usize]
                .split(|value| *value == 0)
                .filter(|value| !value.is_empty())
                .map(|value| String::from_utf16(value).map_err(io::Error::other))
                .collect();
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(2) if name.is_some() => return Ok(Vec::new()),
            Some(122) if length < 1024 * 1024 => length *= 2,
            Some(_) | None => return Err(error),
        }
    }
}

pub(super) struct LocalDeviceMap {
    lease: PathLease,
    owner: Snapshot,
    binding: Binding,
    active: bool,
}

impl LocalDeviceMap {
    pub(super) fn create(target: &Target, owner: &Snapshot) -> io::Result<Self> {
        no_impersonation()?;
        require(owner.user != "S-1-5-18", "局部盘映射拒绝 LocalSystem")?;
        require(
            Snapshot::capture(unsafe { GetCurrentProcess() })? == *owner,
            "局部盘映射创建身份变化",
        )?;
        let lease = PathLease::capture(&target.path)?;
        require(
            Target::capture(&lease)? == *target,
            "局部盘映射物理树身份变化",
        )?;
        // QueryDosDevice 无名称查询同时包含调用方可见的局部和全局名称。
        let names = query(None)?;
        let device = (b'D'..=b'Z')
            .map(|letter| format!("{}:", char::from(letter)))
            .find(|device| !names.iter().any(|name| name.eq_ignore_ascii_case(device)))
            .ok_or_else(|| invalid("局部盘映射没有空闲盘符"))?;
        require(query(Some(&device))?.is_empty(), "局部盘映射盘符已被占用")?;
        let binding = Binding {
            target: target.clone(),
            mapped_root: PathBuf::from(format!("{device}\\")),
            auth_low: owner.auth_low,
            auth_high: owner.auth_high,
            user: owner.user.clone(),
            session: owner.session,
        };
        let mut result = Self {
            lease,
            owner: owner.clone(),
            binding,
            active: false,
        };
        let name = wide(device.as_ref())?;
        let target = wide(target.nt_path.as_ref())?;
        unsafe {
            DefineDosDeviceW(
                DDD_RAW_TARGET_PATH | DDD_NO_BROADCAST_SYSTEM,
                PCWSTR(name.as_ptr()),
                PCWSTR(target.as_ptr()),
            )
        }
        .map_err(io::Error::other)?;
        result.active = true;
        result.verify()?;
        Ok(result)
    }

    pub(super) fn binding(&self) -> &Binding {
        &self.binding
    }

    fn verify_owner(&self) -> io::Result<()> {
        no_impersonation()?;
        require(
            Snapshot::capture(unsafe { GetCurrentProcess() })? == self.owner,
            "局部盘映射持有身份变化",
        )
    }

    pub(super) fn verify(&self) -> io::Result<()> {
        self.verify_owner()?;
        require(self.active, "局部盘映射已释放")?;
        self.lease.verify()?;
        self.binding
            .validate(&Target::capture(&self.lease)?, &self.owner)?;
        require(
            query(Some(&alias(&self.binding.mapped_root)?))?
                == [self.binding.target.nt_path.clone()]
                && file_identity(&open_path_locked(&self.binding.mapped_root)?)?
                    == self.binding.target.identity,
            "局部盘映射回读或盘根 FileID 不匹配",
        )
    }

    pub(super) fn close(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.verify_owner()?;
        let verified = self.verify();
        let device = alias(&self.binding.mapped_root)?;
        let name = wide(device.as_ref())?;
        let target = wide(self.binding.target.nt_path.as_ref())?;
        // 即便回读发现外来定义，也只删除本轮的精确目标，绝不弹出别人的定义。
        unsafe {
            DefineDosDeviceW(
                DDD_REMOVE_DEFINITION
                    | DDD_EXACT_MATCH_ON_REMOVE
                    | DDD_RAW_TARGET_PATH
                    | DDD_NO_BROADCAST_SYSTEM,
                PCWSTR(name.as_ptr()),
                PCWSTR(target.as_ptr()),
            )
        }
        .map_err(io::Error::other)?;
        self.active = false;
        let absent = query(Some(&device))?.is_empty();
        verified?;
        require(absent, "局部盘映射删除后仍有可见定义")
    }
}

impl Drop for LocalDeviceMap {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "windows_station_device_map_tests.rs"]
mod tests;
