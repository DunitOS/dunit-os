use dunit::{
    io::{BufReader, Read, Write},
    Error,
};

struct Input<'a> {
    bytes: &'a [u8],
    chunk: usize,
}
impl Read for Input<'_> {
    fn read(&mut self, output: &mut [u8]) -> dunit::Result<usize> {
        let count = output.len().min(self.bytes.len()).min(self.chunk);
        output[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        Ok(count)
    }
}
#[derive(Default)]
struct Output {
    bytes: Vec<u8>,
    chunk: usize,
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> dunit::Result<usize> {
        let count = self.chunk.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
}
#[test]
fn line_reader_keeps_batched_lines_blank_lines_and_eof_tail() {
    let mut input = BufReader::new(Input {
        bytes: b"first\n\nthird\ntail",
        chunk: usize::MAX,
    });
    for expected in ["first", "", "third", "tail"] {
        assert_eq!(input.read_line().unwrap().as_deref(), Some(expected));
    }
    assert_eq!(input.read_line().unwrap(), None);
    assert_eq!(input.read_line().unwrap(), None);
}
#[test]
fn unicode_and_long_lines_survive_every_chunk_boundary() {
    let text = "я😀".repeat(700) + "\nsecond\n";
    for chunk in [1, 2, 3, 5, 64, 1024, 4096] {
        let mut input = BufReader::new(Input {
            bytes: text.as_bytes(),
            chunk,
        });
        assert_eq!(input.read_line().unwrap().unwrap(), "я😀".repeat(700));
        assert_eq!(input.read_line().unwrap().as_deref(), Some("second"));
        assert_eq!(input.read_line().unwrap(), None);
    }
}
#[test]
fn invalid_utf8_reports_errno_and_keeps_following_line() {
    let mut input = BufReader::new(Input {
        bytes: b"\xff\nvalid\n",
        chunk: 100,
    });
    assert_eq!(input.read_line().unwrap_err().name(), "EINVAL");
    assert_eq!(input.read_line().unwrap().as_deref(), Some("valid"));
}
#[test]
fn read_after_line_uses_already_buffered_data() {
    let mut input = BufReader::new(Input {
        bytes: b"line\npayload",
        chunk: 100,
    });
    assert_eq!(input.read_line().unwrap().as_deref(), Some("line"));
    let mut output = [0; 7];
    input.read_exact(&mut output).unwrap();
    assert_eq!(&output, b"payload");
    assert_eq!(input.read(&mut []).unwrap(), 0);
}
#[test]
fn exact_reads_and_read_to_end_handle_short_transfers_and_eof() {
    let mut input = Input {
        bytes: b"abcdef",
        chunk: 2,
    };
    let mut exact = [0; 4];
    input.read_exact(&mut exact).unwrap();
    assert_eq!(&exact, b"abcd");
    let mut rest = vec![b'!'];
    assert_eq!(input.read_to_end(&mut rest).unwrap(), 2);
    assert_eq!(rest, b"!ef");
    assert_eq!(input.read_exact(&mut [0]).unwrap_err().name(), "EIO");
}
#[test]
fn formatted_writes_complete_partial_transfers_and_reject_zero_progress() {
    let mut output = Output {
        chunk: 2,
        ..Output::default()
    };
    output
        .write_fmt(format_args!("{} {}", i64::MIN, "я"))
        .unwrap();
    assert_eq!(output.bytes, format!("{} я", i64::MIN).as_bytes());
    output.chunk = 0;
    assert_eq!(output.write_all(b"x").unwrap_err().name(), "EIO");
    output.write_all(b"").unwrap();
}
#[test]
fn formatting_preserves_the_original_io_error() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> dunit::Result<usize> {
            Err(Error(libdunit::EPIPE))
        }
    }
    assert_eq!(
        Broken.write_fmt(format_args!("value {}", 1)),
        Err(Error(libdunit::EPIPE))
    );
}
