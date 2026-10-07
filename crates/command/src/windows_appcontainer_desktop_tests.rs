use super::*;
use windows::Win32::System::StationsAndDesktops::UOI_NAME;

const OWNER: &str = "S-1-5-21-111-222-333-1001";
const CONTAINER: &str = "S-1-15-2-1-2-3-4-5-6-7";

#[test]
fn expected_station_mismatch_still_precedes_caller_station_reuse() {
    let failure = verify_expected_station_name(
        "private_station",
        "PRIVATE_EXPECTED_STATION",
        "PRIVATE_STATION",
    )
    .unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        diagnostic_check(&failure),
        Some("new_logon_station_name_mismatch")
    );
    assert_eq!(diagnostic_caller_station_matched(&failure), Some(true));
    assert_eq!(diagnostic_code(&failure), None);
    assert_eq!(
        failure.to_string(),
        "新登录窗口站或 AppContainer SID 不匹配"
    );
    assert!(!format!("{failure:?}").contains("PRIVATE_"));
}

#[test]
fn expected_station_match_leaves_caller_safety_check_to_the_existing_next_branch() {
    verify_expected_station_name("STATION", "station", "STATION").unwrap();
}

#[test]
fn new_logon_station_selection_preserves_the_expected_automatic_station() {
    assert!(
        !needs_owned_station("PRIVATE_EXPECTED", "private_expected", "PRIVATE_CALLER").unwrap()
    );
}

#[test]
fn new_logon_station_selection_creates_only_for_the_observed_caller() {
    assert!(needs_owned_station("PRIVATE_CALLER", "PRIVATE_EXPECTED", "private_caller").unwrap());

    let failure =
        needs_owned_station("PRIVATE_OTHER", "PRIVATE_EXPECTED", "PRIVATE_CALLER").unwrap_err();
    assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        diagnostic_check(&failure),
        Some("new_logon_station_name_mismatch")
    );
    assert_eq!(diagnostic_caller_station_matched(&failure), Some(false));
    assert_eq!(diagnostic_code(&failure), None);
    assert!(!format!("{failure:?}").contains("PRIVATE_"));
}

#[test]
fn new_logon_station_selection_rejects_caller_as_the_expected_station() {
    let failure =
        needs_owned_station("PRIVATE_CALLER", "private_caller", "PRIVATE_CALLER").unwrap_err();

    assert_eq!(
        diagnostic_check(&failure),
        Some("new_logon_caller_station_reused")
    );
    assert_eq!(diagnostic_caller_station_matched(&failure), None);
}

#[test]
fn new_logon_created_station_requires_the_bound_name_without_reusing_caller() {
    verify_created_station_name("PRIVATE_NEW", "private_new", "PRIVATE_CALLER").unwrap();

    for (name, expected, caller, stage) in [
        (
            "PRIVATE_OTHER",
            "PRIVATE_NEW",
            "PRIVATE_CALLER",
            "new_logon_created_station_name_mismatch",
        ),
        (
            "PRIVATE_CALLER",
            "PRIVATE_NEW",
            "PRIVATE_CALLER",
            "new_logon_created_station_name_mismatch",
        ),
        (
            "PRIVATE_CALLER",
            "private_caller",
            "PRIVATE_CALLER",
            "new_logon_created_caller_station_reused",
        ),
    ] {
        let failure = verify_created_station_name(name, expected, caller).unwrap_err();
        assert_eq!(diagnostic_check(&failure), Some(stage));
        assert_eq!(diagnostic_caller_station_matched(&failure), None);
        assert_eq!(diagnostic_code(&failure), None);
        assert!(!format!("{failure:?}").contains("PRIVATE_"));
    }
}

#[test]
fn new_logon_owned_restore_accepts_only_the_retained_station_pair() {
    // 这些不透明标记只进入纯状态判断，不传给任何 Windows API。
    let original = HWINSTA(1usize as *mut c_void);
    let created = HWINSTA(2usize as *mut c_void);
    let unrelated = HWINSTA(3usize as *mut c_void);
    let station = NewLogonStation::Owned {
        original,
        handle: Some(created),
    };

    assert_eq!(station.restore_target(created).unwrap(), Some(original));
    assert_eq!(station.restore_target(original).unwrap(), None);
    let failure = station.restore_target(unrelated).unwrap_err();
    assert_eq!(
        diagnostic_check(&failure),
        Some("new_logon_owned_restore_station_changed")
    );
}

#[test]
fn new_logon_borrowed_station_never_requests_a_process_switch() {
    let original = HWINSTA(1usize as *mut c_void);
    let unrelated = HWINSTA(2usize as *mut c_void);
    let station = NewLogonStation::Borrowed(original);

    assert_eq!(station.restore_target(original).unwrap(), None);
    let failure = station.restore_target(unrelated).unwrap_err();
    assert_eq!(
        diagnostic_check(&failure),
        Some("new_logon_restore_station_changed")
    );
}

#[test]
fn new_logon_closed_owned_station_cannot_rebind_an_unretained_handle() {
    let original = HWINSTA(1usize as *mut c_void);
    let released = HWINSTA(2usize as *mut c_void);
    let station = NewLogonStation::Owned {
        original,
        handle: None,
    };

    assert_eq!(station.restore_target(original).unwrap(), None);
    assert_eq!(
        diagnostic_check(&station.restore_target(released).unwrap_err()),
        Some("new_logon_owned_restore_station_changed")
    );
    assert_eq!(
        diagnostic_check(&station.handle().unwrap_err()),
        Some("new_logon_owned_station_closed")
    );
}

fn station_name(station: HWINSTA) -> io::Result<Vec<u16>> {
    let mut name = [0u16; 128];
    let mut used = 0;
    unsafe {
        GetUserObjectInformationW(
            HANDLE(station.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            size_of_val(&name) as u32,
            Some(&mut used),
        )
    }
    .map_err(io::Error::other)?;
    let units = used as usize / size_of::<u16>();
    if used as usize != units * size_of::<u16>()
        || !(2..=name.len()).contains(&units)
        || name[units - 1] != 0
        || name[..units - 1].contains(&0)
    {
        return Err(invalid("无名窗口站名称读回无效"));
    }
    Ok(name[..units - 1].to_vec())
}

pub(crate) fn unnamed_station_create_only_reports_identity_and_cleanup() {
    let _creation = CREATION_LOCK.lock().unwrap();
    let original = unsafe { GetProcessWindowStation() }.unwrap();
    let owner = current_user_sid().unwrap();
    let descriptor = Descriptor::new(
        &owner,
        CONTAINER,
        STATION_OWNER_ACCESS,
        STATION_CONTAINER_ACCESS,
    )
    .unwrap();
    let attributes = descriptor.attributes();
    let created = unsafe {
        CreateWindowStationW(
            PCWSTR::null(),
            CWF_CREATE_ONLY,
            STATION_OWNER_ACCESS,
            Some(&attributes),
        )
    };
    match created {
        Err(error) => {
            eprintln!(
                "atomic_windows_unnamed_station_control={{\"created\":false,\"hresult\":{}}}",
                error.code().0 as u32
            );
            assert!(unsafe { GetProcessWindowStation() }.is_ok_and(|current| current == original));
        }
        Ok(station) => {
            let different_object = station_name(station)
                .and_then(|name| station_name(original).map(|original_name| name != original_name));
            let descriptor_matches = verify_object(HANDLE(station.0), &descriptor, true);
            let selected = unsafe { GetProcessWindowStation() };
            let original_unchanged = selected.as_ref().is_ok_and(|current| *current == original);
            let restore = if original_unchanged {
                Ok(())
            } else {
                unsafe { SetProcessWindowStation(original) }.map_err(io::Error::other)
            };
            let closed = restore
                .and_then(|()| unsafe { CloseWindowStation(station) }.map_err(io::Error::other));
            eprintln!(
                "atomic_windows_unnamed_station_control={{\"created\":true,\"different_object\":{},\"descriptor_matches\":{},\"original_station_unchanged\":{original_unchanged},\"original_station_restored\":{},\"closed\":{}}}",
                match different_object.as_ref() {
                    Ok(value) => value.to_string(),
                    Err(_) => "null".to_owned(),
                },
                descriptor_matches.is_ok(),
                unsafe { GetProcessWindowStation() }.is_ok_and(|current| current == original),
                closed.is_ok(),
            );
            assert!(different_object.unwrap());
            descriptor_matches.unwrap();
            closed.unwrap();
            assert!(unsafe { GetProcessWindowStation() }.is_ok_and(|current| current == original));
        }
    }
}

pub(crate) fn private_descriptor_binds_exact_owner_container_and_low_label() {
    let descriptor = Descriptor::new(
        OWNER,
        CONTAINER,
        STATION_OWNER_ACCESS,
        STATION_CONTAINER_ACCESS,
    )
    .unwrap();
    let (owner, dacl, label) = relative_parts(descriptor.bytes()).unwrap();
    assert!(!descriptor.attributes().bInheritHandle.as_bool());
    assert!(!owner.is_empty());
    assert_eq!(u16::from_le_bytes([dacl[4], dacl[5]]), 2);
    assert_eq!(u32::from_le_bytes(label[12..16].try_into().unwrap()), 1);
    assert_eq!(&label[16..], &[1, 1, 0, 0, 0, 0, 0, 16, 0, 16, 0, 0]);
    verify_descriptor(descriptor.bytes(), descriptor.bytes()).unwrap();
}

pub(crate) fn private_descriptor_rejects_broader_container_rights() {
    let expected = Descriptor::new(
        OWNER,
        CONTAINER,
        DESKTOP_OWNER_ACCESS,
        DESKTOP_CONTAINER_ACCESS,
    )
    .unwrap();
    let broader =
        Descriptor::new(OWNER, CONTAINER, DESKTOP_OWNER_ACCESS, DESKTOP_OWNER_ACCESS).unwrap();
    let failure = verify_descriptor(broader.bytes(), expected.bytes()).unwrap_err();
    assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
    assert_eq!(diagnostic_check(&failure), Some("descriptor_dacl_mismatch"));
    assert_eq!(diagnostic_code(&failure), None);
    assert_eq!(diagnostic_caller_station_matched(&failure), None);
    assert_eq!(
        failure.to_string(),
        "私有桌面 owner、DACL 或低完整性标签读回不匹配"
    );
}

pub(crate) fn private_descriptor_rejects_a_different_container() {
    let expected = Descriptor::new(
        OWNER,
        CONTAINER,
        STATION_OWNER_ACCESS,
        STATION_CONTAINER_ACCESS,
    )
    .unwrap();
    let other = Descriptor::new(
        OWNER,
        "S-1-15-2-1-2-3-4-5-6-8",
        STATION_OWNER_ACCESS,
        STATION_CONTAINER_ACCESS,
    )
    .unwrap();
    assert!(verify_descriptor(other.bytes(), expected.bytes()).is_err());
}

pub(crate) fn private_descriptor_rejects_medium_integrity_or_missing_protection() {
    let expected = Descriptor::new(
        OWNER,
        CONTAINER,
        STATION_OWNER_ACCESS,
        STATION_CONTAINER_ACCESS,
    )
    .unwrap();
    let mut changed = expected.bytes().to_vec();
    let header = unsafe {
        std::ptr::read_unaligned(changed.as_ptr().cast::<SECURITY_DESCRIPTOR_RELATIVE>())
    };
    let label_rid = header.Sacl as usize + 24;
    changed[label_rid..label_rid + 4].copy_from_slice(&0x2000u32.to_le_bytes());
    let failure = verify_descriptor(&changed, expected.bytes()).unwrap_err();
    assert_eq!(
        diagnostic_check(&failure),
        Some("descriptor_label_mismatch")
    );

    let mut changed = expected.bytes().to_vec();
    let control = u16::from_le_bytes([changed[2], changed[3]]) & !SE_DACL_PROTECTED.0;
    changed[2..4].copy_from_slice(&control.to_le_bytes());
    let failure = verify_descriptor(&changed, expected.bytes()).unwrap_err();
    assert_eq!(diagnostic_check(&failure), Some("descriptor_control"));
}

#[test]
fn descriptor_diagnostic_preserves_the_first_failed_comparison() {
    let expected = Descriptor::new(
        OWNER,
        CONTAINER,
        STATION_OWNER_ACCESS,
        STATION_CONTAINER_ACCESS,
    )
    .unwrap();
    let other = Descriptor::new(
        "S-1-5-21-111-222-333-1002",
        CONTAINER,
        STATION_OWNER_ACCESS,
        STATION_OWNER_ACCESS,
    )
    .unwrap();

    let failure = verify_descriptor(other.bytes(), expected.bytes()).unwrap_err();

    assert_eq!(failure.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        diagnostic_check(&failure),
        Some("descriptor_owner_mismatch")
    );
    assert_eq!(diagnostic_code(&failure), None);
}

pub(crate) fn private_descriptor_rejects_out_of_bounds_relative_offsets_and_ace_sizes() {
    let expected = Descriptor::new(
        OWNER,
        CONTAINER,
        STATION_OWNER_ACCESS,
        STATION_CONTAINER_ACCESS,
    )
    .unwrap();
    let mut changed = expected.bytes().to_vec();
    changed[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    let failure = verify_descriptor(&changed, expected.bytes()).unwrap_err();
    assert_eq!(diagnostic_check(&failure), Some("descriptor_offset_bounds"));

    let mut changed = expected.bytes().to_vec();
    let dacl = u32::from_le_bytes(changed[16..20].try_into().unwrap()) as usize;
    changed[dacl + 10..dacl + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    let failure = verify_descriptor(&changed, expected.bytes()).unwrap_err();
    assert_eq!(diagnostic_check(&failure), Some("ace_length_bounds"));
}

pub(crate) fn private_desktop_rejects_unbound_names_before_reading_sid_or_creating_objects() {
    let result = PrivateDesktop::create("WinSta0\\Default", PSID::default());
    assert_eq!(result.err().unwrap().to_string(), "私有桌面代次名称无效");
}

pub(crate) fn private_descriptor_rejects_label_bytes_without_a_present_sacl() {
    let expected = Descriptor::new(
        OWNER,
        CONTAINER,
        STATION_OWNER_ACCESS,
        STATION_CONTAINER_ACCESS,
    )
    .unwrap();
    let mut changed = expected.bytes().to_vec();
    let control = u16::from_le_bytes([changed[2], changed[3]]) & !SE_SACL_PRESENT.0;
    changed[2..4].copy_from_slice(&control.to_le_bytes());
    assert!(verify_descriptor(&changed, expected.bytes()).is_err());
}

pub(crate) fn private_desktop_never_publishes_a_route_for_missing_objects() {
    let mut desktop = PrivateDesktop {
        station: None,
        desktop: None,
        original: None,
        startup_name: wide("InfiniShell.Probe.fixture\\Probe".as_ref()).unwrap(),
        station_sd: Descriptor::new(
            OWNER,
            CONTAINER,
            STATION_OWNER_ACCESS,
            STATION_CONTAINER_ACCESS,
        )
        .unwrap(),
        desktop_sd: Descriptor::new(
            OWNER,
            CONTAINER,
            DESKTOP_OWNER_ACCESS,
            DESKTOP_CONTAINER_ACCESS,
        )
        .unwrap(),
    };
    let mut startup = STARTUPINFOW::default();

    assert!(desktop.configure_startup(&mut startup).is_err());
    assert!(startup.lpDesktop.is_null());
    desktop.close().unwrap();
    assert!(desktop.station.is_none() && desktop.desktop.is_none());
}

pub(crate) fn private_desktop_admin_observation_distinguishes_non_member_from_query_failure() {
    assert_eq!(
        administrator_membership_json(Ok(true)),
        r#"{"status":"ok","member":true,"hresult":null}"#
    );
    assert_eq!(
        administrator_membership_json(Ok(false)),
        r#"{"status":"ok","member":false,"hresult":null}"#
    );
    assert_eq!(
        administrator_membership_json(Err(2147942405)),
        r#"{"status":"query_error","member":null,"hresult":2147942405}"#
    );
}
