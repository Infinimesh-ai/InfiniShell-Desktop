use futures::executor::block_on;
use futures::io::BufReader;

use super::*;

#[test]
fn receive_rejects_oversized_header_before_reading_payload() {
    let header = 13_usize.to_be_bytes();
    let mut reader = BufReader::new(header.as_slice());

    let error = block_on(receive_message::<Vec<u8>, _>(&mut reader, Some(12))).unwrap_err();

    assert!(matches!(
        error,
        ProtocolError::FrameTooLarge {
            frame_bytes: 13,
            max_frame_bytes: 12,
        }
    ));
    assert!(reader.get_ref().is_empty());
}

#[test]
fn send_accepts_a_frame_at_the_selected_limit() {
    let mut wire = Vec::new();

    block_on(send_message(&mut wire, vec![1_u8; 4], Some(12))).unwrap();

    assert_eq!(wire.len(), USIZE_SIZE + 12);
}

#[test]
fn send_rejects_an_oversized_frame_without_writing_its_header() {
    let mut wire = Vec::new();

    let error = block_on(send_message(&mut wire, vec![1_u8; 5], Some(12))).unwrap_err();

    assert!(matches!(
        error,
        ProtocolError::FrameTooLarge {
            frame_bytes: 13,
            max_frame_bytes: 12,
        }
    ));
    assert!(wire.is_empty());
}

#[test]
fn send_without_a_limit_preserves_unbounded_framing() {
    let mut wire = Vec::new();

    block_on(send_message(&mut wire, vec![1_u8; 5], None)).unwrap();

    assert_eq!(wire.len(), USIZE_SIZE + 13);
}
