use uuid::Uuid;
use windows_registry::Type;

use super::*;

const CLASSES: &str = "Software\\Classes";
const SCHEME: &str = "infinishell-fixture";
const APP_ID: &str = "test.infinishell.InfiniShellFixture";
const SCHEME_KEY: &str = "Software\\Classes\\infinishell-fixture";
const COMMAND_KEY: &str = "Software\\Classes\\infinishell-fixture\\shell\\open\\command";
const AUMID_KEY: &str = "Software\\Classes\\AppUserModelId\\test.infinishell.InfiniShellFixture";

struct IsolatedRegistry {
    path: String,
    root: Key,
}

impl IsolatedRegistry {
    fn new() -> Self {
        // 不重定向 HKCU，也不访问 Software\Classes 下的真实产品键。
        let path = format!("Software\\InfiniShellRegistryTest-{}", Uuid::new_v4());
        let root = CURRENT_USER.create(&path).unwrap();
        Self { path, root }
    }

    fn register(&self, executable: &str) -> Result<()> {
        register_uri_handler_at(
            &self.root,
            CLASSES,
            SCHEME,
            "InfiniShell Fixture",
            APP_ID,
            Path::new(executable),
        )
    }
}

impl Drop for IsolatedRegistry {
    fn drop(&mut self) {
        CURRENT_USER.remove_tree(&self.path).unwrap();
    }
}

#[test]
fn creates_missing_classes_uri_and_aumid() {
    let fixture = IsolatedRegistry::new();
    assert!(fixture.root.open(CLASSES).is_err());

    fixture.register(r"C:\Fixture\infinishell.exe").unwrap();

    let scheme = fixture.root.open(SCHEME_KEY).unwrap();
    assert_eq!(scheme.get_string("").unwrap(), "InfiniShell Fixture");
    assert_eq!(scheme.get_string("URL Protocol").unwrap(), "");
    assert_eq!(
        scheme.get_string(OWNER_PATH).unwrap(),
        r"C:\Fixture\infinishell.exe"
    );
    assert_eq!(
        fixture
            .root
            .open(COMMAND_KEY)
            .unwrap()
            .get_string("")
            .unwrap(),
        r#""C:\Fixture\infinishell.exe" "%1""#
    );
    let aumid = fixture.root.open(AUMID_KEY).unwrap();
    assert_eq!(
        aumid.get_string("DisplayName").unwrap(),
        "InfiniShellFixture"
    );
    assert_eq!(
        aumid.get_string(OWNER_PATH).unwrap(),
        r"C:\Fixture\infinishell.exe"
    );
}

#[test]
fn repairs_legacy_command_without_deleting_unrelated_values() {
    let fixture = IsolatedRegistry::new();
    let scheme = fixture.root.create(SCHEME_KEY).unwrap();
    scheme.set_u32("URL Protocol", 1).unwrap();
    scheme.set_string("KeepFixtureValue", "untouched").unwrap();
    fixture
        .root
        .create(COMMAND_KEY)
        .unwrap()
        .set_string("", r#""C:\RemovedOldVersion\infinishell.exe" "%0""#)
        .unwrap();
    let other_scheme = fixture
        .root
        .create("Software\\Classes\\other-fixture")
        .unwrap();
    other_scheme.set_string("", "other owner").unwrap();

    fixture.register(r"C:\Current\infinishell.exe").unwrap();

    assert_eq!(
        fixture
            .root
            .open(COMMAND_KEY)
            .unwrap()
            .get_string("")
            .unwrap(),
        r#""C:\Current\infinishell.exe" "%1""#
    );
    assert_eq!(scheme.get_string("URL Protocol").unwrap(), "");
    assert_eq!(scheme.get_type("URL Protocol").unwrap(), Type::String);
    assert_eq!(scheme.get_string("KeepFixtureValue").unwrap(), "untouched");
    assert_eq!(other_scheme.get_string("").unwrap(), "other owner");
}

#[test]
fn updates_uri_and_aumid_ownership_to_running_instance() {
    let fixture = IsolatedRegistry::new();
    fixture.register(r"C:\InstanceA\infinishell.exe").unwrap();

    fixture.register(r"C:\InstanceB\infinishell.exe").unwrap();

    assert_eq!(
        fixture
            .root
            .open(COMMAND_KEY)
            .unwrap()
            .get_string("")
            .unwrap(),
        r#""C:\InstanceB\infinishell.exe" "%1""#
    );
    assert_eq!(
        fixture
            .root
            .open(SCHEME_KEY)
            .unwrap()
            .get_string(OWNER_PATH)
            .unwrap(),
        r"C:\InstanceB\infinishell.exe"
    );
    assert_eq!(
        fixture
            .root
            .open(AUMID_KEY)
            .unwrap()
            .get_string(OWNER_PATH)
            .unwrap(),
        r"C:\InstanceB\infinishell.exe"
    );
}

#[test]
fn writes_quoted_unicode_paths_with_terminated_registry_strings() {
    let fixture = IsolatedRegistry::new();

    fixture
        .register(r"C:\测试 Program Files\infinishell.exe")
        .unwrap();

    let command = fixture.root.open(COMMAND_KEY).unwrap();
    assert_eq!(
        command.get_string("").unwrap(),
        r#""C:\测试 Program Files\infinishell.exe" "%1""#
    );
    assert_eq!(command.get_type("").unwrap(), Type::String);
    assert!(command.get_bytes("").unwrap().ends_with(&[0, 0]));
    let scheme = fixture.root.open(SCHEME_KEY).unwrap();
    assert_eq!(
        scheme.get_string(OWNER_PATH).unwrap(),
        r"C:\测试 Program Files\infinishell.exe"
    );
    assert!(scheme.get_bytes(OWNER_PATH).unwrap().ends_with(&[0, 0]));
    assert_eq!(scheme.get_bytes("URL Protocol").unwrap(), vec![0, 0]);
    let aumid = fixture.root.open(AUMID_KEY).unwrap();
    assert!(aumid.get_bytes(OWNER_PATH).unwrap().ends_with(&[0, 0]));
}

#[test]
fn registration_can_create_children_from_read_only_parent_handle() {
    let fixture = IsolatedRegistry::new();
    // open 只请求 KEY_READ；子键 create 单独请求写权限，不能混淆两者。
    let read_only_root = CURRENT_USER.open(&fixture.path).unwrap();

    register_uri_handler_at(
        &read_only_root,
        CLASSES,
        SCHEME,
        "InfiniShell Fixture",
        APP_ID,
        Path::new(r"C:\Fixture\infinishell.exe"),
    )
    .unwrap();

    assert_eq!(
        fixture
            .root
            .open(COMMAND_KEY)
            .unwrap()
            .get_string("")
            .unwrap(),
        r#""C:\Fixture\infinishell.exe" "%1""#
    );
}

#[test]
fn registration_propagates_registry_stage_and_win32_error() {
    let error = register_uri_handler_at(
        &Key::default(),
        CLASSES,
        SCHEME,
        "InfiniShell Fixture",
        APP_ID,
        Path::new(r"C:\Fixture\infinishell.exe"),
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Could not create or open URI classes key"
    );
    assert!(error
        .downcast_ref::<windows_result::Error>()
        .unwrap()
        .code()
        .is_err());
}
