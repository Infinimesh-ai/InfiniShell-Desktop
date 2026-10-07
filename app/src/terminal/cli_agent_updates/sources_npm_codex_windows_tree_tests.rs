use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows::Win32::Foundation::ERROR_SHARING_VIOLATION;
use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READONLY, SEC_IMAGE};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;

use super::*;

fn package_at(root: &Path, name: &str) -> PathBuf {
    let package = root.join(name);
    fs::create_dir_all(package.join("bin")).unwrap();
    // 只按扩展名进入生产冻结路径；这些夹具字节不会执行。
    fs::write(package.join("bin/native.exe"), b"image freeze fixture").unwrap();
    fs::write(package.join("package.json"), b"{}").unwrap();
    package
}

#[test]
fn held_image_freeze_rejects_package_rename_without_changing_tree() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = package_at(&root, "package");
    let expected = snapshot(&package).unwrap();
    let backup = root.join("backup");
    let frozen = freeze_images(&package, &expected).unwrap();
    assert_eq!(frozen.len(), 1);

    assert_eq!(
        rename(&package, &backup, &expected),
        Err(Error::PersistenceFailed)
    );

    assert_eq!(snapshot(&package).unwrap(), expected);
    assert!(!backup.exists());
    drop(frozen);
}

#[test]
fn inactive_image_publish_keeps_backup_frozen_until_stage_is_published() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = package_at(&root, "package");
    let stage = package_at(&root, "stage");
    fs::write(stage.join("bin/native.exe"), b"updated image fixture").unwrap();
    let expected = snapshot(&package).unwrap();
    let prepared = snapshot(&stage).unwrap();
    let backup = root.join("backup");
    let frozen = rename_inactive_images(&package, &backup, &expected).unwrap();
    assert_eq!(frozen.len(), 1);
    assert!(!package.exists());
    assert_eq!(snapshot(&backup).unwrap(), expected);
    assert_eq!(
        OpenOptions::new()
            .write(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
            .open(backup.join("bin/native.exe"))
            .unwrap_err()
            .raw_os_error(),
        Some(ERROR_SHARING_VIOLATION.0 as i32)
    );

    rename(&stage, &package, &prepared).unwrap();
    assert!(!stage.exists());
    assert_eq!(snapshot(&package).unwrap(), prepared);
    assert_eq!(snapshot(&backup).unwrap(), expected);
    drop(frozen);
    drop(
        OpenOptions::new()
            .write(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
            .open(backup.join("bin/native.exe"))
            .unwrap(),
    );
    remove_matching(&backup, &expected).unwrap();
    assert!(!backup.exists());
    assert_eq!(snapshot(&package).unwrap(), prepared);
}

#[test]
fn inactive_image_recovery_keeps_stage_frozen_until_backup_is_restored() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = package_at(&root, "package");
    let backup = package_at(&root, "backup");
    fs::write(package.join("bin/native.exe"), b"updated image fixture").unwrap();
    let prepared = snapshot(&package).unwrap();
    let original = snapshot(&backup).unwrap();
    let stage = root.join("stage");

    let frozen = rename_inactive_images(&package, &stage, &prepared).unwrap();
    assert_eq!(frozen.len(), 1);
    rename(&backup, &package, &original).unwrap();
    assert!(!backup.exists());
    assert_eq!(snapshot(&package).unwrap(), original);
    assert_eq!(snapshot(&stage).unwrap(), prepared);
    assert_eq!(
        OpenOptions::new()
            .write(true)
            .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
            .open(stage.join("bin/native.exe"))
            .unwrap_err()
            .raw_os_error(),
        Some(ERROR_SHARING_VIOLATION.0 as i32)
    );
    drop(frozen);
    remove_matching(&stage, &prepared).unwrap();
    assert!(!stage.exists());
    assert_eq!(snapshot(&package).unwrap(), original);
}

#[test]
fn independent_reader_rejects_inactive_rename_without_changing_tree() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = package_at(&root, "package");
    let expected = snapshot(&package).unwrap();
    let backup = root.join("backup");
    // 这是生产冻结句柄之外的独立打开；允许全部共享仍不等于允许父目录改名。
    let independent = OpenOptions::new()
        .read(true)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .open(package.join("bin/native.exe"))
        .unwrap();
    assert_eq!(
        rename_inactive_images(&package, &backup, &expected).unwrap_err(),
        Error::PersistenceFailed
    );
    assert_eq!(snapshot(&package).unwrap(), expected);
    assert!(!backup.exists());

    drop(independent);
    let frozen = rename_inactive_images(&package, &backup, &expected).unwrap();
    assert!(!package.exists());
    assert_eq!(snapshot(&backup).unwrap(), expected);
    drop(frozen);
}

fn retain_image_section(package: &Path) -> OwnedHandle {
    let mut system = [0u16; 32768];
    let length = unsafe { GetSystemDirectoryW(Some(&mut system)) } as usize;
    assert!(length > 0 && length < system.len());
    let source = PathBuf::from(OsString::from_wide(&system[..length])).join("whoami.exe");
    let image = package.join("bin/native.exe");
    fs::copy(source, &image).unwrap();
    let file = OpenOptions::new()
        .read(true)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_DELETE).0)
        .open(&image)
        .unwrap();
    // 仅保留真实 PE 的 SEC_IMAGE 节对象，不创建进程，也不执行映像入口。
    let mapping = unsafe {
        CreateFileMappingW(
            HANDLE(file.as_raw_handle()),
            None,
            PAGE_READONLY | SEC_IMAGE,
            0,
            0,
            None,
        )
    }
    .unwrap();
    let mapping = unsafe { OwnedHandle::from_raw_handle(mapping.0) };
    drop(file);
    mapping
}

#[test]
fn retained_image_section_rejects_inactive_rename_without_changing_tree() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = package_at(&root, "package");
    let mapping = retain_image_section(&package);
    let expected = snapshot(&package).unwrap();
    let backup = root.join("backup");

    assert_eq!(
        rename_inactive_images(&package, &backup, &expected).unwrap_err(),
        Error::SourceChanged
    );
    assert_eq!(snapshot(&package).unwrap(), expected);
    assert!(!backup.exists());
    drop(mapping);
}

#[test]
fn retained_image_section_rejects_freeze_after_original_file_is_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = package_at(&root, "package");
    let mapping = retain_image_section(&package);
    let expected = snapshot(&package).unwrap();
    let backup = root.join("backup");

    assert_eq!(
        freeze_images(&package, &expected).unwrap_err(),
        Error::SourceChanged
    );
    assert_eq!(snapshot(&package).unwrap(), expected);

    let renamed = rename(&package, &backup, &expected);
    eprintln!("保留 SEC_IMAGE 节对象时的生产父目录改名结果：{renamed:?}");
    // 记录实际文件系统行为，不能预先把映像节与普通打开句柄等同。
    match renamed {
        Ok(()) => {
            assert!(!package.exists());
            assert_eq!(snapshot(&backup).unwrap(), expected);
            assert_eq!(
                freeze_images(&backup, &expected).unwrap_err(),
                Error::SourceChanged
            );
            drop(mapping);
            drop(freeze_images(&backup, &expected).unwrap());
        }
        Err(error) => {
            assert_eq!(error, Error::PersistenceFailed);
            assert_eq!(snapshot(&package).unwrap(), expected);
            assert!(!backup.exists());
            drop(mapping);
            drop(freeze_images(&package, &expected).unwrap());
            rename(&package, &backup, &expected).unwrap();
            assert_eq!(snapshot(&backup).unwrap(), expected);
        }
    }
}
