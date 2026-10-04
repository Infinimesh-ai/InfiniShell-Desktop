use super::*;

// 只构造真实发布布局；小文件不冒充固定发行映像，也不调用发行合同验证。
pub(in super::super::super) fn published_fixture(
    bin: &Path,
    id: Uuid,
    existing_new: bool,
) -> Mirror {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    std::fs::create_dir_all(bin).unwrap();
    for (name, bytes) in [("grok-1.0.40", b"old image"), ("grok-1.0.41", b"new image")] {
        std::fs::write(bin.join(name), bytes).unwrap();
        std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let directory = Directory::open(bin).unwrap();
    let old_name = OsString::from("grok-1.0.40");
    let new_name = OsString::from("grok-1.0.41");
    let link_stage = OsString::from(format!(".infinishell-grok-npm-link-{id}"));
    symlink(&old_name, bin.join("grok")).unwrap();
    symlink(&new_name, bin.join(&link_stage)).unwrap();
    let old_file = file_node(&directory, &old_name).unwrap();
    let new_file = file_node(&directory, &new_name).unwrap();
    let record = Mirror {
        bin: bin.to_owned(),
        directory: directory.identity().unwrap(),
        old_name,
        old_file,
        new_name,
        existing_new: existing_new.then(|| new_file.clone()),
        file_stage: format!(".infinishell-grok-npm-image-{id}").into(),
        old_link: link(&directory, OsStr::new("grok")).unwrap(),
        new_file: Some(new_file),
        new_link: Some(link(&directory, &link_stage).unwrap()),
        link_stage,
    };
    directory
        .exchange(OsStr::new("grok"), &record.link_stage)
        .unwrap();
    record
}

pub(in super::super::super) fn change_new_readonly_acl(record: &Mirror) {
    let directory = Directory::open(&record.bin).unwrap();
    let file = directory.read_file(&record.new_name).unwrap();
    let before = acl::capture(&file).unwrap();
    let desired = acl::readonly_file_test_acl(file.metadata().unwrap().mode() & 0o777).unwrap();
    assert_ne!(before.acl, desired);
    acl::apply_to_new(&file, &before, &desired).unwrap();
}

fn mirror(old: &str, target: &str, id: Uuid) -> Mirror {
    let image = contract::native(old).unwrap();
    let identity = Identity {
        device: 1,
        inode: 2,
        uid: 501,
        gid: 20,
        mode: u32::from(libc::S_IFREG) | 0o700,
        acl: None,
    };
    Mirror {
        bin: PathBuf::from("/private/grok-home/bin"),
        directory: Identity {
            mode: u32::from(libc::S_IFDIR) | 0o700,
            ..identity.clone()
        },
        old_name: format!("grok-{old}").into(),
        old_file: Node {
            identity,
            length: image.length,
            sha256: Some(image.digest().unwrap()),
        },
        new_name: format!("grok-{target}").into(),
        existing_new: None,
        file_stage: format!(".infinishell-grok-npm-image-{id}").into(),
        link_stage: format!(".infinishell-grok-npm-link-{id}").into(),
        old_link: Link {
            device: 1,
            inode: 3,
            uid: 501,
            gid: 20,
            mode: u32::from(libc::S_IFLNK) | 0o700,
            target: format!("grok-{old}").into(),
        },
        new_file: None,
        new_link: None,
    }
}

#[test]
fn saved_legacy_mirror_still_binds_the_41_target() {
    let id = Uuid::new_v4();
    let saved = serde_json::to_vec(&mirror("1.0.40", "1.0.41", id)).unwrap();
    let restored: Mirror = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored.new_name, OsString::from("grok-1.0.41"));
    assert_eq!(
        restored.validate(Path::new("/private/grok-home"), "1.0.40", "1.0.41", id),
        Ok(())
    );
    assert!(
        restored
            .validate(Path::new("/private/grok-home"), "1.0.40", "1.0.46", id)
            .is_err()
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn saved_current_mirror_rejects_a_different_target_or_native_image() {
    let id = Uuid::new_v4();
    let saved = serde_json::to_vec(&mirror("1.0.41", "1.0.46", id)).unwrap();
    let mut restored: Mirror = serde_json::from_slice(&saved).unwrap();
    assert_eq!(
        restored.validate(Path::new("/private/grok-home"), "1.0.41", "1.0.46", id),
        Ok(())
    );
    assert!(
        restored
            .validate(Path::new("/private/grok-home"), "1.0.41", "1.0.41", id)
            .is_err()
    );
    // 已存在的新映像不能用旧映像的有效摘要冒充。
    restored.existing_new = Some(restored.old_file.clone());
    assert!(
        restored
            .validate(Path::new("/private/grok-home"), "1.0.41", "1.0.46", id)
            .is_err()
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_link_name_accepts_only_a_reviewed_native_version() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().unwrap();
    let directory = Directory::open(&temporary.path().canonicalize().unwrap()).unwrap();
    let path = temporary.path().join("grok");
    symlink("grok-1.0.46", &path).unwrap();
    assert_eq!(
        link(&directory, OsStr::new("grok")).unwrap().target,
        OsString::from("grok-1.0.46")
    );
    std::fs::remove_file(&path).unwrap();
    symlink("grok-1.0.47", &path).unwrap();
    assert_eq!(
        link(&directory, OsStr::new("grok")),
        Err(Error::UnsupportedSource)
    );
    std::fs::remove_file(&path).unwrap();
    symlink("../grok-1.0.46", &path).unwrap();
    assert_eq!(
        link(&directory, OsStr::new("grok")),
        Err(Error::UnsupportedSource)
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn prepare_rejects_wrong_target_before_reading_or_writing_files() {
    let mut record = mirror("1.0.41", "1.0.46", Uuid::new_v4());
    let mut input = tempfile::tempfile().unwrap();
    assert_eq!(
        record.prepare(&mut input, "1.0.41"),
        Err(Error::RecoveryRequired)
    );
    assert_eq!(input.metadata().unwrap().len(), 0);
}
