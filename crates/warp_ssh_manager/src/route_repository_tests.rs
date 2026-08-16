use super::{SshRepository, SshRepositoryError, setup_in_memory};
use crate::types::SshRouteHop;

fn hops() -> Vec<SshRouteHop> {
    vec![
        SshRouteHop {
            node_id: None,
            target_alias: "bastion".to_string(),
            port: None,
        },
        SshRouteHop {
            node_id: None,
            target_alias: "staging".to_string(),
            port: Some(2222),
        },
    ]
}

#[test]
fn route_round_trip_stores_structure_without_credentials() {
    let mut connection = setup_in_memory();
    let route = SshRepository::create_route(&mut connection, "生产环境", None, &hops()).unwrap();
    assert_eq!(route.name, "生产环境");
    assert_eq!(route.hops, hops());
    assert!(route.last_connected_at.is_none());

    SshRepository::mark_route_connected(&mut connection, &route.id).unwrap();
    let loaded = SshRepository::get_route(&mut connection, &route.id)
        .unwrap()
        .unwrap();
    assert!(loaded.last_connected_at.is_some());
    assert_eq!(loaded.hops[1].target_alias, "staging");
    assert_eq!(loaded.hops[1].port, Some(2222));
}

#[test]
fn route_rejects_empty_and_over_depth_paths() {
    let mut connection = setup_in_memory();
    let error = SshRepository::create_route(&mut connection, "empty", None, &[]).unwrap_err();
    assert!(matches!(error, SshRepositoryError::InvalidRoute(_)));

    let over_depth = (0..9)
        .map(|index| SshRouteHop {
            node_id: None,
            target_alias: format!("host-{index}"),
            port: None,
        })
        .collect::<Vec<_>>();
    let error =
        SshRepository::create_route(&mut connection, "too deep", None, &over_depth).unwrap_err();
    assert!(matches!(error, SshRepositoryError::InvalidRoute(_)));
}

#[test]
fn route_rejects_aliases_that_could_inject_options_or_commands() {
    let mut connection = setup_in_memory();
    for target_alias in ["-oProxyCommand=evil", "host name", "host\nwhoami"] {
        let invalid_hop = SshRouteHop {
            node_id: None,
            target_alias: target_alias.to_string(),
            port: None,
        };
        let error = SshRepository::create_route(&mut connection, "invalid", None, &[invalid_hop])
            .unwrap_err();
        assert!(matches!(error, SshRepositoryError::InvalidRoute(_)));
    }

    let zero_port = SshRouteHop {
        node_id: None,
        target_alias: "host".to_string(),
        port: Some(0),
    };
    let error =
        SshRepository::create_route(&mut connection, "invalid", None, &[zero_port]).unwrap_err();
    assert!(matches!(error, SshRepositoryError::InvalidRoute(_)));
}
