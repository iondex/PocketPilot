use super::*;
use lark_websocket_protobuf::pbbp2::Header;

fn frame(id: &str, sum: usize, seq: usize, bytes: Vec<u8>) -> Frame {
    Frame {
        headers: vec![
            Header {
                key: "message_id".into(),
                value: id.into(),
            },
            Header {
                key: "sum".into(),
                value: sum.to_string(),
            },
            Header {
                key: "seq".into(),
                value: seq.to_string(),
            },
        ],
        payload: Some(bytes),
        ..Default::default()
    }
}

#[test]
fn duplicates_do_not_grow_buffers_and_complete_events_keep_order() {
    let mut buffers = HashMap::new();
    assert!(assemble_frame(&mut buffers, frame("event", 2, 1, b"second".to_vec())).is_none());
    assert!(assemble_frame(&mut buffers, frame("event", 2, 1, b"second".to_vec())).is_none());
    assert_eq!(buffers["event"].bytes, 6);
    let complete = assemble_frame(&mut buffers, frame("event", 2, 0, b"first".to_vec())).unwrap();
    assert_eq!(complete.payload.unwrap(), b"firstsecond");
    assert!(buffers.is_empty());
}

#[test]
fn fragment_count_pending_count_and_expiration_are_bounded() {
    let mut buffers = HashMap::new();
    assert!(assemble_frame(&mut buffers, frame("bad", usize::MAX, 0, vec![])).is_none());
    assert!(buffers.is_empty());
    for index in 0..MAX_PENDING + 1 {
        assemble_frame(&mut buffers, frame(&index.to_string(), 2, 0, vec![1]));
    }
    assert_eq!(buffers.len(), MAX_PENDING);
    for buffer in buffers.values_mut() {
        buffer.created = Instant::now() - TTL;
    }
    assemble_frame(&mut buffers, frame("fresh", 2, 0, vec![1]));
    assert_eq!(buffers.len(), 1);
}

#[test]
fn byte_budgets_and_conflicting_fragments_cannot_expand_allocation() {
    let mut buffers = HashMap::new();
    assemble_frame(&mut buffers, frame("one", 3, 0, vec![1; MAX_MESSAGE]));
    assemble_frame(&mut buffers, frame("two", 3, 0, vec![1; MAX_MESSAGE]));
    assemble_frame(&mut buffers, frame("three", 2, 0, vec![1]));
    assert_eq!(buffers.len(), 2);
    assemble_frame(&mut buffers, frame("one", 3, 1, vec![1]));
    assert!(!buffers.contains_key("one"));
    assemble_frame(&mut buffers, frame("two", 3, 0, vec![2]));
    assert!(buffers.is_empty());
}
