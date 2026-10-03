use super::*;

fn package(root: &Path) -> PathBuf {
    let package = root.join("package");
    let public = package.join("bin/claude.exe");
    let native = package.join("node_modules/@anthropic-ai/claude-code-win32-x64/claude.exe");
    fs::create_dir_all(public.parent().unwrap()).unwrap();
    fs::create_dir_all(native.parent().unwrap()).unwrap();
    fs::write(&public, b"hardlink cleanup fixture").unwrap();
    fs::hard_link(&public, &native).unwrap();
    package
}

#[test]
fn interrupted_cleanup_accepts_only_the_remaining_name_of_the_known_native_pair() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = package(&root);
    let original = snapshot(&package).unwrap();
    let backup = root.join("backup");
    rename(&package, &backup, &original).unwrap();
    fs::remove_file(backup.join("node_modules/@anthropic-ai/claude-code-win32-x64/claude.exe"))
        .unwrap();
    remove_matching(&backup, &original).unwrap();
    assert!(!backup.exists());
}

#[test]
fn package_hardlinks_reject_external_names_and_changed_remaining_files() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let package = package(&root);
    let original = snapshot(&package).unwrap();
    let outside = root.join("outside.exe");
    fs::hard_link(package.join("bin/claude.exe"), &outside).unwrap();
    assert!(snapshot(&package).is_err());
    fs::remove_file(outside).unwrap();
    fs::remove_file(package.join("node_modules/@anthropic-ai/claude-code-win32-x64/claude.exe"))
        .unwrap();
    fs::write(package.join("bin/claude.exe"), b"externally modified").unwrap();
    assert!(remove_matching(&package, &original).is_err());
    assert_eq!(
        fs::read(package.join("bin/claude.exe")).unwrap(),
        b"externally modified"
    );
}
