use super::*;

#[test]
fn authorize_url_contains_required_params() {
    let pkce = PkceParams::generate();
    let url = authorize_url(&pkce);

    assert!(url.starts_with("https://auth.x.ai/oauth2/authorize?"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains(&format!("client_id={CLIENT_ID}")));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("scope=openid"));
    assert!(url.contains("plan=generic"));
    assert!(url.contains("referrer=warp"));
    // The redirect URI must be percent-encoded and match the registered value.
    assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A56121%2Fcallback"));
    // The CSRF state and PKCE challenge are echoed into the URL verbatim
    // (both are URL-safe base64, so no percent-encoding is applied).
    assert!(url.contains(&format!("state={}", pkce.state)));
    assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
}

#[test]
fn token_response_parses_minimal_and_full() {
    let minimal: TokenResponse =
        serde_json::from_str(r#"{"access_token":"abc"}"#).expect("minimal response should parse");
    assert_eq!(minimal.access_token, "abc");
    assert!(minimal.refresh_token.is_none());
    assert!(minimal.expires_in.is_none());

    // Unconsumed response fields (token_type, scope) are ignored by serde.
    let full: TokenResponse = serde_json::from_str(
        r#"{"access_token":"a","refresh_token":"r","token_type":"Bearer","expires_in":3600,"scope":"api:access"}"#,
    )
    .expect("full response should parse");
    assert_eq!(full.access_token, "a");
    assert_eq!(full.refresh_token.as_deref(), Some("r"));
    assert_eq!(full.expires_in, Some(3600));
}

#[test]
fn manual_code_exchange_captures_attempt_verifier() {
    let pkce = PkceParams::generate();
    let exchange = ManualCodeExchange {
        verifier: pkce.verifier.clone(),
    };
    assert_eq!(exchange.verifier, pkce.verifier);
}

#[test]
fn manual_code_exchange_rejects_blank_code() {
    let exchange = ManualCodeExchange {
        verifier: "verifier".to_string(),
    };
    let result = warpui_core::r#async::block_on(exchange.exchange("   "));
    assert!(result.is_err());
}

/// 取消测试需要立即重新绑定同一端口，因此使用低于各平台默认临时端口范围的端口池。
/// 不需要重绑定的回调测试使用系统分配的临时端口，避免抢占取消测试刚释放的端口。
const TEST_PORT_BASE: u16 = 21121;
const TEST_PORT_SPAN: u16 = 200;

/// 为取消测试建立与生产代码相同的非阻塞监听器。
fn bind_cancellation_test_listener() -> (TcpListener, std::net::SocketAddr) {
    // PID 偏移只分散不同测试进程的起始端口，不保证它们不会相撞。
    let offset = (std::process::id() % u32::from(TEST_PORT_SPAN)) as u16;
    for candidate in 0..TEST_PORT_SPAN {
        let port = TEST_PORT_BASE + (offset + candidate) % TEST_PORT_SPAN;
        let Ok(listener) = TcpListener::bind((REDIRECT_HOST, port)) else {
            continue;
        };
        listener
            .set_nonblocking(true)
            .expect("test callback listener should be non-blocking");
        let address = listener
            .local_addr()
            .expect("test callback listener should have an address");
        return (listener, address);
    }
    panic!(
        "no free loopback port in {TEST_PORT_BASE}..{} for the test callback listener",
        TEST_PORT_BASE + TEST_PORT_SPAN
    );
}

/// `run_oauth_flow` must not hand back a result while the callback thread still
/// owns the loopback socket: the caller may immediately rebind the redirect
/// port, and that bind fails with "address in use" against a still-open
/// listener.
///
/// The rebind is asserted over many cancel cycles — with no sleeps or retries —
/// so a reintroduced teardown race is caught here instead of showing up as an
/// intermittent CI failure.
#[test]
fn cancelling_loopback_wait_releases_listener() {
    const CANCEL_CYCLES: usize = 100;

    for cycle in 0..CANCEL_CYCLES {
        let (listener, address) = bind_cancellation_test_listener();
        let cancellation = OauthCancellationHandle {
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        cancellation.cancel();

        let result = warpui_core::r#async::block_on(run_oauth_flow(
            listener,
            PkceParams::generate(),
            cancellation,
        ));

        assert_eq!(
            result
                .expect_err("cancelled callback wait should fail")
                .to_string(),
            "Grok authorization was cancelled"
        );
        TcpListener::bind(address).unwrap_or_else(|error| {
            panic!("取消第 {cycle} 轮后无法重新绑定回调端口 {address}: {error}")
        });
    }
}

/// Guards the non-cancelled path: the callback captured by the listener thread
/// is still delivered to the flow after the listener is closed. A mismatched
/// CSRF state stops the flow before the network token exchange, so the CSRF
/// error is proof the callback made the trip.
#[test]
fn loopback_callback_is_delivered_after_the_listener_closes() {
    let listener = TcpListener::bind((REDIRECT_HOST, 0))
        .expect("test callback listener should bind an ephemeral port");
    listener
        .set_nonblocking(true)
        .expect("test callback listener should be non-blocking");
    let address = listener
        .local_addr()
        .expect("test callback listener should have an address");
    let cancellation = OauthCancellationHandle {
        cancelled: Arc::new(AtomicBool::new(false)),
    };

    // Stand in for the browser hitting the redirect URI.
    let browser = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("test browser should connect");
        stream
            .write_all(
                b"GET /callback?code=test-code&state=unexpected-state HTTP/1.1\r\n\
                  Host: 127.0.0.1\r\n\r\n",
            )
            .expect("test browser should send the callback request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("test browser should read the callback response");
    });

    let result = warpui_core::r#async::block_on(run_oauth_flow(
        listener,
        PkceParams::generate(),
        cancellation,
    ));
    browser.join().expect("test browser thread should finish");

    assert!(
        result
            .expect_err("a mismatched callback state should fail")
            .to_string()
            .contains("state did not match")
    );
}
