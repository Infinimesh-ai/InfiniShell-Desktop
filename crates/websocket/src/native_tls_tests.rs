use std::sync::Arc;
use std::time::Duration;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::ServerCertVerifier;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, Error, RootCertStore};

// 固定在全部夹具的有效期内，避免系统时间掩盖证书链的实际验证结果。
const VERIFICATION_TIME: Duration = Duration::from_secs(1_793_016_000);

fn bundled_verifier(root_pem: &[u8]) -> Arc<WebPkiServerVerifier> {
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(root_pem).unwrap())
        .unwrap();
    WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(aws_lc_rs::default_provider()),
    )
    .build()
    .unwrap()
}

#[test]
fn bundled_tls_rejects_distrusted_emsign_intermediate_with_original_root_trusted() {
    // 显式信任原根证书，以验证锁定依赖中的公钥拒绝规则，而不是依赖系统根证书删除。
    let verifier = bundled_verifier(include_bytes!("testdata/tls/emsign-ecc-root-g3.pem"));
    let chain =
        CertificateDer::pem_slice_iter(include_bytes!("testdata/tls/emsign-g3b-noct-chain.pem"))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
    let server_name = ServerName::try_from("dv-g3b-noct-emudhra.we-have-a-problem.com").unwrap();

    let error = verifier
        .verify_server_cert(
            &chain[0],
            &chain[1..],
            &server_name,
            &[],
            UnixTime::since_unix_epoch(VERIFICATION_TIME),
        )
        .unwrap_err();

    // rustls 0.23.39 将新版 WebPKI 的拒绝原因保存在 Other 中；过期、缺根和签名错误不能通过。
    let Error::InvalidCertificate(CertificateError::Other(cause)) = error else {
        panic!("unexpected certificate failure: {error:?}");
    };
    assert_eq!(cause.0.to_string(), "CertDistrusted");
}

#[test]
fn bundled_tls_accepts_valid_certificate_chain() {
    let verifier = bundled_verifier(include_bytes!("testdata/tls/rustls-p256-root.pem"));
    let leaf = CertificateDer::from_pem_slice(include_bytes!("testdata/tls/rustls-p256-leaf.pem"))
        .unwrap();
    let intermediate =
        CertificateDer::from_pem_slice(include_bytes!("testdata/tls/rustls-p256-intermediate.pem"))
            .unwrap();
    let server_name = ServerName::try_from("testserver.com").unwrap();

    let result = verifier.verify_server_cert(
        &leaf,
        &[intermediate],
        &server_name,
        &[],
        UnixTime::since_unix_epoch(VERIFICATION_TIME),
    );

    assert!(
        result.is_ok(),
        "valid certificate chain rejected: {result:?}"
    );
}
