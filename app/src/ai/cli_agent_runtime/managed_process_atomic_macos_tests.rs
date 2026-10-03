use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use uuid::Uuid;

use super::*;

const BINDING_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct Fixture {
    _temporary: TempDir,
    root: PathBuf,
    state: PathBuf,
    source: PathBuf,
    expected: ExpectedFileIdentity,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let state = root.join("state");
        let installation = root.join("installation");
        create_private_directory(&state);
        create_private_directory(&installation);
        let source = installation.join("program");
        copy_signed_macho(&source);
        let expected = ExpectedFileIdentity::capture(&source).unwrap();
        Self {
            _temporary: temporary,
            root,
            state,
            source,
            expected,
        }
    }

    fn generation_path(&self, generation: Uuid) -> PathBuf {
        self.state.join(SNAPSHOT_ROOT).join(generation.to_string())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        make_tree_removable(&self.root);
    }
}

#[test]
fn replacement_after_handle_verification_is_rejected() {
    let fixture = Fixture::new();
    let generation = Uuid::new_v4();
    let source = fixture.source.clone();
    let replaced = fixture.root.join("replaced-program");

    let result = prepare_native_executable_inner(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
        || {
            fs::rename(&source, &replaced)?;
            fs::copy("/bin/cat", &source)?;
            fs::set_permissions(&source, fs::Permissions::from_mode(0o700))
        },
        || Ok(()),
    );

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_source_raced"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn same_contents_with_new_inode_are_rejected() {
    let fixture = Fixture::new();
    let generation = Uuid::new_v4();
    let source = fixture.source.clone();
    let replaced = fixture.root.join("replaced-program");
    let replacement = fixture.root.join("same-contents-new-inode");
    fs::copy(&source, &replacement).unwrap();
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o700)).unwrap();

    let result = prepare_native_executable_inner(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
        || {
            fs::rename(&source, &replaced)?;
            fs::rename(&replacement, &source)
        },
        || Ok(()),
    );

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_source_raced"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn symlink_to_program_outside_installation_is_rejected() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("outside-program");
    copy_signed_macho(&outside);
    fs::remove_file(&fixture.source).unwrap();
    symlink(&outside, &fixture.source).unwrap();
    let expected = ExpectedFileIdentity::capture(&fixture.source).unwrap();
    let generation = Uuid::new_v4();

    let result = prepare_native_executable(&fixture.state, generation, BINDING_DIGEST, &expected);

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_source_identity_invalid"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn stale_scratch_is_never_reused_as_generation() {
    let fixture = Fixture::new();
    let generation = Uuid::new_v4();
    let snapshot_root = fixture.state.join(SNAPSHOT_ROOT);
    create_private_directory(&snapshot_root);
    let stale = snapshot_root.join(format!(".scratch-{generation}-stale"));
    create_private_directory(&stale);
    fs::write(stale.join(PROGRAM_FILE), b"stale executable").unwrap();

    let published = prepare_native_executable(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
    )
    .unwrap();

    assert_eq!(stale.exists(), true);
    assert_eq!(published.execution_path().starts_with(&stale), false);
    assert_eq!(published.execution_path().is_file(), true);
    published.verify_for_execution().unwrap();
}

#[test]
fn duplicate_generation_never_replaces_first_snapshot() {
    let fixture = Fixture::new();
    let generation = Uuid::new_v4();
    let first = prepare_native_executable(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
    )
    .unwrap();
    let first_summary_sha256 = first.summary_sha256().to_owned();

    let result = prepare_native_executable(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
    );

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(first.summary_sha256(), first_summary_sha256);
    first.verify_for_execution().unwrap();
}

#[test]
fn changed_persisted_summary_is_rejected_before_execution() {
    let fixture = Fixture::new();
    let generation = Uuid::new_v4();
    let published = prepare_native_executable(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
    )
    .unwrap();
    let generation_directory = published.execution_path().parent().unwrap();
    let summary = generation_directory.join(SUMMARY_FILE);
    fs::set_permissions(
        generation_directory,
        fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE),
    )
    .unwrap();
    fs::set_permissions(&summary, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&summary, b"{}").unwrap();
    fs::set_permissions(&summary, fs::Permissions::from_mode(PUBLISHED_SUMMARY_MODE)).unwrap();
    fs::set_permissions(
        generation_directory,
        fs::Permissions::from_mode(PUBLISHED_DIRECTORY_MODE),
    )
    .unwrap();

    let result = published.verify_for_execution();

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_summary_changed"
    );
}

#[test]
fn invalid_signature_leaves_no_published_executable() {
    let fixture = Fixture::new();
    remove_embedded_signature(&fixture.source);
    let expected = ExpectedFileIdentity::capture(&fixture.source).unwrap();
    let generation = Uuid::new_v4();

    let result = prepare_native_executable(&fixture.state, generation, BINDING_DIGEST, &expected);

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_signature_invalid"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn failure_after_atomic_publish_withdraws_executable() {
    let fixture = Fixture::new();
    let generation = Uuid::new_v4();
    let summary = fixture.generation_path(generation).join(SUMMARY_FILE);

    let result = prepare_native_executable_inner(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
        || Ok(()),
        || fs::set_permissions(&summary, fs::Permissions::from_mode(0o600)),
    );

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_private_file_invalid"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn hardlinked_source_is_rejected() {
    let fixture = Fixture::new();
    fs::hard_link(&fixture.source, fixture.root.join("second-link")).unwrap();
    let expected = ExpectedFileIdentity::capture(&fixture.source).unwrap();
    let generation = Uuid::new_v4();

    let result = prepare_native_executable(&fixture.state, generation, BINDING_DIGEST, &expected);

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_source_file_unsafe"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn special_file_is_rejected_without_blocking() {
    let fixture = Fixture::new();
    fs::remove_file(&fixture.source).unwrap();
    let source = CString::new(fixture.source.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(source.as_ptr(), 0o700) }, 0);
    let metadata = fs::symlink_metadata(&fixture.source).unwrap();
    let expected = ExpectedFileIdentity {
        path: fixture.source.clone(),
        canonical_path: fixture.source.clone(),
        sha256: "0".repeat(64),
        size: metadata.len(),
        file_id: Some(ExpectedFileId {
            volume: metadata.dev(),
            index: metadata.ino(),
        }),
    };
    let generation = Uuid::new_v4();

    let result = prepare_native_executable(&fixture.state, generation, BINDING_DIGEST, &expected);

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_program_not_regular"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn writable_source_mode_is_rejected() {
    let fixture = Fixture::new();
    fs::set_permissions(&fixture.source, fs::Permissions::from_mode(0o722)).unwrap();
    let expected = ExpectedFileIdentity::capture(&fixture.source).unwrap();
    let generation = Uuid::new_v4();

    let result = prepare_native_executable(&fixture.state, generation, BINDING_DIGEST, &expected);

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_source_file_unsafe"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn writable_ancestor_is_rejected() {
    let fixture = Fixture::new();
    let installation = fixture.source.parent().unwrap();
    fs::set_permissions(installation, fs::Permissions::from_mode(0o777)).unwrap();
    let generation = Uuid::new_v4();

    let result = prepare_native_executable(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
    );

    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_ancestor_unsafe"
    );
    assert_eq!(fixture.generation_path(generation).exists(), false);
}

#[test]
fn published_program_and_summary_are_read_only_and_bound() {
    let fixture = Fixture::new();
    let generation = Uuid::new_v4();

    let published = prepare_native_executable(
        &fixture.state,
        generation,
        BINDING_DIGEST,
        &fixture.expected,
    )
    .unwrap();

    assert_eq!(
        fs::symlink_metadata(published.execution_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        PUBLISHED_PROGRAM_MODE
    );
    assert_eq!(published.summary().generation, generation);
    assert_eq!(published.summary().binding_digest, BINDING_DIGEST);
    assert_eq!(
        published.summary().program_relative_path,
        Path::new(PROGRAM_FILE)
    );
    published.verify_for_execution().unwrap();
}

#[test]
fn quarantine_attribute_is_preserved_byte_for_byte() {
    let fixture = Fixture::new();
    let quarantine_name = CString::new("com.apple.quarantine").unwrap();
    let quarantine_value = b"0181;60000000;;00000000-0000-0000-0000-000000000000";
    let source = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&fixture.source)
        .unwrap();
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                source.as_raw_fd(),
                quarantine_name.as_ptr(),
                quarantine_value.as_ptr().cast(),
                quarantine_value.len(),
                0,
                0,
            )
        },
        0
    );
    drop(source);
    let expected = ExpectedFileIdentity::capture(&fixture.source).unwrap();
    let generation = Uuid::new_v4();

    let published =
        prepare_native_executable(&fixture.state, generation, BINDING_DIGEST, &expected).unwrap();

    let source = fs::File::open(&fixture.source).unwrap();
    let snapshot = fs::File::open(published.execution_path()).unwrap();
    assert_eq!(
        extended_attributes(&snapshot).unwrap(),
        extended_attributes(&source).unwrap()
    );
    assert!(
        extended_attributes(&snapshot)
            .unwrap()
            .iter()
            .any(|(name, value)| { name == b"com.apple.quarantine" && value == quarantine_value })
    );
    published.verify_for_execution().unwrap();
}

#[test]
fn system_macho_dependency_closure_is_accepted() {
    verify_system_dependency_closure(Path::new("/bin/ls")).unwrap();
}

fn create_private_directory(path: &Path) {
    fs::DirBuilder::new()
        .recursive(false)
        .mode(PRIVATE_DIRECTORY_MODE)
        .create(path)
        .unwrap();
}

fn copy_signed_macho(path: &Path) {
    fs::copy("/bin/ls", path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn remove_embedded_signature(path: &Path) {
    let status = command::blocking::Command::new("/usr/bin/codesign")
        .args([
            std::ffi::OsStr::new("--remove-signature"),
            std::ffi::OsStr::new("--"),
            path.as_os_str(),
        ])
        .status()
        .unwrap();
    assert_eq!(status.success(), true);
}

fn make_tree_removable(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_dir() {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE));
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                make_tree_removable(&entry.path());
            }
        }
    } else {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
fn homebrew_fixture() -> (Fixture, Manifest) {
    let mut fixture = Fixture::new();
    let prefix = fixture.root.join("brew");
    create_private_directory(&prefix);
    let caskroom = prefix.join("Caskroom");
    create_private_directory(&caskroom);
    fs::set_permissions(&caskroom, fs::Permissions::from_mode(0o775)).unwrap();
    let source = caskroom.join("codex/0.160.0/bin/codex");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::rename(&fixture.source, &source).unwrap();
    fixture.source = source.clone();
    fixture.expected = ExpectedFileIdentity::capture(&source).unwrap();
    let generation = Uuid::new_v4();
    let cwd = version_probe::prepare_directory(&fixture.state, generation).unwrap();
    let manifest = Manifest {
        version: 1,
        launch_allowed: true,
        generation,
        token: Uuid::new_v4(),
        parent_control: "127.0.0.1:1234".parse().unwrap(),
        executable: source,
        arguments: vec!["--version".into()],
        atomic_cwd: Some(super::super::AtomicDirectoryIdentity::capture(&cwd).unwrap()),
        cwd,
        expected_files: vec![fixture.expected.clone()],
        grok_stdio_eof: None,
        isolated_home: None,
        isolated_state_dir: None,
        environment: None,
        atomic_launch_kind: Some(AtomicLaunchKind::CodexHomebrewVersionProbeV1),
    };
    (fixture, manifest)
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn private_homebrew_caskroom_allows_only_the_bound_probe_snapshot() {
    let (fixture, manifest) = homebrew_fixture();
    let strict = prepare_native_executable(
        &fixture.state,
        manifest.generation,
        BINDING_DIGEST,
        &fixture.expected,
    );
    assert_eq!(
        strict.unwrap_err().to_string(),
        "managed_process.atomic_macos_ancestor_unsafe"
    );
    let published =
        prepare_homebrew_probe_executable(&fixture.state, BINDING_DIGEST, &manifest).unwrap();
    published.verify_for_execution().unwrap();
    assert_eq!(
        published.summary().source_file_id,
        PersistentFileId::from(fixture.expected.file_id.unwrap())
    );
    assert_eq!(published.summary().sha256, fixture.expected.sha256);
    assert_eq!(
        published.summary().signature_requirement,
        CODESIGN_REQUIREMENT
    );
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn homebrew_ancestor_exception_rejects_generic_native_authority() {
    let (fixture, mut manifest) = homebrew_fixture();
    manifest.atomic_launch_kind = Some(AtomicLaunchKind::NativeFile);
    let result = prepare_homebrew_probe_executable(&fixture.state, BINDING_DIGEST, &manifest);
    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_homebrew_probe_kind_invalid"
    );
    assert!(!fixture.generation_path(manifest.generation).exists());
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn group_writable_caskroom_under_public_prefix_is_rejected() {
    let (fixture, manifest) = homebrew_fixture();
    fs::set_permissions(fixture.root.join("brew"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(prepare_homebrew_probe_executable(&fixture.state, BINDING_DIGEST, &manifest).is_err());
    assert!(!fixture.generation_path(manifest.generation).exists());
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn homebrew_exception_does_not_accept_another_writable_ancestor() {
    let (fixture, manifest) = homebrew_fixture();
    fs::set_permissions(
        fixture.root.join("brew/Caskroom/codex"),
        fs::Permissions::from_mode(0o775),
    )
    .unwrap();
    let result = prepare_homebrew_probe_executable(&fixture.state, BINDING_DIGEST, &manifest);
    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_ancestor_unsafe"
    );
    assert!(!fixture.generation_path(manifest.generation).exists());
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn homebrew_exception_does_not_relax_snapshot_state_permissions() {
    let (fixture, manifest) = homebrew_fixture();
    fs::set_permissions(&fixture.state, fs::Permissions::from_mode(0o775)).unwrap();
    let result = prepare_homebrew_probe_executable(&fixture.state, BINDING_DIGEST, &manifest);
    assert_eq!(
        result.unwrap_err().to_string(),
        "managed_process.atomic_macos_ancestor_unsafe"
    );
    assert!(!fixture.generation_path(manifest.generation).exists());
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn caskroom_permission_change_after_source_verification_is_rejected() {
    let (fixture, manifest) = homebrew_fixture();
    let caskroom = PrivateHomebrewCaskroom::capture(&fixture.source).unwrap();
    let result = prepare_executable_inner(
        &fixture.state,
        manifest.generation,
        BINDING_DIGEST,
        &fixture.expected,
        Some(&caskroom),
        || fs::set_permissions(&caskroom.caskroom_path, fs::Permissions::from_mode(0o777)),
        || Ok(()),
    );
    assert!(result.is_err());
    assert!(!fixture.generation_path(manifest.generation).exists());
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn caskroom_replacement_after_source_verification_is_rejected() {
    let (fixture, manifest) = homebrew_fixture();
    let caskroom = PrivateHomebrewCaskroom::capture(&fixture.source).unwrap();
    let result = prepare_executable_inner(
        &fixture.state,
        manifest.generation,
        BINDING_DIGEST,
        &fixture.expected,
        Some(&caskroom),
        || {
            fs::rename(
                &caskroom.caskroom_path,
                fixture.root.join("retained-caskroom"),
            )?;
            fs::create_dir(&caskroom.caskroom_path)?;
            fs::set_permissions(&caskroom.caskroom_path, fs::Permissions::from_mode(0o775))
        },
        || Ok(()),
    );
    assert!(result.is_err());
    assert!(!fixture.generation_path(manifest.generation).exists());
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn caskroom_symlink_cannot_substitute_for_the_original_directory() {
    let (fixture, manifest) = homebrew_fixture();
    let caskroom = fixture.root.join("brew/Caskroom");
    let outside = fixture.root.join("outside-caskroom");
    fs::rename(&caskroom, &outside).unwrap();
    symlink(&outside, &caskroom).unwrap();
    assert!(prepare_homebrew_probe_executable(&fixture.state, BINDING_DIGEST, &manifest).is_err());
    assert!(!fixture.generation_path(manifest.generation).exists());
}

#[cfg(all(feature = "local_fs", target_arch = "aarch64"))]
#[test]
fn caskroom_acl_added_after_capture_is_rejected() {
    let (fixture, manifest) = homebrew_fixture();
    let caskroom = PrivateHomebrewCaskroom::capture(&fixture.source).unwrap();
    let status = command::blocking::Command::new("/bin/chmod")
        .args(["+a", "everyone allow read"])
        .arg(&caskroom.caskroom_path)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(caskroom.verify().is_err());
    assert!(prepare_homebrew_probe_executable(&fixture.state, BINDING_DIGEST, &manifest).is_err());
    assert!(!fixture.generation_path(manifest.generation).exists());
}
