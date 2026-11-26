#!/usr/bin/env python3
"""Compile and run bounded deterministic fuzz/malformed-corpus suites across every network parser."""
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]

FUZZER_RS = r"""
use std::collections::VecDeque;
use dunit_browser_net::{
    cookies::CookieJar,
    policy::Origin,
    wire::{ControlMessage, FetchResponsePayload},
};
use dunit_cert_store::TrustBundle;
use dunit_http_core::{
    Connector, Header, HttpError, Limits, Method, Request, Scheme, Session, Transport, Url,
};
use dunit_socket_abi::{
    decode_dns_records, BridgeHeader, BridgeHeader6, DnsRecord, Endpoint, Endpoint6, Endpoints,
    Endpoints6, Options, RecvHeader, RecvHeader6,
};
use dunit_tls_core::{
    x509::{parse_ecdsa_sig_der, Certificate},
    ByteStream, TlsClient, TlsError,
};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next() & 0xFF) as u8).collect()
    }
}

struct MaliciousByteStream(Vec<u8>, usize);
impl Transport for MaliciousByteStream {
    fn read(&mut self, out: &mut [u8]) -> Result<usize, HttpError> {
        if self.1 >= self.0.len() { return Ok(0); }
        let n = (self.0.len() - self.1).min(out.len()).min(13);
        out[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
        self.1 += n;
        Ok(n)
    }
    fn write_all(&mut self, _buf: &[u8]) -> Result<(), HttpError> { Ok(()) }
}

struct MaliciousConnector(Vec<u8>);
impl Connector for MaliciousConnector {
    type Conn = MaliciousByteStream;
    fn connect(&mut self, _scheme: Scheme, _host: &str, _port: u16) -> Result<Self::Conn, HttpError> {
        Ok(MaliciousByteStream(self.0.clone(), 0))
    }
}

struct MaliciousTlsStream(Vec<u8>, usize);
impl ByteStream for MaliciousTlsStream {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TlsError> {
        if self.1 >= self.0.len() { return Ok(0); }
        let n = (self.0.len() - self.1).min(buf.len()).min(17);
        buf[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
        self.1 += n;
        Ok(n)
    }
    fn write_all_bytes(&mut self, _buf: &[u8]) -> Result<(), TlsError> { Ok(()) }
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut v = seed.to_vec();
    match rng.next() % 5 {
        0 if !v.is_empty() => {
            let idx = (rng.next() as usize) % v.len();
            v[idx] ^= (rng.next() as u8).max(1);
        }
        1 if v.len() > 2 => {
            v.truncate(1 + ((rng.next() as usize) % (v.len() - 1)));
        }
        2 => {
            let extra_len = 1 + ((rng.next() as usize) % 64);
            let extra = rng.bytes(extra_len);
            v.extend_from_slice(&extra);
        }
        3 if v.len() >= 2 => {
            let idx = (rng.next() as usize) % v.len();
            v[idx] = if rng.next() % 2 == 0 { 0 } else { 0xFF };
        }
        _ => {
            let n = (rng.next() as usize) % 400;
            v = rng.bytes(n);
        }
    }
    v
}

fn main() {
    let mut rng = Rng(0xC0DE_5AFE_2026_1126);
    let url_seed = Url::parse("https://example.du.net.test:8443/path?q=1").unwrap();
    let http_seeds: &[&[u8]] = &[
        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ntest",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 10\r\n\r\n\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\x03",
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        b"HTTP/1.1 302 Found\r\nLocation: http://[::1]/x\r\nContent-Length: 0\r\n\r\n",
    ];
    let tls_seeds: &[&[u8]] = &[
        b"\x16\x03\x03\x00\x05\x02\x00\x00\x01\x00",
        b"\x15\x03\x03\x00\x02\x02\x28",
        b"\x17\x03\x03\xff\xff",
    ];
    let msg_seed = ControlMessage::empty(2, 99).encode().to_vec();
    let brsp_seed = FetchResponsePayload {
        status: 200,
        final_url: "https://du.net.test/".into(),
        content_type: "text/plain".into(),
        download_filename: None,
        tls_info: "TLS 1.3".into(),
        detail: String::new(),
        headers: vec![Header {
            name: "content-type".into(),
            value: "text/plain".into(),
        }],
        body: b"ok".to_vec(),
    }.encode();

    for iter in 0..2500usize {
        let mut inputs: VecDeque<Vec<u8>> = VecDeque::new();
        inputs.push_back(rng.bytes(iter % 320));
        inputs.push_back(mutate(http_seeds[iter % http_seeds.len()], &mut rng));
        inputs.push_back(mutate(tls_seeds[iter % tls_seeds.len()], &mut rng));
        inputs.push_back(mutate(&msg_seed, &mut rng));
        inputs.push_back(mutate(&brsp_seed, &mut rng));

        while let Some(sample) = inputs.pop_front() {
            let _ = Options::decode(&sample);
            let _ = Options::decode_version(&sample);
            let _ = Endpoint::decode(&sample);
            let _ = RecvHeader::decode(&sample);
            let _ = Endpoints::decode(&sample);
            let _ = BridgeHeader::decode(&sample);
            let _ = Endpoint6::decode(&sample);
            let _ = RecvHeader6::decode(&sample);
            let _ = Endpoints6::decode(&sample);
            let _ = BridgeHeader6::decode(&sample);
            let mut dns_out = [DnsRecord::default(); 16];
            let _ = decode_dns_records(&sample, &mut dns_out);

            if let Ok(s) = core::str::from_utf8(&sample) {
                if let Ok(u) = Url::parse(s) {
                    let _ = Origin::from_url(&u);
                }
                let mut jar = CookieJar::new(8, 32);
                let _ = jar.apply_response_headers(
                    &url_seed,
                    &[Header {
                        name: "set-cookie".into(),
                        value: s.into(),
                    }],
                    100,
                );
            }
            let _ = Certificate::parse(&sample);
            let _ = parse_ecdsa_sig_der(&sample);
            let _ = TrustBundle::decode_verified(&sample);
            let _ = ControlMessage::decode(&sample);
            let _ = FetchResponsePayload::decode(&sample);
        }

        let http_mal = mutate(http_seeds[iter % http_seeds.len()], &mut rng);
        let req = Request::new(Method::Get, "http://198.51.100.1/fuzz").unwrap();
        let mut sess = Session::new(MaliciousConnector(http_mal), Limits::default());
        let _ = sess.send(&req);

        let tls_mal = mutate(tls_seeds[iter % tls_seeds.len()], &mut rng);
        let _ = TlsClient::connect(
            MaliciousTlsStream(tls_mal, 0),
            "fuzz.du.net.test",
            &[],
            1000,
            [0x11; 32],
            [0x22; 32],
        );
    }
    println!("[FUZZ] abi_socket + http_core + tls_core + cert_store + browser_net: 2500 rounds OK");
}
"""


def build_rlib(rustc: str, tmp: Path, name: str, src: Path, deps: dict[str, Path]) -> Path:
    out = tmp / f"lib{name}.rlib"
    cmd = [
        rustc,
        "--edition=2021",
        "--crate-type=rlib",
        f"--crate-name={name}",
        "-O",
        "-Dwarnings",
        "-Coverflow-checks=yes",
        "-Cdebug-assertions=yes",
        "-L",
        f"dependency={tmp}",
        str(src),
        "-o",
        str(out),
    ]
    for dep_name, dep_path in deps.items():
        cmd += ["--extern", f"{dep_name}={dep_path}"]
    subprocess.run(cmd, check=True)
    return out


def main() -> None:
    rustc = os.environ.get("RUSTC", "rustc")
    subprocess.run(["python3", str(ROOT / "tools/test_net_core.py")], check=True)
    with tempfile.TemporaryDirectory(prefix="dunit-fuzz-") as td:
        tmp = Path(td)
        abi_socket = build_rlib(rustc, tmp, "dunit_socket_abi", ROOT / "abi/socket/src/lib.rs", {})
        http_core = build_rlib(rustc, tmp, "dunit_http_core", ROOT / "userspace/http-core/src/lib.rs", {})
        tls_core = build_rlib(rustc, tmp, "dunit_tls_core", ROOT / "userspace/tls-core/src/lib.rs", {})
        cert_store = build_rlib(
            rustc, tmp, "dunit_cert_store", ROOT / "userspace/cert-store/src/lib.rs",
            {"dunit_tls_core": tls_core},
        )
        browser_net = build_rlib(
            rustc, tmp, "dunit_browser_net", ROOT / "userspace/browser-net/src/lib.rs",
            {"dunit_http_core": http_core, "dunit_tls_core": tls_core},
        )
        harness = tmp / "fuzz_runner.rs"
        harness.write_text(FUZZER_RS)
        runner = tmp / "fuzz_runner"
        subprocess.run([
            rustc, "--edition=2021", "-O", "-Dwarnings", "-Coverflow-checks=yes",
            "-L", f"dependency={tmp}",
            str(harness),
            "--extern", f"dunit_socket_abi={abi_socket}",
            "--extern", f"dunit_http_core={http_core}",
            "--extern", f"dunit_tls_core={tls_core}",
            "--extern", f"dunit_cert_store={cert_store}",
            "--extern", f"dunit_browser_net={browser_net}",
            "-o", str(runner),
        ], check=True)
        subprocess.run([str(runner)], check=True, timeout=60)


if __name__ == "__main__":
    main()
