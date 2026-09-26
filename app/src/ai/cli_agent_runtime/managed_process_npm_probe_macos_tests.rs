use std::path::Path;

use super::expand;

#[test]
fn bare_macho_tokens_use_the_corresponding_image_directory() {
    let loader = Path::new("/opt/homebrew/Cellar/node/25.9.0_1/lib/libnode.141.dylib");
    let executable = Path::new("/opt/homebrew/Cellar/node/25.9.0_1/bin/node");

    assert_eq!(
        expand("@loader_path", loader, executable).unwrap(),
        Path::new("/opt/homebrew/Cellar/node/25.9.0_1/lib")
    );
    assert_eq!(
        expand("@executable_path", loader, executable).unwrap(),
        Path::new("/opt/homebrew/Cellar/node/25.9.0_1/bin")
    );
}

#[test]
fn macho_token_suffixes_keep_the_existing_relative_resolution() {
    let loader = Path::new("/opt/homebrew/Cellar/node/25.9.0_1/lib/libnode.141.dylib");
    let executable = Path::new("/opt/homebrew/Cellar/node/25.9.0_1/bin/node");

    assert_eq!(
        expand("@loader_path/../Frameworks", loader, executable).unwrap(),
        Path::new("/opt/homebrew/Cellar/node/25.9.0_1/lib/../Frameworks")
    );
    assert_eq!(
        expand("@executable_path/../lib", loader, executable).unwrap(),
        Path::new("/opt/homebrew/Cellar/node/25.9.0_1/bin/../lib")
    );
}

#[test]
fn similar_unknown_macho_tokens_remain_rejected() {
    let loader = Path::new("/opt/homebrew/Cellar/node/25.9.0_1/lib/libnode.141.dylib");
    let executable = Path::new("/opt/homebrew/Cellar/node/25.9.0_1/bin/node");

    for value in [
        "@loader_path_extra",
        "@loader_path_extra/lib",
        "@loader_paths",
        "@executable_path_extra",
        "@executable_path_extra/lib",
        "@executable_paths",
        "@rpath",
        "relative/lib",
    ] {
        assert!(expand(value, loader, executable).is_err(), "{value}");
    }
}
