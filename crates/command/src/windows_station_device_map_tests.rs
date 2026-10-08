use super::*;

#[test]
fn mapping_target_requires_a_normal_local_subdirectory() {
    assert_eq!(
        physical_parts(r"\\?\C:\private\candidate").unwrap(),
        ("C:", r"private\candidate")
    );
    for path in [
        r"C:\private\candidate",
        r"\\?\C:\",
        r"\\?\UNC\server\share\candidate",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1\candidate",
        r"\\?\C:\..\candidate",
        r"\\?\C:\private\\candidate",
        r"\\?\C:\private\candidate:stream",
        r"\\?\C:\private/candidate",
        "\\\\?\\C:\\private\0candidate",
    ] {
        assert!(physical_parts(path).is_err(), "{path:?}");
    }
}

#[test]
fn mapping_alias_cannot_name_global_objects_or_relative_paths() {
    for value in [r"D:\", r"Z:\"] {
        assert!(alias(Path::new(value)).is_ok());
    }
    for value in [
        r"C:\",
        r"d:\",
        "D:",
        r"D:\child",
        r"Global\D:",
        r"\\?\D:\",
        r"D:\\",
        "D:\\\0",
    ] {
        assert!(alias(Path::new(value)).is_err(), "{value:?}");
    }
}

#[test]
fn mapping_receipt_binds_target_identity_and_exact_logon() {
    let owner = Snapshot {
        pid: 30,
        created: 3000,
        auth_low: 21,
        auth_high: 0,
        session: 1,
        user: "S-1-5-21-1-2-3-1001".to_owned(),
        integrity: "S-1-16-8192".to_owned(),
        elevated: 0,
        token_type: 1,
    };
    let target = Target {
        path: PathBuf::from(r"\\?\C:\private\candidate"),
        identity: FileIdentity {
            volume: 1,
            high: 0,
            low: 2,
        },
        nt_path: r"\Device\HarddiskVolume1\private\candidate".to_owned(),
    };
    let receipt = Binding {
        target: target.clone(),
        mapped_root: PathBuf::from(r"D:\"),
        auth_low: owner.auth_low,
        auth_high: owner.auth_high,
        user: owner.user.clone(),
        session: owner.session,
    };
    receipt.validate(&target, &owner).unwrap();
    for changed in [
        Binding {
            auth_low: 22,
            ..receipt.clone()
        },
        Binding {
            auth_high: 1,
            ..receipt.clone()
        },
        Binding {
            session: 2,
            ..receipt.clone()
        },
        Binding {
            user: "S-1-5-18".to_owned(),
            ..receipt.clone()
        },
        Binding {
            mapped_root: PathBuf::from(r"Global\D:"),
            ..receipt.clone()
        },
        Binding {
            target: Target {
                identity: FileIdentity {
                    low: 3,
                    ..target.identity.clone()
                },
                ..target.clone()
            },
            ..receipt.clone()
        },
        Binding {
            target: Target {
                nt_path: r"\Device\HarddiskVolume1".to_owned(),
                ..target.clone()
            },
            ..receipt.clone()
        },
    ] {
        assert!(changed.validate(&target, &owner).is_err());
    }
}
