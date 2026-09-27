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
