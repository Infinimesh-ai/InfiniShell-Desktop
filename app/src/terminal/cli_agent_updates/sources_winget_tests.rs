use super::*;

#[test]
fn stable_target_has_only_the_reviewed_current_source_edges() {
    if cfg!(target_arch = "x86_64") {
        supports(CLIAgent::Claude, "2.1.285").unwrap();
        for installed in ["2.1.285", "2.1.286"] {
            supports_transition(CLIAgent::Claude, installed, "2.1.285").unwrap();
        }
    } else {
        assert!(supports(CLIAgent::Claude, "2.1.285").is_err());
        assert!(supports_transition(CLIAgent::Claude, "2.1.286", "2.1.285").is_err());
    }
    for (installed, target) in [
        ("2.1.280", "2.1.285"),
        ("2.1.285", "2.1.286"),
        ("2.1.286", "2.1.287"),
        ("2.1.287", "2.1.285"),
    ] {
        assert!(supports_transition(CLIAgent::Claude, installed, target).is_err());
    }
    if cfg!(any(target_arch = "x86_64", target_arch = "aarch64")) {
        supports(CLIAgent::Claude, "2.1.280").unwrap();
    }
}
