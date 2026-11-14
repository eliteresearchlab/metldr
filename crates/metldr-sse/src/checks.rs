use super::Sse;

fn feed(parser: &mut Sse, byte: u8) -> Result<Option<(String, String)>, String> {
    parser.next(&mut &[byte][..])
}

#[test]
fn framing_preserves_events_and_rejects_invalid_or_incomplete_input() {
    let mut parser = Sse::new("Messages");
    let wire = "\u{feff}: keepalive\r\nevent: answer\r\ndata: h\u{e9}\r\ndata: world\r\n\r\nevent: ignored\n\ndata:\n\n";
    let events: Vec<_> = wire
        .bytes()
        .filter_map(|byte| feed(&mut parser, byte).transpose())
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        events,
        [
            ("answer".into(), "h\u{e9}\nworld".into()),
            ("".into(), "".into())
        ]
    );
    parser.finish().unwrap();

    let mut parser = Sse::with_bound("Responses", 8);
    for byte in b"data: 12\ndata: 34\n\n" {
        feed(&mut parser, *byte).unwrap();
    }
    parser.finish().unwrap();

    let mut parser = Sse::with_bound("Chat", 4);
    for byte in b"data" {
        feed(&mut parser, *byte).unwrap();
    }
    assert_eq!(
        feed(&mut parser, b':').unwrap_err(),
        "Chat SSE line exceeds bound"
    );

    let mut parser = Sse::with_bound("Google", 8);
    for byte in b"data: 12\ndata: 34\ndata: 56" {
        feed(&mut parser, *byte).unwrap();
    }
    assert_eq!(
        feed(&mut parser, b'\n').unwrap_err(),
        "Google SSE event exceeds bound"
    );

    let mut parser = Sse::new("Messages");
    feed(&mut parser, 0xff).unwrap();
    assert_eq!(
        feed(&mut parser, b'\n').unwrap_err(),
        "Messages SSE is not UTF8"
    );

    for wire in ["data: pending", "data: pending\n", "event: pending\n"] {
        let mut parser = Sse::new("Responses");
        for byte in wire.bytes() {
            feed(&mut parser, byte).unwrap();
        }
        assert_eq!(
            parser.finish().unwrap_err(),
            "Responses SSE ended within an event"
        );
    }
}

#[test]
fn every_chunk_size_yields_the_same_events() {
    let wire = "\u{feff}event: a\r\ndata: one\r\r\ndata: two\n\ndata: three\r\n\r\n".as_bytes();
    for size in 1..=wire.len() {
        let mut parser = Sse::new("Messages");
        let mut events = Vec::new();
        for chunk in wire.chunks(size) {
            let mut rest = chunk;
            while let Some(event) = parser.next(&mut rest).unwrap() {
                events.push(event);
            }
        }
        parser.finish().unwrap();
        let expected = [
            ("a".into(), "one".into()),
            ("".into(), "two".into()),
            ("".into(), "three".into()),
        ];
        assert_eq!(events, expected, "chunk size {size}");
    }
}
