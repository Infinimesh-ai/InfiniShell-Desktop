#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::io::Write as _;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use flate2::{Compression, write::GzEncoder};

use super::*;

const CASK_0156: &[u8] = br###"cask "codex" do
  arch arm: "aarch64", intel: "x86_64"
  os macos: "apple-darwin", linux: "unknown-linux-musl"

  version "0.156.1"
  sha256 arm:          "fea42f9625091f011e38f059da974d52e57ba31831648bb1c7f0b1a385fde547",
         intel:        "618dbcd55419fa041871f777a14b107ceb3fe2d339ef81e21e6ab5374420dc71",
         arm64_linux:  "fdd47ed6aade0360796fd3f6f95a45096f327c15e19e8c7339f9dc5633041786",
         x86_64_linux: "8b711520beddf385467b8da4d2c93736637c6ba1e46811cf0d8606b7c490b6f6"

  url "https://github.com/openai/codex/releases/download/rust-v#{version}/codex-package-#{arch}-#{os}.tar.gz"
  name "Codex"
  desc "OpenAI's coding agent that runs in your terminal"
  homepage "https://github.com/openai/codex"

  livecheck do
    url :url
    regex(/^rust[._-]v?(\d+(?:\.\d+)+)$/i)
    strategy :github_latest
  end

  binary "bin/codex"
  generate_completions_from_executable "bin/codex", "completion"

  zap rmdir: "~/.codex"
end
"###;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const CASK_0160: &[u8] = br###"cask "codex" do
  arch arm: "aarch64", intel: "x86_64"
  os macos: "apple-darwin", linux: "unknown-linux-musl"

  version "0.160.0"
  sha256 arm:          "007df41b607dbbc8d204b9746ce7fed2d4ce6c813f44c32ceee54175ca796525",
         intel:        "4d50514b2d8acd81ca8cfee55b667b3dc4f7681b65bf3299ff06b11a064f8861",
         arm64_linux:  "7f0fe42ff22ecfa3a47bc4a34f5b22c4218b431a4ec0aba51c7d98299f07900c",
         x86_64_linux: "4fcc47ab57f52ff75363951a8761146cd10c8288bd86fed45487dbb204a16b71"

  url "https://github.com/openai/codex/releases/download/rust-v#{version}/codex-package-#{arch}-#{os}.tar.gz"
  name "Codex"
  desc "OpenAI's coding agent that runs in your terminal"
  homepage "https://github.com/openai/codex"

  livecheck do
    url :url
    regex(/^rust[._-]v?(\d+(?:\.\d+)+)$/i)
    strategy :github_latest
  end

  binary "bin/codex"
  generate_completions_from_executable "bin/codex", "completion"

  zap rmdir: "~/.codex"
end
"###;

#[test]
fn old_release_keeps_its_original_source_and_full_package() {
    let (metadata, url, _) = release("0.156.1", CASK_0156).unwrap();
    assert_eq!(metadata["version"], "0.156.1");
    assert_eq!(
        metadata["tap_git_head"],
        "8fb173ad115785e9dd050af916e14b3d3af26d65"
    );
    assert!(is_archive_url(&url));
    assert!(
        package("0.156.1")
            .unwrap()
            .files
            .contains_key(Path::new("codex-resources/voice/bin/codex-voice-host"))
    );
    assert!(release("0.156.1", b"cask with unreviewed install script").is_err());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_macos_release_binds_its_own_source_and_every_resource() {
    let (metadata, url, digest) = release("0.160.0", CASK_0160).unwrap();
    assert_eq!(
        metadata["tap_git_head"],
        "b5244981746375f8dee94b80c7f7738ff105256e"
    );
    assert_eq!(
        url,
        "https://github.com/openai/codex/releases/download/rust-v0.160.0/codex-package-aarch64-apple-darwin.tar.gz"
    );
    assert_eq!(
        digest,
        brew::decode_sha256("007df41b607dbbc8d204b9746ce7fed2d4ce6c813f44c32ceee54175ca796525")
            .unwrap()
    );
    let package = package("0.160.0").unwrap();
    assert_eq!(package.files.len(), 42);
    assert_eq!(package.directories.len(), 10);
    assert_eq!(
        package.files[Path::new("bin/codex")],
        (
            241555024,
            "112fae7a5a1223e673c8a1791d32338f37df8b527ff1159bb8adac6c4dbf1b4b".into(),
            0o755
        )
    );
    assert_eq!(
        package.files[Path::new("codex-package.json")].1,
        "404c4c935bf9b7a57fc02b6f0d09ba6b7bbbf15cf14286fc017de2a6c01cab47"
    );
    assert!(release("0.160.0", CASK_0156).is_err());
    assert!(release("0.156.1", CASK_0160).is_err());
    assert_eq!(supports_installed("0.160.0"), Ok(()));
}

#[cfg(target_os = "linux")]
#[test]
fn macos_current_release_does_not_expand_linux_contract() {
    assert_eq!(supports("0.160.0"), Err(Error::InvalidRelease));
    assert_eq!(supports_installed("0.160.0"), Err(Error::InvalidRelease));
    assert!(!is_archive_url(
        "https://github.com/openai/codex/releases/download/rust-v0.160.0/codex-package-x86_64-unknown-linux-musl.tar.gz"
    ));
}

#[test]
fn asset_redirect_permission_rejects_unknown_releases_and_url_suffixes() {
    assert!(!is_archive_url(
        "https://github.com/openai/codex/releases/download/rust-v0.999.0/codex-package-aarch64-apple-darwin.tar.gz"
    ));
    assert!(!is_archive_url(
        "https://github.com/openai/codex/releases/download/rust-v0.156.1/codex-package-aarch64-apple-darwin.tar.gz?redirect=another-package"
    ));
    assert!(!is_archive_url(
        "https://github.com/openai/codex/releases/download/rust-v0.156.1/codex-package-aarch64-apple-darwin.tar.gz.exe"
    ));
    assert_eq!(supports_installed("0.155.1"), Ok(()));
    assert_eq!(supports_installed("0.160.1"), Err(Error::InvalidRelease));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn current_archive_rejects_changed_member_mode_before_writing_it() {
    let temporary = tempfile::tempdir().unwrap();
    let stage = Directory::open(&temporary.path().canonicalize().unwrap()).unwrap();
    let mut input = tempfile::tempfile().unwrap();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = tar::Builder::new(&mut encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(200);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "codex-package.json", &[b'x'; 200][..])
            .unwrap();
        archive.finish().unwrap();
    }
    input.write_all(&encoder.finish().unwrap()).unwrap();
    // 稀疏长度满足外层长度检查，内部真实 tar 的错误 mode 仍必须拒绝。
    input.set_len(129976298).unwrap();
    assert_eq!(
        unpack(&mut input, &stage, "0.160.0"),
        Err(Error::InvalidRelease)
    );
    assert!(!temporary.path().join("0.160.0").exists());
}
