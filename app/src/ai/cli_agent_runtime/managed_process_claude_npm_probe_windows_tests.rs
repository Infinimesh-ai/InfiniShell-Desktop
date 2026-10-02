use super::*;

// 仅构造完整准入描述；真实路径身份与字节在 spawn 前另由 verify_expected_files 复核。
fn input(mode: &str, version: &str) -> ProbeInputs {
    let program = system_program(mode).unwrap();
    let prefix = PathBuf::from(r"C:\npm-probe-fixture");
    let stage = prefix
        .join("node_modules/@anthropic-ai")
        .join(format!(".infinishell-npm-{}", Uuid::new_v4()));
    let captured = ExpectedFileIdentity::capture(&program).unwrap();
    let mut files = vec![captured.clone()];
    for (path, size, digest) in contract::shims()
        .into_iter()
        .map(|(name, text)| {
            (
                prefix.join(name),
                text.len() as u64,
                format!("{:x}", Sha256::digest(text.as_bytes())),
            )
        })
        .chain(
            contract::files_for(version)
                .unwrap()
                .into_iter()
                .map(|(path, (size, digest, _))| (stage.join(path), size, digest)),
        )
    {
        let mut file = captured.clone();
        file.path = path.clone();
        file.canonical_path = path;
        file.size = size;
        file.sha256 = digest;
        files.push(file);
    }
    let mut arguments = vec![mode.into(), stage.into_os_string(), prefix.into_os_string()];
    if version != contract::VERSION {
        arguments.push(version.into());
    }
    ProbeInputs {
        program,
        arguments,
        files,
    }
}

#[test]
fn old_wire_remains_280_and_only_explicit_285_adds_a_new_candidate() {
    for mode in ["cmd", "powershell"] {
        let old = input(mode, contract::VERSION);
        validate(&old).unwrap();
        let original = serde_json::to_vec(&old).unwrap();
        let decoded: ProbeInputs = serde_json::from_slice(&original).unwrap();
        assert_eq!(decoded.arguments.len(), 3);
        assert_eq!(decoded.version(), contract::VERSION);
        validate(&decoded).unwrap();
        let current = input(mode, "2.1.285");
        validate(&current).unwrap();
        for rejected in ["2.1.280", "2.1.287", "2.1.288"] {
            let mut changed = current.clone();
            changed.arguments[3] = rejected.into();
            assert!(validate(&changed).is_err());
        }
        let mut missing = current;
        missing.arguments.pop();
        assert!(validate(&missing).is_err());
    }
}

#[test]
fn both_shell_modes_require_the_same_complete_target_version_tree() {
    for mode in ["cmd", "powershell"] {
        let current = input(mode, "2.1.285");
        validate(&current).unwrap();
        let mut mixed = current.clone();
        let other = contract::files_for("2.1.287").unwrap();
        let native = mixed.stage().join(contract::NATIVE);
        let file = mixed
            .files
            .iter_mut()
            .find(|file| file.path == native)
            .unwrap();
        (file.size, file.sha256) = (
            other[&PathBuf::from(contract::NATIVE)].0,
            other[&PathBuf::from(contract::NATIVE)].1.clone(),
        );
        assert!(validate(&mixed).is_err());
        let mut missing = current.clone();
        missing.files.pop();
        assert!(validate(&missing).is_err());
        let mut changed_shim = current;
        changed_shim.files[1].sha256 = "0".repeat(64);
        assert!(validate(&changed_shim).is_err());
    }
}
