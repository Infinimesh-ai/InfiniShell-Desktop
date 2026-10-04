use super::*;

fn mac_read_and_deny() -> MacAcl {
    MacAcl {
        flags: MAC_NO_INHERIT,
        entries: Entries(vec![
            MacAce {
                uuid: [0x11; 16],
                tag: 2,
                flags: 0x10,
                rights: 0x1004,
            },
            MacAce {
                uuid: [0x22; 16],
                tag: 1,
                flags: 0x160,
                rights: 0x100a8a,
            },
        ]),
    }
}

// 固定 UAPI 字节：owner rwx、uid 1001 r--、group r-x、mask r--、other ---。
const POSIX_READ: &[u8] = &[
    2, 0, 0, 0, 1, 0, 7, 0, 255, 255, 255, 255, 2, 0, 4, 0, 233, 3, 0, 0, 4, 0, 5, 0, 255, 255,
    255, 255, 16, 0, 4, 0, 255, 255, 255, 255, 32, 0, 0, 0, 255, 255, 255, 255,
];

#[test]
fn mac_roundtrip_preserves_deny_order_uuid_and_all_supported_flags() {
    let original = mac_read_and_deny();
    let decoded = decode_mac(&encode_mac(&original).unwrap()).unwrap();
    assert_eq!(decoded, original);
    let mut reversed = original.clone();
    reversed.entries.0.reverse();
    assert_ne!(
        encode_mac(&original).unwrap(),
        encode_mac(&reversed).unwrap()
    );
}

#[test]
fn mac_unknown_permissions_are_rejected_even_on_deny() {
    let mut value = mac_read_and_deny();
    value.entries.0[0].rights |= 1 << 31;
    assert_eq!(encode_mac(&value).unwrap_err().operation, "mac_ace");
}

#[test]
fn mac_allow_write_is_rejected_even_when_inherit_only() {
    let mut value = mac_read_and_deny();
    value.entries.0[1].rights |= 1 << 2;
    assert_eq!(encode_mac(&value).unwrap_err().operation, "mac_allow_write");
}

#[test]
fn mac_unknown_flags_are_not_silently_discarded() {
    let mut bytes = encode_mac(&mac_read_and_deny()).unwrap();
    bytes[40..44].copy_from_slice(&1_u32.to_be_bytes());
    assert_eq!(decode_mac(&bytes).unwrap_err().operation, "mac_acl_flags");
    let mut bytes = encode_mac(&mac_read_and_deny()).unwrap();
    bytes[60..64].copy_from_slice(&0x402_u32.to_be_bytes());
    assert_eq!(decode_mac(&bytes).unwrap_err().operation, "mac_ace");
}

#[test]
fn mac_truncated_or_excessive_external_count_is_rejected_before_import() {
    let bytes = encode_mac(&mac_read_and_deny()).unwrap();
    assert_eq!(
        decode_mac(&bytes[..bytes.len() - 1]).unwrap_err().kind,
        FailureKind::Invalid
    );
    let mut bytes = bytes;
    bytes[36..40].copy_from_slice(&129_u32.to_be_bytes());
    assert_eq!(decode_mac(&bytes).unwrap_err().kind, FailureKind::TooLarge);
}

#[test]
fn mac_absent_and_present_empty_acl_have_distinct_journal_descriptions() {
    let absent = Acl::MacV1 { extended: None };
    let empty = Acl::MacV1 {
        extended: Some(MacAcl {
            flags: 0,
            entries: Entries(vec![]),
        }),
    };
    assert_ne!(
        serde_json::to_vec(&absent).unwrap(),
        serde_json::to_vec(&empty).unwrap()
    );
    assert_eq!(
        decode_mac(
            &encode_mac(&MacAcl {
                flags: 0,
                entries: Entries(vec![])
            })
            .unwrap()
        )
        .unwrap()
        .entries
        .0
        .len(),
        0
    );
}

#[test]
fn linux_fixed_xattr_preserves_mask_instead_of_recalculating_it() {
    let parsed = decode_posix(POSIX_READ).unwrap();
    assert_eq!(parsed.0[2].permissions, 5);
    assert_eq!(parsed.0[3].permissions, 4);
    assert_eq!(encode_posix(&parsed).unwrap(), POSIX_READ);
    let acl = Acl::LinuxV1 {
        access: Some(parsed),
        default: None,
    };
    assert!(acl.validate(false, 0o100740).is_ok());
    assert_eq!(
        acl.validate(false, 0o100750).unwrap_err().operation,
        "access_mode"
    );
}

#[test]
fn linux_masked_raw_write_is_rejected() {
    let mut bytes = POSIX_READ.to_vec();
    bytes[14] = 6;
    assert_eq!(
        decode_posix(&bytes).unwrap_err().operation,
        "posix_non_owner_write"
    );
}

#[test]
fn linux_duplicate_named_principal_is_rejected() {
    let mut value = decode_posix(POSIX_READ).unwrap();
    value.0.insert(2, value.0[1].clone());
    assert_eq!(
        encode_posix(&value).unwrap_err().operation,
        "posix_order_or_duplicate"
    );
}

#[test]
fn linux_named_principal_requires_explicit_mask() {
    let mut value = decode_posix(POSIX_READ).unwrap();
    value.0.remove(3);
    assert_eq!(
        encode_posix(&value).unwrap_err().operation,
        "posix_required_entries"
    );
}

#[test]
fn linux_default_acl_is_distinct_and_cannot_be_attached_to_file() {
    let value = Acl::LinuxV1 {
        access: None,
        default: Some(decode_posix(POSIX_READ).unwrap()),
    };
    assert!(value.validate(true, 0o40700).is_ok());
    assert_eq!(
        value.validate(false, 0o100600).unwrap_err().operation,
        "default_on_file"
    );
}

#[test]
fn linux_unknown_version_truncation_and_permission_bits_are_rejected() {
    let mut bytes = POSIX_READ.to_vec();
    bytes[0] = 3;
    assert_eq!(
        decode_posix(&bytes).unwrap_err().operation,
        "posix_xattr_format"
    );
    assert_eq!(
        decode_posix(&POSIX_READ[..43]).unwrap_err().kind,
        FailureKind::Invalid
    );
    let mut bytes = POSIX_READ.to_vec();
    bytes[6] = 8;
    assert_eq!(decode_posix(&bytes).unwrap_err().operation, "posix_entry");
}

#[test]
fn journal_acl_deserialization_is_bounded_before_native_application() {
    let value = MacAcl {
        flags: 0,
        entries: Entries(vec![
            MacAce {
                uuid: [1; 16],
                tag: 2,
                flags: 0,
                rights: 2,
            };
            129
        ]),
    };
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(serde_json::from_slice::<MacAcl>(&bytes).is_err());
}

#[test]
fn journal_unknown_fields_are_not_accepted_as_future_acl_semantics() {
    let value = br#"{"format":"LinuxV1","access":null,"default":null,"inherited_write":true}"#;
    assert!(serde_json::from_slice::<Acl>(value).is_err());
}

#[test]
fn absent_conversion_preserves_explicit_empty_mac_acl() {
    assert!(Acl::absent().into_optional().is_none());
    let empty = Acl::MacV1 {
        extended: Some(MacAcl {
            flags: 0,
            entries: Entries(vec![]),
        }),
    };
    assert!(empty.clone().into_optional().is_some());
    assert_eq!(
        empty.inherit(false, 0o644).unwrap(),
        (Acl::MacV1 { extended: None }, 0o644)
    );
}

#[test]
fn linux_inheritance_masks_access_and_preserves_directory_default() {
    let parent = Acl::LinuxV1 {
        access: None,
        default: Some(decode_posix(POSIX_READ).unwrap()),
    };
    let (file, mode) = parent.inherit(false, 0o640).unwrap();
    assert_eq!(mode, 0o640);
    assert_eq!(
        file,
        Acl::LinuxV1 {
            access: Some(Entries(vec![
                PosixEntry {
                    tag: 1,
                    permissions: 6,
                    id: UNDEFINED_ID
                },
                PosixEntry {
                    tag: 2,
                    permissions: 4,
                    id: 1001
                },
                PosixEntry {
                    tag: 4,
                    permissions: 5,
                    id: UNDEFINED_ID
                },
                PosixEntry {
                    tag: 16,
                    permissions: 4,
                    id: UNDEFINED_ID
                },
                PosixEntry {
                    tag: 32,
                    permissions: 0,
                    id: UNDEFINED_ID
                },
            ])),
            default: None
        }
    );
    let (directory, mode) = parent.inherit(true, 0o750).unwrap();
    assert_eq!(mode, 0o740);
    assert_eq!(
        directory,
        Acl::LinuxV1 {
            access: Some(decode_posix(POSIX_READ).unwrap()),
            default: Some(decode_posix(POSIX_READ).unwrap())
        }
    );
}

#[test]
fn linux_simple_default_becomes_mode_without_access_xattr() {
    let default = Entries(vec![
        PosixEntry {
            tag: 1,
            permissions: 7,
            id: UNDEFINED_ID,
        },
        PosixEntry {
            tag: 4,
            permissions: 5,
            id: UNDEFINED_ID,
        },
        PosixEntry {
            tag: 32,
            permissions: 0,
            id: UNDEFINED_ID,
        },
    ]);
    let parent = Acl::LinuxV1 {
        access: None,
        default: Some(default.clone()),
    };
    assert_eq!(
        parent.inherit(false, 0o644).unwrap(),
        (
            Acl::LinuxV1 {
                access: None,
                default: None
            },
            0o640
        )
    );
    assert_eq!(
        parent.inherit(true, 0o755).unwrap(),
        (
            Acl::LinuxV1 {
                access: None,
                default: Some(default)
            },
            0o750
        )
    );
}

#[cfg(target_os = "macos")]
mod mac_native {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    fn acl(entries: Vec<MacAce>) -> Acl {
        Acl::MacV1 {
            extended: Some(MacAcl {
                flags: 0,
                entries: Entries(entries),
            }),
        }
    }

    fn create_file(path: &std::path::Path) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap()
    }

    #[test]
    fn mac_fd_roundtrip_preserves_owner_other_read_allow_and_deny_order() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("candidate");
        let file = create_file(&path);
        let owner = uid_uuid_for_test(unsafe { libc::geteuid() });
        let other = uid_uuid_for_test(0);
        assert_ne!(owner, other);
        let desired = acl(vec![
            MacAce {
                uuid: owner,
                tag: 2,
                flags: 0,
                rights: 1 << 2,
            },
            MacAce {
                uuid: owner,
                tag: 1,
                flags: 0,
                rights: MAC_READ_RIGHTS,
            },
            MacAce {
                uuid: other,
                tag: 1,
                flags: 0,
                rights: MAC_READ_RIGHTS,
            },
        ]);
        let before = capture(&file).unwrap();
        assert!(before.acl.is_absent());
        let applied = apply_to_new(&file, &before, &desired).unwrap();
        assert_eq!(applied.acl, desired);
        verify(&file, &applied).unwrap();
        let error = OpenOptions::new().write(true).open(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let removed = apply_to_new(&file, &applied, &Acl::absent()).unwrap();
        assert!(removed.acl.is_absent());
        drop(file);
        root.close().unwrap();
    }

    #[test]
    fn mac_fd_empty_acl_normalization_is_not_reported_as_exact_copy() {
        let root = tempfile::tempdir().unwrap();
        let file = create_file(&root.path().join("empty"));
        let before = capture(&file).unwrap();
        let empty = acl(vec![]);
        // portable ACL 表示区分空与缺失；本机文件系统规范化后不能报告精确复制成功。
        let error = apply_to_new(&file, &before, &empty).unwrap_err();
        assert_eq!(error.operation, "acl_readback");
        assert_eq!(error.kind, FailureKind::ReadbackMismatch);
        assert!(error.write_attempted);
        let actual = capture(&file).unwrap();
        assert_eq!(actual.acl, Acl::absent());
        assert_ne!(actual.acl, empty);
        assert!(before.identity.unchanged_except_ctime(&actual.identity));
        drop(file);
        root.close().unwrap();
    }

    #[test]
    fn mac_fd_rejects_unknown_write_and_hardlink_before_mutation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("candidate");
        let file = create_file(&path);
        let before = capture(&file).unwrap();
        let owner = uid_uuid_for_test(unsafe { libc::geteuid() });
        let unknown = acl(vec![MacAce {
            uuid: owner,
            tag: 2,
            flags: 0,
            rights: 1 << 31,
        }]);
        assert!(
            !apply_to_new(&file, &before, &unknown)
                .unwrap_err()
                .write_attempted
        );
        let write = acl(vec![MacAce {
            uuid: owner,
            tag: 1,
            flags: 0,
            rights: 1 << 2,
        }]);
        assert!(
            !apply_to_new(&file, &before, &write)
                .unwrap_err()
                .write_attempted
        );
        verify(&file, &before).unwrap();
        fs::hard_link(&path, root.path().join("alias")).unwrap();
        let linked = capture(&file).unwrap();
        let read = acl(vec![MacAce {
            uuid: owner,
            tag: 1,
            flags: 0,
            rights: 1 << 1,
        }]);
        let error = apply_to_new(&file, &linked, &read).unwrap_err();
        assert_eq!(error.operation, "apply_hardlink");
        assert!(!error.write_attempted);
        verify(&file, &linked).unwrap();
        drop(file);
        root.close().unwrap();
    }

    #[test]
    fn mac_inheritance_plan_matches_kernel_files_directories_and_limit() {
        let root = tempfile::tempdir().unwrap();
        let parent = File::open(root.path()).unwrap();
        let owner = uid_uuid_for_test(unsafe { libc::geteuid() });
        // 分别覆盖文件专用、目录专用、只继承和单代继承；deny 不剥夺删除或改 ACL。
        let desired = Acl::MacV1 {
            extended: Some(MacAcl {
                flags: MAC_NO_INHERIT,
                entries: Entries(vec![
                    MacAce {
                        uuid: owner,
                        tag: 2,
                        flags: 0x120,
                        rights: 1 << 2,
                    },
                    MacAce {
                        uuid: owner,
                        tag: 1,
                        flags: 0x40,
                        rights: MAC_READ_RIGHTS,
                    },
                    MacAce {
                        uuid: owner,
                        tag: 1,
                        flags: 0x160,
                        rights: MAC_READ_RIGHTS,
                    },
                    MacAce {
                        uuid: owner,
                        tag: 1,
                        flags: 0xe0,
                        rights: MAC_READ_RIGHTS,
                    },
                ]),
            }),
        };
        apply_to_new(&parent, &capture(&parent).unwrap(), &desired).unwrap();
        let child = create_file(&root.path().join("child"));
        assert_eq!(
            capture(&child).unwrap().acl,
            desired.inherit(false, 0o600).unwrap().0
        );
        let directory = root.path().join("directory");
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let directory_fd = File::open(&directory).unwrap();
        let expected = desired.inherit(true, 0o700).unwrap().0;
        assert_eq!(capture(&directory_fd).unwrap().acl, expected);
        let grandchild = create_file(&directory.join("grandchild"));
        assert_eq!(
            capture(&grandchild).unwrap().acl,
            expected.inherit(false, 0o600).unwrap().0
        );
        drop(grandchild);
        drop(directory_fd);
        drop(child);
        drop(parent);
        root.close().unwrap();
    }
}

#[cfg(target_os = "linux")]
mod linux_native {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

    #[test]
    fn linux_fd_roundtrip_default_inheritance_and_removal_match_kernel() {
        let root = tempfile::tempdir().unwrap();
        let parent = File::open(root.path()).unwrap();
        let desired = Acl::LinuxV1 {
            access: None,
            default: Some(decode_posix(POSIX_READ).unwrap()),
        };
        let applied = apply_to_new(&parent, &capture(&parent).unwrap(), &desired).unwrap();
        assert_eq!(applied.acl, desired);
        let child = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o640)
            .open(root.path().join("child"))
            .unwrap();
        let (expected, mode) = desired.inherit(false, 0o640).unwrap();
        assert_eq!(capture(&child).unwrap().acl, expected);
        assert_eq!(child.metadata().unwrap().mode() & 0o777, mode);
        let directory = root.path().join("directory");
        fs::DirBuilder::new()
            .mode(0o750)
            .create(&directory)
            .unwrap();
        let directory_fd = File::open(&directory).unwrap();
        let (expected, mode) = desired.inherit(true, 0o750).unwrap();
        assert_eq!(capture(&directory_fd).unwrap().acl, expected);
        assert_eq!(directory_fd.metadata().unwrap().mode() & 0o777, mode);
        // 新节点复制与删除均通过原 fd；父目录的内容变化后重新取得身份。
        let copy = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.path().join("copy"))
            .unwrap();
        copy.set_permissions(fs::Permissions::from_mode(0o640))
            .unwrap();
        let desired_copy = capture(&child).unwrap().acl;
        assert_eq!(
            apply_to_new(&copy, &capture(&copy).unwrap(), &desired_copy)
                .unwrap()
                .acl,
            desired_copy
        );
        assert!(
            apply_to_new(&parent, &capture(&parent).unwrap(), &Acl::absent())
                .unwrap()
                .acl
                .is_absent()
        );
        drop(copy);
        drop(directory_fd);
        drop(child);
        drop(parent);
        root.close().unwrap();
    }
}
