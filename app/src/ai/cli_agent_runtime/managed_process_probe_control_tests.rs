use super::*;
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, TcpListener};
use std::sync::mpsc;

fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (control, _) = listener.accept().unwrap();
    (control, peer)
}

fn wait_for_reader(listener: &mut CancellationListener) {
    let reader = listener.reader.take().unwrap();
    let (finished, completion) = mpsc::channel();
    thread::spawn(move || {
        let result = reader.join();
        let _ = finished.send(result);
    });
    completion
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert!(listener.token().load(Ordering::Acquire));
}

#[test]
fn idle_connection_outlives_its_handshake_timeout() {
    let (control, mut peer) = connected_pair();
    control
        .set_read_timeout(Some(Duration::from_millis(1)))
        .unwrap();
    let mut listener = CancellationListener::start(control).unwrap();
    let (clock_sender, clock) = mpsc::channel::<()>();

    // 独立计时跨过旧握手期限及多个轮询片，不让控制 socket 经历接收超时。
    assert_eq!(
        clock.recv_timeout(Duration::from_millis(350)),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    drop(clock_sender);
    assert!(!listener.token().load(Ordering::Acquire));
    request_stop(&mut peer).unwrap();
    wait_for_reader(&mut listener);
}

#[test]
fn stop_request_writes_only_the_stop_byte_before_eof() {
    let (mut control, mut peer) = connected_pair();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();

    request_stop(&mut control).unwrap();

    let mut request = [0_u8; 1];
    peer.read_exact(&mut request).unwrap();
    assert_eq!(request, [2]);
    assert_eq!(peer.read(&mut request).unwrap(), 0);
}

#[test]
fn stop_request_cancels_the_bound_probe() {
    let (control, mut peer) = connected_pair();
    let mut listener = CancellationListener::start(control).unwrap();

    request_stop(&mut peer).unwrap();

    wait_for_reader(&mut listener);
}

#[test]
fn control_eof_cancels_the_bound_probe() {
    let (control, peer) = connected_pair();
    let mut listener = CancellationListener::start(control).unwrap();

    peer.shutdown(Shutdown::Write).unwrap();

    wait_for_reader(&mut listener);
}

#[test]
fn unexpected_control_byte_cancels_without_waiting_for_eof() {
    let (control, mut peer) = connected_pair();
    let mut listener = CancellationListener::start(control).unwrap();

    peer.write_all(&[9]).unwrap();

    wait_for_reader(&mut listener);
}

#[test]
fn dropping_listener_wakes_and_joins_its_reader() {
    let (control, peer) = connected_pair();
    let listener = CancellationListener::start(control).unwrap();
    let token = listener.token();
    let (finished, completion) = mpsc::channel();

    thread::spawn(move || {
        drop(listener);
        let _ = finished.send(());
    });

    completion.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(token.load(Ordering::Acquire));
    drop(peer);
}

#[test]
fn job_authorization_rejects_reordered_repeated_and_unsolicited_acknowledgments() {
    let mut state = AuthorizationState::default();
    assert!(state.begin(1).is_err());
    assert!(state.acknowledge(JOB_ACK_FIRST).is_err());
    state.begin(0).unwrap();
    assert!(state.begin(0).is_err());
    assert!(state.acknowledge(JOB_ACK_SECOND).is_err());
    state.acknowledge(JOB_ACK_FIRST).unwrap();
    assert!(state.acknowledge(JOB_ACK_FIRST).is_err());
    assert_eq!(state.completed, 0);
}

fn read_job_request(peer: &mut TcpStream) -> Vec<u8> {
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut header = [0u8; 4];
    peer.read_exact(&mut header).unwrap();
    let count = u32::from_le_bytes(header) as usize;
    assert!(count <= MAX_JOB_REQUEST);
    let mut bytes = vec![0; count];
    peer.read_exact(&mut bytes).unwrap();
    bytes
}

#[test]
fn job_authorization_completes_only_the_two_bound_stages() {
    let (control, peer) = connected_pair();
    let listener = Arc::new(CancellationListener::start(control).unwrap());
    let requester = Arc::clone(&listener);
    let (done, completed) = mpsc::channel();
    let caller = thread::spawn(move || {
        let result = requester
            .authorize_payload(0, b"bootstrap")
            .and_then(|()| requester.authorize_payload(1, b"candidate"));
        done.send(result).unwrap();
    });
    let mut supervisor = SupervisorControl::new(peer).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut received = Vec::new();
    while supervisor.acknowledged != 2 || supervisor.pending_ack.is_some() {
        assert!(Instant::now() < deadline);
        supervisor
            .poll(|bytes| {
                received.push(bytes.to_vec());
                Ok(())
            })
            .unwrap();
        thread::yield_now();
    }
    completed
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    caller.join().unwrap();
    assert_eq!(received, [b"bootstrap".to_vec(), b"candidate".to_vec()]);
    assert!(!listener.token().load(Ordering::Acquire));
    assert!(listener.authorize_payload(1, b"candidate").is_err());
    assert!(listener.token().load(Ordering::Acquire));
}

#[test]
fn stop_wakes_a_pending_job_authorization_without_an_ack() {
    let (control, mut peer) = connected_pair();
    let listener = Arc::new(CancellationListener::start(control).unwrap());
    let requester = Arc::clone(&listener);
    let (done, completed) = mpsc::channel();
    let caller = thread::spawn(move || {
        done.send(requester.authorize_payload(0, b"bootstrap"))
            .unwrap()
    });
    assert_eq!(read_job_request(&mut peer), b"bootstrap");
    request_stop(&mut peer).unwrap();
    assert_eq!(
        completed
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    caller.join().unwrap();
}

#[test]
fn eof_cannot_authorize_a_pending_process() {
    let (control, mut peer) = connected_pair();
    let listener = Arc::new(CancellationListener::start(control).unwrap());
    let requester = Arc::clone(&listener);
    let (done, completed) = mpsc::channel();
    let caller = thread::spawn(move || {
        done.send(requester.authorize_payload(0, b"bootstrap"))
            .unwrap()
    });
    assert_eq!(read_job_request(&mut peer), b"bootstrap");
    peer.shutdown(Shutdown::Write).unwrap();
    assert_eq!(
        completed
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
    caller.join().unwrap();
}

#[test]
fn a_stop_in_the_same_reply_batch_takes_precedence_over_ack() {
    let (control, mut peer) = connected_pair();
    let mut listener = CancellationListener::start(control).unwrap();
    listener.authorization.0.lock().unwrap().begin(0).unwrap();
    peer.write_all(&[JOB_ACK_FIRST, 2]).unwrap();
    wait_for_reader(&mut listener);
    assert_eq!(
        listener.check_cancelled().unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(listener.authorization.0.lock().unwrap().completed, 0);
}

#[test]
fn supervisor_rejects_oversized_requests_before_authorizing() {
    let (control, mut peer) = connected_pair();
    let mut supervisor = SupervisorControl::new(control).unwrap();
    peer.write_all(&4097u32.to_le_bytes()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(Instant::now() < deadline);
        if let Err(error) = supervisor.poll(|_| panic!("超长请求不能进入 Job 授权")) {
            assert_eq!(error.to_string(), "managed_process.probe_job_request_size");
            break;
        }
        thread::yield_now();
    }
}

#[test]
fn supervisor_rejects_partial_request_eof_without_authorization() {
    let (control, mut peer) = connected_pair();
    let mut supervisor = SupervisorControl::new(control).unwrap();
    peer.write_all(&[2, 0, 0, 0, 7]).unwrap();
    peer.shutdown(Shutdown::Write).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        assert!(Instant::now() < deadline);
        if let Err(error) = supervisor.poll(|_| panic!("残缺请求不能进入 Job 授权")) {
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
            break;
        }
        thread::yield_now();
    }
}

#[test]
fn supervisor_stop_discards_a_pending_ack() {
    let (control, mut peer) = connected_pair();
    let mut supervisor = SupervisorControl::new(control).unwrap();
    supervisor.pending_ack = Some(JOB_ACK_FIRST);
    supervisor.request_stop().unwrap();
    supervisor.poll(|_| panic!("停止后不能继续授权")).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, [2]);
}
