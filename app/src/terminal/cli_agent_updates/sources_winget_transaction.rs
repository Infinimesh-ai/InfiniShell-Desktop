//! WinGet 单文件 portable 的文件/ARP 双状态恢复层。
//! 发布必须持有独立 AppContainer 原生探针及清理收据，文件与登记分别核对和恢复。

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::path::{Path, PathBuf};

use futures::{AsyncReadExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use uuid::Uuid;
use warpui::r#async::FutureExt as _;
use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, EqualSid, GetLengthSid, IsValidAcl, IsValidSid,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfo,
    FileRenameInfo, GetFileInformationByHandle, READ_CONTROL, SetFileInformationByHandle,
    WRITE_DAC,
};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

use super::winget::{Owner, UNINSTALL};
use super::{
    CLIAgent, Error, MAX_CONFIG, MAX_OUTPUT, UPDATE_TIMEOUT, UpdatePlan, VerificationProgress,
    managed_process, read_limited,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Image {
    volume: u32,
    index: u64,
    length: u64,
    sha256: [u8; 32],
    security: [u8; 32],
}

struct Security {
    allocation: PSECURITY_DESCRIPTOR,
    owner: PSID,
    acl: *mut ACL,
}

impl Security {
    fn read(file: &File) -> Result<Self, Error> {
        let mut value = Self {
            allocation: PSECURITY_DESCRIPTOR::default(),
            owner: PSID::default(),
            acl: std::ptr::null_mut(),
        };
        let status = unsafe {
            GetSecurityInfo(
                HANDLE(file.as_raw_handle()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                Some(&mut value.owner),
                None,
                Some(&mut value.acl),
                None,
                Some(&mut value.allocation),
            )
        };
        if status.0 != 0
            || value.owner.0.is_null()
            || value.acl.is_null()
            || !unsafe { IsValidSid(value.owner) }.as_bool()
            || !unsafe { IsValidAcl(value.acl) }.as_bool()
        {
            return Err(Error::PermissionDenied);
        }
        Ok(value)
    }

    fn digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(unsafe {
            std::slice::from_raw_parts(self.owner.0.cast::<u8>(), GetLengthSid(self.owner) as usize)
        });
        digest.update(unsafe {
            std::slice::from_raw_parts(self.acl.cast::<u8>(), usize::from((*self.acl).AclSize))
        });
        digest.finalize().into()
    }
}

impl Drop for Security {
    fn drop(&mut self) {
        if !self.allocation.0.is_null() {
            unsafe { LocalFree(Some(HLOCAL(self.allocation.0))) };
        }
    }
}

fn copy_security(original: &Path, candidate: &Path) -> Result<(), Error> {
    let source = File::open(original).map_err(|_| Error::SourceChanged)?;
    let candidate = OpenOptions::new()
        .access_mode((READ_CONTROL | WRITE_DAC).0)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(candidate)
        .map_err(|_| Error::PermissionDenied)?;
    let source = Security::read(&source)?;
    let before = Security::read(&candidate)?;
    unsafe { EqualSid(source.owner, before.owner) }.map_err(|_| Error::PermissionDenied)?;
    let status = unsafe {
        SetSecurityInfo(
            HANDLE(candidate.as_raw_handle()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(source.acl),
            None,
        )
    };
    if status.0 != 0 || Security::read(&candidate)?.digest() != source.digest() {
        return Err(Error::PermissionDenied);
    }
    Ok(())
}

fn image(path: &Path) -> Result<Image, Error> {
    super::plain_ancestors(path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_DELETE).0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .map_err(|_| Error::SourceChanged)?;
    let mut before = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut before) }
        .map_err(|_| Error::SourceChanged)?;
    if before.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT).0 != 0
        || before.nNumberOfLinks != 1
    {
        return Err(Error::UnsupportedSource);
    }
    let length = (u64::from(before.nFileSizeHigh) << 32) | u64::from(before.nFileSizeLow);
    if length == 0 || length > 512 * 1024 * 1024 {
        return Err(Error::SourceChanged);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut read = 0_u64;
    loop {
        let size = file.read(&mut buffer).map_err(|_| Error::SourceChanged)?;
        if size == 0 {
            break;
        }
        read += size as u64;
        if read > length {
            return Err(Error::SourceChanged);
        }
        digest.update(&buffer[..size]);
    }
    let mut after = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut after) }
        .map_err(|_| Error::SourceChanged)?;
    if read != length
        || before.dwVolumeSerialNumber != after.dwVolumeSerialNumber
        || before.nFileIndexHigh != after.nFileIndexHigh
        || before.nFileIndexLow != after.nFileIndexLow
        || before.ftLastWriteTime.dwHighDateTime != after.ftLastWriteTime.dwHighDateTime
        || before.ftLastWriteTime.dwLowDateTime != after.ftLastWriteTime.dwLowDateTime
    {
        return Err(Error::SourceChanged);
    }
    let security = Security::read(&file)?.digest();
    Ok(Image {
        volume: before.dwVolumeSerialNumber,
        index: (u64::from(before.nFileIndexHigh) << 32) | u64::from(before.nFileIndexLow),
        length,
        sha256: digest.finalize().into(),
        security,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
enum Phase {
    Prepared,
    ReplaceIntent,
    Registering,
    Committed,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateReceipt {
    pub(super) generation: Uuid,
    pub(super) binding_digest: String,
    pub(super) program: PathBuf,
    pub(super) version: String,
    pub(super) sha256: [u8; 32],
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    id: Uuid,
    owner: Owner,
    target_version: String,
    target_sha256: String,
    candidate: PathBuf,
    backup: PathBuf,
    original: Image,
    prepared: Image,
    probe: CandidateReceipt,
    phase: Phase,
    intent: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreparedCandidate {
    pub(super) id: Uuid,
    pub(super) owner: Owner,
    pub(super) program: PathBuf,
    pub(super) version: String,
    pub(super) sha256: [u8; 32],
    manifest_sha256: [u8; 32],
    original: Image,
    image: Option<Image>,
    probe: Option<CandidateReceipt>,
    intent: String,
}

fn candidate_path(root: &Path) -> PathBuf {
    root.join("claude-winget-candidate.json")
}

fn save_candidate(root: &Path, prepared: &PreparedCandidate) -> Result<(), Error> {
    let mut file = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    file.write_all(&serde_json::to_vec(prepared).map_err(|_| Error::PersistenceFailed)?)
        .map_err(|_| Error::PersistenceFailed)?;
    file.as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    file.persist(candidate_path(root))
        .map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)
}

async fn download(url: &str, limit: u64) -> Result<NamedTempFile, Error> {
    let mut output = NamedTempFile::new().map_err(|_| Error::PersistenceFailed)?;
    let response = http_client::Client::new()
        .get(url)
        .timeout(super::UPDATE_TIMEOUT)
        .send()
        .await
        .map_err(|_| Error::Network)?;
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(Error::Network);
    }
    let stream = response.bytes_stream();
    futures::pin_mut!(stream);
    let mut length = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::Network)?;
        length = length
            .checked_add(chunk.len() as u64)
            .ok_or(Error::InvalidRelease)?;
        if length > limit {
            return Err(Error::InvalidRelease);
        }
        output
            .write_all(&chunk)
            .map_err(|_| Error::PersistenceFailed)?;
    }
    if length == 0 {
        return Err(Error::InvalidRelease);
    }
    output
        .as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    Ok(output)
}

/// 仅准备官方 portable 清单指定的固定 native；不运行 winget、不修改 ARP 或当前入口。
pub(super) async fn prepare_candidate(
    owner: Owner,
    root: &Path,
    intent: &str,
) -> Result<PreparedCandidate, Error> {
    if candidate_path(root)
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    owner.verify_layout()?;
    if Owner::read()?.as_ref() != Some(&owner) {
        return Err(Error::SourceChanged);
    }
    let old = image(&owner.target)?;
    if old.sha256 != super::brew::decode_sha256(&owner.sha256)? {
        return Err(Error::SourceChanged);
    }
    let version = "2.1.280";
    let url = format!(
        "https://raw.githubusercontent.com/microsoft/winget-pkgs/master/manifests/a/Anthropic/ClaudeCode/{version}/Anthropic.ClaudeCode.installer.yaml"
    );
    let metadata = download(&url, MAX_CONFIG).await?;
    let bytes = read_limited(metadata.path(), MAX_CONFIG)?;
    let manifest: serde_json::Value =
        serde_yaml::from_slice(&bytes).map_err(|_| Error::InvalidRelease)?;
    if manifest["PackageIdentifier"] != "Anthropic.ClaudeCode"
        || manifest["PackageVersion"] != version
        || manifest["InstallerType"] != "portable"
        || manifest["ManifestType"] != "installer"
        || manifest["ManifestVersion"] != "1.12.0"
        || manifest["Commands"] != serde_json::json!(["claude"])
        || manifest.get("Dependencies").is_some()
        || manifest.get("InstallerSwitches").is_some()
        || manifest.get("NestedInstallerType").is_some()
    {
        return Err(Error::InvalidRelease);
    }
    let architecture = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return Err(Error::UnsupportedPlatform),
    };
    let installers = manifest["Installers"]
        .as_array()
        .ok_or(Error::InvalidRelease)?;
    let matches: Vec<_> = installers
        .iter()
        .filter(|item| item["Architecture"] == architecture)
        .collect();
    let [installer] = matches.as_slice() else {
        return Err(Error::InvalidRelease);
    };
    if installer.as_object().is_none_or(|fields| {
        fields.keys().any(|key| {
            !matches!(
                key.as_str(),
                "Architecture" | "InstallerUrl" | "InstallerSha256"
            )
        })
    }) {
        return Err(Error::UnsupportedSource);
    }
    let expected_url = format!(
        "https://downloads.claude.ai/claude-code-releases/{version}/win32-{architecture}/claude.exe"
    );
    let installer_url = installer["InstallerUrl"]
        .as_str()
        .ok_or(Error::InvalidRelease)?;
    if installer_url != expected_url {
        return Err(Error::InvalidRelease);
    }
    let sha256 = super::brew::decode_sha256(
        installer["InstallerSha256"]
            .as_str()
            .ok_or(Error::InvalidRelease)?,
    )?;
    let candidate = download(installer_url, 512 * 1024 * 1024).await?;
    if super::stamp(candidate.path())?.digest != sha256 {
        return Err(Error::InvalidRelease);
    }
    owner.verify_layout()?;
    if Owner::read()?.as_ref() != Some(&owner) || image(&owner.target)? != old {
        return Err(Error::SourceChanged);
    }
    let id = Uuid::new_v4();
    let program = owner.root.join(format!(".infinishell-winget-{id}.exe"));
    let mut prepared = PreparedCandidate {
        id,
        owner,
        program,
        version: version.to_owned(),
        sha256,
        manifest_sha256: Sha256::digest(bytes).into(),
        original: old,
        image: None,
        probe: None,
        intent: intent.to_owned(),
    };
    // 构造中断时仍保留归属记录，不按目录扫描删除未知 exe。
    save_candidate(root, &prepared)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(&prepared.program)
        .map_err(|_| Error::PersistenceFailed)?;
    std::io::copy(
        &mut File::open(candidate.path()).map_err(|_| Error::PersistenceFailed)?,
        &mut output,
    )
    .map_err(|_| Error::PersistenceFailed)?;
    output.sync_all().map_err(|_| Error::PersistenceFailed)?;
    drop(output);
    // 当前用户拥有的候选沿用原文件 DACL；外部改动还会由后续完整身份核验阻止发布。
    copy_security(&prepared.owner.target, &prepared.program)?;
    prepared.program = prepared
        .program
        .canonicalize()
        .map_err(|_| Error::SourceChanged)?;
    let identity = image(&prepared.program)?;
    if identity.sha256 != prepared.sha256 {
        return Err(Error::SourceChanged);
    }
    prepared.image = Some(identity);
    save_candidate(root, &prepared)?;
    Ok(prepared)
}

pub(super) async fn execute(
    plan: &UpdatePlan,
    root: &Path,
    progress: Option<VerificationProgress>,
) -> Result<String, Error> {
    super::winget::supports(plan.agent, &plan.target_version)?;
    if plan.config.is_some() {
        return Err(Error::ChannelMismatch);
    }
    super::verify_installation_identity(&plan.installation)?;
    let owner = Owner::read()?.ok_or(Error::SourceChanged)?;
    owner.verify_layout()?;
    if owner.version != plan.installed_version
        || owner
            .target
            .canonicalize()
            .map_err(|_| Error::SourceChanged)?
            != plan.installation.stamp.canonical
        || super::brew::decode_sha256(&owner.sha256)? != plan.installation.stamp.digest
    {
        return Err(Error::SourceChanged);
    }
    if path(root)
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::RecoveryRequired);
    }
    let mut prepared = prepare_candidate(owner, root, &plan.intent).await?;
    let result = async {
        let held = prepared.image.as_ref().ok_or(Error::RecoveryRequired)?;
        let expected = managed_process::ExpectedFileIdentity::capture_release_image(
            &prepared.program,
            held.length,
            prepared.sha256,
        )
        .map_err(|_| Error::SourceChanged)?;
        let binding_digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&prepared).map_err(|_| Error::PersistenceFailed)?)
        );
        let binding = managed_process::PreparedLaunchBinding::claude_winget_version_probe(
            binding_digest.clone(),
        )
        .map_err(|_| Error::UnsupportedPlatform)?;
        let generation = Uuid::new_v4();
        prepared.probe = Some(CandidateReceipt {
            generation,
            binding_digest,
            program: prepared.program.clone(),
            version: String::new(),
            sha256: prepared.sha256,
        });
        save_candidate(root, &prepared)?;
        let mut child = managed_process::spawn_bound_version_probe(
            root,
            generation,
            &prepared.program,
            expected,
            &binding,
        )
        .await
        .map_err(|_| Error::RecoveryRequired)?;
        let mut stdout = child.stdout.take().ok_or(Error::RecoveryRequired)?;
        let mut bytes = Vec::new();
        let output = (&mut stdout)
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .with_timeout(UPDATE_TIMEOUT)
            .await;
        drop(stdout);
        let receipt = if output.is_ok() {
            child.finish_after_stdin_close().await
        } else {
            child.finish().await
        }
        .map_err(|_| Error::RecoveryRequired)?;
        if !receipt.cleanup_confirmed {
            return Err(Error::RecoveryRequired);
        }
        match output {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return Err(Error::ProbeFailed),
            Err(_) => return Err(Error::TimedOut),
        }
        if receipt.exit_code != Some(0) || bytes.len() as u64 > MAX_OUTPUT {
            return Err(Error::ProbeFailed);
        }
        let observed = super::parse_cli_agent_version(
            CLIAgent::Claude,
            std::str::from_utf8(&bytes).map_err(|_| Error::ProbeFailed)?,
        )
        .ok_or(Error::ProbeFailed)?;
        if observed != prepared.version {
            return Err(Error::VersionMismatch);
        }
        prepared
            .probe
            .as_mut()
            .ok_or(Error::RecoveryRequired)?
            .version = observed;
        save_candidate(root, &prepared)?;
        if let Some(progress) = progress {
            progress.enter().await;
        }
        let probe = prepared.probe.take().ok_or(Error::RecoveryRequired)?;
        publish_verified_candidate(
            root,
            prepared.owner.clone(),
            prepared.id,
            probe,
            plan.intent.clone(),
            &prepared.original,
            prepared.image.as_ref().ok_or(Error::RecoveryRequired)?,
        )
    }
    .await;
    if result.is_err() {
        recover(CLIAgent::Claude, &prepared.owner.public, root)?;
    } else {
        cleanup_candidate(root, &prepared.owner.public)?;
    }
    result
}

fn path(root: &Path) -> PathBuf {
    root.join("claude-winget-portable.json")
}

fn save(root: &Path, journal: &Journal) -> Result<(), Error> {
    let mut temporary = NamedTempFile::new_in(root).map_err(|_| Error::PersistenceFailed)?;
    temporary
        .write_all(&serde_json::to_vec(journal).map_err(|_| Error::PersistenceFailed)?)
        .map_err(|_| Error::PersistenceFailed)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| Error::PersistenceFailed)?;
    temporary
        .persist(path(root))
        .map_err(|_| Error::PersistenceFailed)?;
    super::sync_config_directory(root)
}

fn verify_probe(root: &Path, receipt: &CandidateReceipt) -> Result<(), Error> {
    // 不能接受 native_file 或其他来源的探针收据替代 AppContainer 合同。
    let binding = managed_process::PreparedLaunchBinding::from_persisted(
        receipt.binding_digest.clone(),
        Some("claude_winget_version_probe_v1"),
    )
    .map_err(|_| Error::UnsupportedPlatform)?;
    let exited = managed_process::confirmed_exit_with_binding(root, receipt.generation, &binding)
        .map_err(|_| Error::RecoveryRequired)?
        .ok_or(Error::RecoveryRequired)?;
    if !exited.cleanup_confirmed || exited.exit_code != Some(0) || receipt.version != "2.1.280" {
        return Err(Error::RecoveryRequired);
    }
    Ok(())
}

fn hold_parent(path: &Path) -> Result<Vec<File>, Error> {
    let mut held = Vec::new();
    for ancestor in path
        .parent()
        .ok_or(Error::SourceChanged)?
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let file = OpenOptions::new()
            .read(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
            .custom_flags((FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS).0)
            .open(ancestor)
            .map_err(|_| Error::PermissionDenied)?;
        let mut identity = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut identity) }
            .map_err(|_| Error::SourceChanged)?;
        if identity.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0
            || identity.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
        {
            return Err(Error::UnsupportedSource);
        }
        held.push(file);
    }
    Ok(held)
}

fn hold_file(path: &Path, expected: &Image) -> Result<File, Error> {
    // DELETE 句柄由本事务持有，但不与其他打开者共享写入/删除，防止校验后替换对象。
    let file = OpenOptions::new()
        .access_mode(0x8000_0000 | DELETE.0 | READ_CONTROL.0)
        .share_mode(FILE_SHARE_READ.0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
        .map_err(|_| Error::SourceChanged)?;
    if image(path)? != *expected {
        return Err(Error::SourceChanged);
    }
    Ok(file)
}

fn rename_owned(file: &File, destination: &Path) -> Result<(), Error> {
    let name: Vec<_> = destination.as_os_str().encode_wide().collect();
    if name.is_empty() || name.len() > 32767 || name.contains(&0) || !destination.is_absolute() {
        return Err(Error::SourceChanged);
    }
    let length = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
    let mut storage = vec![0_usize; length.div_ceil(std::mem::size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast(),
            name.len(),
        );
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileRenameInfo,
            info.cast(),
            length as u32,
        )
    }
    .map_err(|_| Error::PersistenceFailed)
}

fn replace(
    target: &Path,
    candidate: &Path,
    backup: &Path,
    old: &Image,
    new: &Image,
) -> Result<(), Error> {
    if target.parent() != candidate.parent() || target.parent() != backup.parent() {
        return Err(Error::SourceChanged);
    }
    let _parents = hold_parent(target)?;
    let old_file = hold_file(target, old)?;
    let new_file = hold_file(candidate, new)?;
    // 两次均禁止覆盖已有目标；第二步失败由 journal 恢复。外部新建的目标永远不被覆盖。
    rename_owned(&old_file, backup)?;
    rename_owned(&new_file, target)
}

fn owned_arp(journal: &Journal) -> Result<Owner, Error> {
    let actual = Owner::read()?.ok_or(Error::SourceChanged)?;
    actual.verify_layout()?;
    let mut same = actual.clone();
    same.version = journal.owner.version.clone();
    same.sha256 = journal.owner.sha256.clone();
    if same != journal.owner
        || ![
            journal.owner.version.as_str(),
            journal.target_version.as_str(),
        ]
        .contains(&actual.version.as_str())
        || ![
            journal.owner.sha256.as_str(),
            journal.target_sha256.as_str(),
        ]
        .contains(&actual.sha256.as_str())
    {
        return Err(Error::SourceChanged);
    }
    Ok(actual)
}

fn publish_arp(journal: &Journal, target: bool) -> Result<(), Error> {
    owned_arp(journal)?;
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            format!(r"{UNINSTALL}\{}", journal.owner.product),
            KEY_READ | KEY_SET_VALUE,
        )
        .map_err(|_| Error::PermissionDenied)?;
    let (version, sha256) = if target {
        (&journal.target_version, &journal.target_sha256)
    } else {
        (&journal.owner.version, &journal.owner.sha256)
    };
    key.set_value("SHA256", sha256)
        .map_err(|_| Error::PersistenceFailed)?;
    // 第一项完成、第二项未完成时仍是可识别的两个版本字段组合，由 journal 继续或回滚。
    owned_arp(journal)?;
    key.set_value("DisplayVersion", version)
        .map_err(|_| Error::PersistenceFailed)?;
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegFlushKey(key: *mut std::ffi::c_void) -> i32;
    }
    if unsafe { RegFlushKey(key.raw_handle()) } != 0 {
        return Err(Error::PersistenceFailed);
    }
    let observed = owned_arp(journal)?;
    if observed.version != *version || observed.sha256 != *sha256 {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

/// 候选必须由官方目标清单校验，且已完成专用原生探针；不下载或运行未知 installer。
/// 调用方持有 agent 更新预约和 journal 锁；此入口不会自行绕过忙碌状态。
fn publish_verified_candidate(
    root: &Path,
    owner: Owner,
    id: Uuid,
    probe: CandidateReceipt,
    intent: String,
    expected_original: &Image,
    expected_candidate: &Image,
) -> Result<String, Error> {
    verify_probe(root, &probe)?;
    owner.verify_layout()?;
    if Owner::read()?.as_ref() != Some(&owner)
        || id.is_nil()
        || path(root)
            .try_exists()
            .map_err(|_| Error::RecoveryRequired)?
    {
        return Err(Error::SourceChanged);
    }
    let candidate = owner.root.join(format!(".infinishell-winget-{id}.exe"));
    let backup = owner.root.join(format!(".infinishell-winget-old-{id}.exe"));
    if probe.program != candidate.canonicalize().map_err(|_| Error::SourceChanged)? {
        return Err(Error::SourceChanged);
    }
    let original = image(&owner.target)?;
    let prepared = image(&candidate)?;
    if original != *expected_original
        || prepared != *expected_candidate
        || original.sha256 != super::brew::decode_sha256(&owner.sha256)?
        || prepared.sha256 != probe.sha256
        || original.volume != prepared.volume
    {
        return Err(Error::SourceChanged);
    }
    let target_sha256 = prepared
        .sha256
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect();
    let mut journal = Journal {
        schema: 1,
        id,
        owner,
        target_version: probe.version.clone(),
        target_sha256,
        candidate,
        backup,
        original,
        prepared,
        probe,
        phase: Phase::Prepared,
        intent,
    };
    save(root, &journal)?;
    let result = (|| {
        if image(&journal.owner.target)? != journal.original
            || image(&journal.candidate)? != journal.prepared
        {
            return Err(Error::SourceChanged);
        }
        owned_arp(&journal)?;
        journal.phase = Phase::ReplaceIntent;
        save(root, &journal)?;
        replace(
            &journal.owner.target,
            &journal.candidate,
            &journal.backup,
            &journal.original,
            &journal.prepared,
        )?;
        if image(&journal.owner.target)? != journal.prepared
            || image(&journal.backup)? != journal.original
        {
            return Err(Error::RecoveryRequired);
        }
        journal.phase = Phase::Registering;
        save(root, &journal)?;
        publish_arp(&journal, true)?;
        journal.phase = Phase::Committed;
        save(root, &journal)?;
        finish(root, &journal)?;
        Ok(journal.target_version.clone())
    })();
    if result.is_err() {
        recover(CLIAgent::Claude, &journal.owner.public, root)?;
    }
    result
}

fn remove_matching(path: &Path, expected: &Image) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let _parents = hold_parent(path)?;
            let file = hold_file(path, expected)?;
            let deletion = FILE_DISPOSITION_INFO { DeleteFile: true };
            // 按已核验句柄删除，不把最后一次摘要检查后的路径复用当作同一个对象。
            unsafe {
                SetFileInformationByHandle(
                    HANDLE(file.as_raw_handle()),
                    FileDispositionInfo,
                    &deletion as *const _ as *const std::ffi::c_void,
                    std::mem::size_of_val(&deletion) as u32,
                )
            }
            .map_err(|_| Error::RecoveryRequired)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::RecoveryRequired),
    }
}

fn finish(root: &Path, journal: &Journal) -> Result<(), Error> {
    verify_probe(root, &journal.probe)?;
    let arp = owned_arp(journal)?;
    if image(&journal.owner.target)? != journal.prepared
        || arp.version != journal.target_version
        || arp.sha256 != journal.target_sha256
    {
        return Err(Error::RecoveryRequired);
    }
    remove_matching(&journal.backup, &journal.original)?;
    fs::remove_file(path(root)).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)
}

fn recover_publication(
    agent: CLIAgent,
    entry: &Path,
    root: &Path,
) -> Result<Option<String>, Error> {
    if agent != CLIAgent::Claude
        || !path(root)
            .try_exists()
            .map_err(|_| Error::RecoveryRequired)?
    {
        return Ok(None);
    }
    let journal: Journal = serde_json::from_slice(&read_limited(&path(root), MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    if journal.schema != 1
        || journal.id.is_nil()
        || journal.target_version != "2.1.280"
        || entry != journal.owner.public && entry != journal.owner.target
        || journal.candidate
            != journal
                .owner
                .root
                .join(format!(".infinishell-winget-{}.exe", journal.id))
        || journal.backup
            != journal
                .owner
                .root
                .join(format!(".infinishell-winget-old-{}.exe", journal.id))
        || journal.probe.program
            != journal
                .owner
                .root
                .canonicalize()
                .map_err(|_| Error::SourceChanged)?
                .join(format!(".infinishell-winget-{}.exe", journal.id))
        || journal.probe.sha256 != journal.prepared.sha256
        || super::brew::decode_sha256(&journal.target_sha256)? != journal.prepared.sha256
    {
        return Err(Error::RecoveryRequired);
    }
    journal.owner.verify_layout()?;
    verify_probe(root, &journal.probe)?;
    if journal.phase == Phase::Committed {
        finish(root, &journal)?;
        return Ok(Some(journal.target_version));
    }
    owned_arp(&journal)?;
    if !journal
        .owner
        .target
        .try_exists()
        .map_err(|_| Error::SourceChanged)?
    {
        let _parents = hold_parent(&journal.owner.target)?;
        let backup = hold_file(&journal.backup, &journal.original)?;
        if image(&journal.candidate)? != journal.prepared {
            return Err(Error::SourceChanged);
        }
        rename_owned(&backup, &journal.owner.target)?;
    }
    let actual = image(&journal.owner.target)?;
    if actual == journal.prepared {
        if image(&journal.backup)? != journal.original {
            return Err(Error::RecoveryRequired);
        }
        // 回滚仍保留候选至登记回滚完成；不覆盖其他程序在同一路径发布的新文件。
        replace(
            &journal.owner.target,
            &journal.backup,
            &journal.candidate,
            &journal.prepared,
            &journal.original,
        )?;
    } else if actual != journal.original {
        return Err(Error::SourceChanged);
    }
    publish_arp(&journal, false)?;
    if image(&journal.owner.target)? != journal.original {
        return Err(Error::RecoveryRequired);
    }
    remove_matching(&journal.candidate, &journal.prepared)?;
    super::save_failure_with_intent(
        root,
        agent,
        entry,
        &journal.target_version,
        Some(journal.intent),
    )?;
    fs::remove_file(path(root)).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)?;
    Ok(None)
}

fn cleanup_candidate(root: &Path, entry: &Path) -> Result<(), Error> {
    let path = candidate_path(root);
    if !path.try_exists().map_err(|_| Error::RecoveryRequired)? {
        return Ok(());
    }
    let prepared: PreparedCandidate = serde_json::from_slice(&read_limited(&path, MAX_CONFIG)?)
        .map_err(|_| Error::RecoveryRequired)?;
    prepared.owner.verify_layout()?;
    let candidate = prepared
        .owner
        .root
        .join(format!(".infinishell-winget-{}.exe", prepared.id));
    let canonical = prepared
        .owner
        .root
        .canonicalize()
        .map_err(|_| Error::SourceChanged)?
        .join(candidate.file_name().ok_or(Error::SourceChanged)?);
    if prepared.id.is_nil()
        || prepared.version != "2.1.280"
        || entry != prepared.owner.public && entry != prepared.owner.target
        || prepared.program != candidate && prepared.program != canonical
    {
        return Err(Error::RecoveryRequired);
    }
    if let Some(probe) = &prepared.probe {
        if probe.program != canonical
            || probe.sha256 != prepared.sha256
            || !probe.version.is_empty() && probe.version != prepared.version
        {
            return Err(Error::RecoveryRequired);
        }
        let binding = managed_process::PreparedLaunchBinding::claude_winget_version_probe(
            probe.binding_digest.clone(),
        )
        .map_err(|_| Error::RecoveryRequired)?;
        super::record_not_started_if_missing(
            root,
            probe.generation,
            &probe.program,
            &["--version".into()],
            &binding,
        )?;
        let receipt =
            managed_process::confirmed_exit_with_binding(root, probe.generation, &binding)
                .map_err(|_| Error::RecoveryRequired)?
                .ok_or(Error::RecoveryRequired)?;
        if !receipt.cleanup_confirmed || !probe.version.is_empty() && receipt.exit_code != Some(0) {
            return Err(Error::RecoveryRequired);
        }
    }
    let current = image(&prepared.owner.target)?;
    let published = prepared
        .image
        .as_ref()
        .is_some_and(|image| *image == current);
    if current != prepared.original && !published {
        return Err(Error::SourceChanged);
    }
    let actual = Owner::read()?.ok_or(Error::SourceChanged)?;
    let mut expected = prepared.owner.clone();
    if published {
        expected.version = prepared.version.clone();
        expected.sha256 = prepared
            .sha256
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect();
    }
    if actual != expected {
        return Err(Error::SourceChanged);
    }
    if let Some(expected) = &prepared.image {
        if expected.sha256 != prepared.sha256 {
            return Err(Error::RecoveryRequired);
        }
        remove_matching(&candidate, expected)?;
    } else if candidate
        .try_exists()
        .map_err(|_| Error::RecoveryRequired)?
    {
        // 尚未封存身份的半成品不能按名称删除；保留明确恢复失败，避免认领外部替换文件。
        return Err(Error::RecoveryRequired);
    }
    if !published {
        super::save_failure_with_intent(
            root,
            CLIAgent::Claude,
            entry,
            &prepared.version,
            Some(prepared.intent),
        )?;
    }
    fs::remove_file(path).map_err(|_| Error::RecoveryRequired)?;
    super::sync_config_directory(root)
}

pub(super) fn recover(agent: CLIAgent, entry: &Path, root: &Path) -> Result<Option<String>, Error> {
    if agent != CLIAgent::Claude {
        return Ok(None);
    }
    let recovered = recover_publication(agent, entry, root)?;
    cleanup_candidate(root, entry)?;
    Ok(recovered)
}
