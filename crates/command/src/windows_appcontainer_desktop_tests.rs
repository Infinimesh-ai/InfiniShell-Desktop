use super::*;
use windows::Win32::System::StationsAndDesktops::UOI_NAME;

const OWNER: &str = "S-1-5-21-111-222-333-1001";
const CONTAINER: &str = "S-1-15-2-1-2-3-4-5-6-7";

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
    assert!(verify_descriptor(broader.bytes(), expected.bytes()).is_err());
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
    assert!(verify_descriptor(&changed, expected.bytes()).is_err());

    let mut changed = expected.bytes().to_vec();
    let control = u16::from_le_bytes([changed[2], changed[3]]) & !SE_DACL_PROTECTED.0;
    changed[2..4].copy_from_slice(&control.to_le_bytes());
    assert!(verify_descriptor(&changed, expected.bytes()).is_err());
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
    assert!(verify_descriptor(&changed, expected.bytes()).is_err());

    let mut changed = expected.bytes().to_vec();
    let dacl = u32::from_le_bytes(changed[16..20].try_into().unwrap()) as usize;
    changed[dacl + 10..dacl + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    assert!(verify_descriptor(&changed, expected.bytes()).is_err());
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
