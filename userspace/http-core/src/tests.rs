use super::*;
use alloc::vec;

struct FakeConn {
    rx: Vec<u8>,
    pos: usize,
    chunk: usize,
    written: Vec<Vec<u8>>,
    fail_on_write: Option<HttpError>,
}

impl FakeConn {
    fn from_slices(chunks: &[&[u8]], read_chunk: usize) -> Self {
        let mut rx = Vec::new();
        for c in chunks {
            rx.extend_from_slice(c);
        }
        Self {
            rx,
            pos: 0,
            chunk: read_chunk.max(1),
            written: Vec::new(),
            fail_on_write: None,
        }
    }
}

impl Transport for FakeConn {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, HttpError> {
        if self.pos >= self.rx.len() {
            return Ok(0);
        }
        let n = (self.rx.len() - self.pos).min(buf.len()).min(self.chunk);
        buf[..n].copy_from_slice(&self.rx[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<(), HttpError> {
        if let Some(err) = self.fail_on_write.clone() {
            return Err(err);
        }
        self.written.push(buf.to_vec());
        Ok(())
    }
}

struct FakeConnector {
    queue: VecDeque<FakeConn>,
    dialed: Vec<(Scheme, String, u16)>,
}

impl FakeConnector {
    fn new(conns: Vec<FakeConn>) -> Self {
        Self {
            queue: conns.into(),
            dialed: Vec::new(),
        }
    }
}

impl Connector for FakeConnector {
    type Conn = FakeConn;
    fn connect(&mut self, scheme: Scheme, host: &str, port: u16) -> Result<Self::Conn, HttpError> {
        self.dialed.push((scheme, host.to_string(), port));
        self.queue.pop_front().ok_or(HttpError::Transport(-111))
    }
}

#[test]
fn url_parsing_and_location_resolution() {
    let url = Url::parse("HTTP://Example.COM:8080/a/b/index.html?x=1#frag").unwrap();
    assert_eq!(url.scheme, Scheme::Http);
    assert_eq!(url.host, "example.com");
    assert_eq!(url.port, 8080);
    assert_eq!(url.path, "/a/b/index.html?x=1");
    assert_eq!(url.host_header(), "example.com:8080");

    assert_eq!(
        url.resolve_location("../sub/item?y=2#h").unwrap().path,
        "/a/sub/item?y=2"
    );
    assert_eq!(url.resolve_location("/root").unwrap().path, "/root");
    assert_eq!(url.resolve_location("?q=new").unwrap().path, "/a/b/index.html?q=new");
    let https = url.resolve_location("https://secure.example/ok").unwrap();
    assert_eq!(https.scheme, Scheme::Https);
    assert_eq!(https.port, 443);
    assert_eq!(
        https.resolve_location("http://downgrade.example/"),
        Err(HttpError::UnsupportedScheme)
    );
    assert_eq!(
        Url::parse("ftp://files.example/"),
        Err(HttpError::UnsupportedScheme)
    );
    assert_eq!(
        Url::parse("http://user:pass@host/"),
        Err(HttpError::InvalidUrl)
    );
    assert_eq!(
        Url::parse("http://bad\r\nhost/"),
        Err(HttpError::InvalidUrl)
    );
}

#[test]
fn chunked_keepalive_reuse_and_small_reads() {
    let first = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nX-One: a\r\n\r\n5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\nX-Trailer: ignore\r\n\r\n";
    let second = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\npong";
    let conn = FakeConn::from_slices(&[first, second], 3);
    let mut sess = Session::with_defaults(FakeConnector::new(vec![conn]));

    let r1 = sess.send(&Request::new(Method::Get, "http://svc.local/1").unwrap()).unwrap();
    assert_eq!(r1.status, 200);
    assert_eq!(r1.body, b"hello world");
    assert_eq!(sess.idle_connections(), 1);

    let r2 = sess.send(&Request::new(Method::Get, "http://svc.local/2").unwrap()).unwrap();
    assert_eq!(r2.body, b"pong");
    assert_eq!(sess.stats(), SessionStats { opened: 1, reused: 1 });
}

#[test]
fn redirects_preserve_or_downgrade_methods_and_stop_loops() {
    let hop1 = b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 0\r\n\r\n";
    let hop2 = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
    let conn = FakeConn::from_slices(&[hop1, hop2], 64);
    let mut sess = Session::with_defaults(FakeConnector::new(vec![conn]));
    let req = Request::new(Method::Post, "http://api.local/start")
        .unwrap()
        .body(b"payload".to_vec());
    let resp = sess.send(&req).unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.url.path, "/next");
    assert_eq!(resp.body, b"ok");

    let loop_resp = b"HTTP/1.1 302 Found\r\nLocation: /loop\r\nConnection: close\r\n\r\n";
    let mut limits = Limits::default();
    limits.max_redirects = 2;
    let conns = (0..4).map(|_| FakeConn::from_slices(&[loop_resp], 128)).collect();
    let mut sess = Session::new(FakeConnector::new(conns), limits);
    assert_eq!(
        sess.send(&Request::new(Method::Get, "http://loop.local/loop").unwrap()),
        Err(HttpError::TooManyRedirects)
    );
}

#[test]
fn smuggling_and_bounds_are_rejected() {
    let dual = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n0\r\n\r\n";
    let mut s1 = Session::with_defaults(FakeConnector::new(vec![FakeConn::from_slices(&[dual], 64)]));
    assert_eq!(
        s1.send(&Request::new(Method::Get, "http://x.local/").unwrap()),
        Err(HttpError::InvalidResponse)
    );

    let conflict_cl = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Length: 5\r\n\r\nabcde";
    let mut s2 = Session::with_defaults(FakeConnector::new(vec![FakeConn::from_slices(&[conflict_cl], 64)]));
    assert_eq!(
        s2.send(&Request::new(Method::Get, "http://x.local/").unwrap()),
        Err(HttpError::InvalidResponse)
    );

    let huge = b"HTTP/1.1 200 OK\r\nContent-Length: 9999\r\n\r\nhi";
    let mut limits = Limits::default();
    limits.max_body_bytes = 1024;
    let mut s3 = Session::new(FakeConnector::new(vec![FakeConn::from_slices(&[huge], 64)]), limits);
    assert_eq!(
        s3.send(&Request::new(Method::Get, "http://x.local/").unwrap()),
        Err(HttpError::TooLarge)
    );

    let cut_chunk = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhi";
    let mut s4 = Session::with_defaults(FakeConnector::new(vec![FakeConn::from_slices(&[cut_chunk], 64)]));
    assert_eq!(
        s4.send(&Request::new(Method::Get, "http://x.local/").unwrap()),
        Err(HttpError::Truncated)
    );
}

#[test]
fn stale_keepalive_connection_retries_idempotent_get() {
    let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\na";
    // Connection 1 gives response 1, then EOF immediately on request 2; connection 2 succeeds.
    let c1 = FakeConn::from_slices(&[ok], 64);
    let c2 = FakeConn::from_slices(&[ok], 64);
    let mut sess = Session::with_defaults(FakeConnector::new(vec![c1, c2]));
    assert_eq!(sess.send(&Request::new(Method::Get, "http://h.local/a").unwrap()).unwrap().body, b"a");
    assert_eq!(sess.send(&Request::new(Method::Get, "http://h.local/b").unwrap()).unwrap().body, b"a");
    assert_eq!(sess.stats(), SessionStats { opened: 2, reused: 1 });
}

#[test]
fn head_and_204_have_empty_body_and_stay_reusable() {
    let head = b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\n\r\n";
    let next = b"HTTP/1.1 204 No Content\r\n\r\n";
    let third = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";
    let conn = FakeConn::from_slices(&[head, next, third], 16);
    let mut sess = Session::with_defaults(FakeConnector::new(vec![conn]));

    let r1 = sess.send(&Request::new(Method::Head, "http://h.local/file").unwrap()).unwrap();
    assert!(r1.body.is_empty());
    let r2 = sess.send(&Request::new(Method::Delete, "http://h.local/item").unwrap()).unwrap();
    assert_eq!(r2.status, 204);
    assert!(r2.body.is_empty());
    let r3 = sess.send(&Request::new(Method::Get, "http://h.local/final").unwrap()).unwrap();
    assert_eq!(r3.body, b"hi");
    assert_eq!(sess.stats(), SessionStats { opened: 1, reused: 2 });
}

#[test]
fn gzip_and_zlib_inflate_and_bomb_protection() {
    // Stored RFC1951 block inside gzip framing for "hello-http":
    let payload = b"hello-http";
    let mut gz = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0, 3];
    gz.push(0x01); // BFINAL=1, BTYPE=00 stored
    let len = payload.len() as u16;
    gz.extend_from_slice(&len.to_le_bytes());
    gz.extend_from_slice(&(!len).to_le_bytes());
    gz.extend_from_slice(payload);
    gz.extend_from_slice(&inflate::crc32(payload).to_le_bytes());
    gz.extend_from_slice(&(payload.len() as u32).to_le_bytes());

    let mut wire = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: ".to_vec();
    wire.extend_from_slice(format!("{}\r\n\r\n", gz.len()).as_bytes());
    wire.extend_from_slice(&gz);

    let mut sess = Session::with_defaults(FakeConnector::new(vec![FakeConn::from_slices(&[&wire], 11)]));
    let resp = sess.send(&Request::new(Method::Get, "http://gz.local/").unwrap()).unwrap();
    assert_eq!(resp.body, payload);

    // Same gzip payload exceeding tight decompressed cap must fail with TooLarge.
    let mut tight = Limits::default();
    tight.max_decompressed_bytes = 4;
    let mut sess_tight = Session::new(FakeConnector::new(vec![FakeConn::from_slices(&[&wire], 64)]), tight);
    assert_eq!(
        sess_tight.send(&Request::new(Method::Get, "http://gz.local/").unwrap()),
        Err(HttpError::TooLarge)
    );

    // RFC1950 zlib stream for "dunit" with stored block and valid Adler-32.
    let zdata = b"dunit";
    let mut zl = vec![0x78, 0x01, 0x01];
    let zlen = zdata.len() as u16;
    zl.extend_from_slice(&zlen.to_le_bytes());
    zl.extend_from_slice(&(!zlen).to_le_bytes());
    zl.extend_from_slice(zdata);
    zl.extend_from_slice(&inflate::adler32(zdata).to_be_bytes());
    assert_eq!(inflate::inflate_zlib(&zl, 64).unwrap(), zdata);

    // Pool evicts oldest idle connections beyond max_idle_connections=2.
    let mut lim = Limits::default();
    lim.max_idle_connections = 2;
    let resp_a = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\na";
    let mut s = Session::new(
        FakeConnector::new(vec![
            FakeConn::from_slices(&[resp_a], 64),
            FakeConn::from_slices(&[resp_a], 64),
            FakeConn::from_slices(&[resp_a], 64),
        ]),
        lim,
    );
    s.send(&Request::new(Method::Get, "http://a.local/").unwrap()).unwrap();
    s.send(&Request::new(Method::Get, "http://b.local/").unwrap()).unwrap();
    s.send(&Request::new(Method::Get, "http://c.local/").unwrap()).unwrap();
    assert_eq!(s.idle_connections(), 2);
}

#[test]
fn real_compressed_huffman_streams_and_corruption_reject() {
    let compressed: &[u8] = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0x4b, 0x29, 0xcd, 0xcb, 0x2c,
        0xd1, 0xcd, 0x2f, 0x56, 0xf0, 0x08, 0x09, 0x09, 0xd0, 0x37, 0xd4, 0x33, 0x54, 0xc8, 0x4e,
        0x4d, 0x2d, 0xd0, 0x4d, 0xcc, 0xc9, 0x2c, 0x4b, 0x55, 0x48, 0xce, 0xcf, 0x2d, 0x28, 0x4a,
        0x2d, 0x2e, 0x4e, 0x4d, 0x51, 0x54, 0x48, 0x19, 0x55, 0x88, 0x4f, 0x21, 0x00, 0x2c, 0xfe,
        0x96, 0x1d, 0x48, 0x01, 0x00, 0x00,
    ];
    let zcompressed: &[u8] = &[
        0x78, 0x9c, 0x4b, 0x29, 0xcd, 0xcb, 0x2c, 0xd1, 0xcd, 0x2f, 0x56, 0xf0, 0x08, 0x09, 0x09,
        0xd0, 0x37, 0xd4, 0x33, 0x54, 0xc8, 0x4e, 0x4d, 0x2d, 0xd0, 0x4d, 0xcc, 0xc9, 0x2c, 0x4b,
        0x55, 0x48, 0xce, 0xcf, 0x2d, 0x28, 0x4a, 0x2d, 0x2e, 0x4e, 0x4d, 0x51, 0x54, 0x48, 0x19,
        0x55, 0xa8, 0x8f, 0x27, 0x78, 0x00, 0x82, 0xcd, 0x6f, 0x59,
    ];
    let mut expected = Vec::new();
    for _ in 0..8 {
        expected.extend_from_slice(b"dunit-os HTTP/1.1 keep-alive compressed! ");
    }
    assert_eq!(inflate::inflate_gzip(compressed, 4096).unwrap(), expected);
    assert_eq!(inflate::inflate_zlib(zcompressed, 4096).unwrap(), expected);

    let mut bad_crc = compressed.to_vec();
    let last = bad_crc.len() - 5;
    bad_crc[last] ^= 0x01;
    assert_eq!(inflate::inflate_gzip(&bad_crc, 4096), Err(HttpError::Decompress));

    // Dynamic Huffman (BTYPE=2) zlib stream over 600 non-trivial bytes.
    let dyn_zl: &[u8] = &[
        0x78, 0x9c, 0x2d, 0xcf, 0x89, 0x52, 0x01, 0x00, 0x14, 0x05, 0x50, 0x4b, 0x28, 0x2a, 0x2a,
        0x2a, 0x0a, 0xd9, 0xd7, 0x50, 0x54, 0x54, 0x54, 0x14, 0xb2, 0x64, 0x0f, 0x85, 0xa2, 0xa2,
        0xa2, 0x50, 0xc8, 0x96, 0x7e, 0xbd, 0x79, 0x33, 0xf7, 0x0b, 0xce, 0x1c, 0x86, 0x2e, 0xd2,
        0x16, 0x7b, 0x2a, 0x6c, 0xd3, 0xcd, 0xf7, 0xa6, 0xb7, 0xc6, 0xb3, 0xa6, 0x7f, 0xb6, 0x2f,
        0xea, 0x02, 0x47, 0x6e, 0xb2, 0x73, 0xf5, 0x2e, 0x74, 0x15, 0x67, 0xda, 0xeb, 0xcf, 0x35,
        0x77, 0x99, 0x65, 0x88, 0x7f, 0x6d, 0x9c, 0x56, 0xb9, 0x96, 0xe4, 0x60, 0xeb, 0xfc, 0x85,
        0x6f, 0xcf, 0x8e, 0x94, 0x97, 0x6f, 0xcb, 0xce, 0xc2, 0xaf, 0x3a, 0xfc, 0xb1, 0x7a, 0x54,
        0x62, 0xea, 0xa3, 0xdd, 0xf5, 0x93, 0x47, 0x8e, 0x39, 0xd1, 0x93, 0x9d, 0x3d, 0x2f, 0xd8,
        0x32, 0x43, 0x79, 0xe0, 0x75, 0x69, 0x3f, 0x3f, 0x55, 0x05, 0x5b, 0x2b, 0x87, 0xf7, 0x0c,
        0x5d, 0xa4, 0x2d, 0x39, 0x7e, 0x98, 0x23, 0xc3, 0xf7, 0x34, 0xbf, 0x4b, 0x46, 0x63, 0x71,
        0xef, 0x96, 0x0c, 0xd1, 0xc1, 0xdd, 0x1f, 0x19, 0x9e, 0x0a, 0xdb, 0x48, 0x46, 0x8d, 0x67,
        0x4d, 0x91, 0x21, 0x70, 0xe4, 0xc6, 0x64, 0xb8, 0x8a, 0x33, 0x0d, 0x19, 0x65, 0x96, 0x21,
        0x46, 0x06, 0xd7, 0x92, 0xec, 0x93, 0x61, 0xcf, 0x8e, 0x14, 0x64, 0x14, 0x7e, 0xd5, 0x21,
        0x32, 0x98, 0xfa, 0x68, 0x87, 0x0c, 0x73, 0xa2, 0x27, 0x25, 0x23, 0x33, 0x94, 0xfb, 0xc9,
        0x98, 0xaa, 0x82, 0x4d, 0x32, 0xd0, 0x34, 0xa2, 0x99, 0x42, 0x73, 0x8c, 0xa6, 0x06, 0xcd,
        0x18, 0x9a, 0x7d, 0x34, 0x15, 0x68, 0x86, 0xd0, 0xec, 0xa0, 0x29, 0x45, 0xd3, 0x8f, 0x66,
        0x13, 0x4d, 0x31, 0x9a, 0x5e, 0x34, 0xeb, 0x68, 0x0a, 0xd1, 0x74, 0xa3, 0x59, 0x45, 0x93,
        0x8f, 0xa6, 0x13, 0xcd, 0x12, 0x9a, 0x1c, 0x34, 0x6d, 0x68, 0xe6, 0xd1, 0x64, 0xa0, 0x69,
        0x42, 0x33, 0x8d, 0xe6, 0x04, 0x4d, 0x2d, 0x9a, 0x71, 0x34, 0x07, 0x68, 0x2a, 0xd1, 0x0c,
        0xa3, 0xd9, 0x45, 0x53, 0x86, 0x66, 0x00, 0xcd, 0x16, 0x9a, 0x12, 0x34, 0x7d, 0x68, 0x36,
        0xd0, 0x14, 0xa1, 0xe9, 0x41, 0xb3, 0x86, 0xa6, 0x00, 0x4d, 0x17, 0x9a, 0xe5, 0x7f, 0x5d,
        0x12, 0x94, 0xfd,
    ];
    let dyn_expected: Vec<u8> = (0..600).map(|i| ((i * 37 + (i >> 3)) & 0x7f) as u8).collect();
    assert_eq!(inflate::inflate_zlib(dyn_zl, 2048).unwrap(), dyn_expected);
}
