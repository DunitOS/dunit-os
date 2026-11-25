#!/usr/bin/env python3
"""Compile/test userspace TLS 1.3 + X.509 crypto crate and verify interop against OpenSSL."""
import os
from pathlib import Path
import socket
import ssl
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]

PROBE_RS = r"""
use std::io::{Read, Write};
use std::net::TcpStream;
use dunit_tls_core::{ByteStream, Certificate, TlsClient, TlsError};

struct StdStream(TcpStream);
impl ByteStream for StdStream {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TlsError> {
        self.0.read(buf).map_err(|e| TlsError::Transport(e.raw_os_error().unwrap_or(5) as isize))
    }
    fn write_all_bytes(&mut self, buf: &[u8]) -> Result<(), TlsError> {
        self.0.write_all(buf).map_err(|e| TlsError::Transport(e.raw_os_error().unwrap_or(5) as isize))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let addr = &args[1];
    let host = &args[2];
    let ca_der = std::fs::read(&args[3]).unwrap();
    let now: u64 = args[4].parse().unwrap();
    let entropy: [u8; 32] = [0x42; 32];
    let privk: [u8; 32] = [0x17; 32];
    let roots = vec![Certificate::parse(&ca_der).unwrap()];
    let tcp = TcpStream::connect(addr).unwrap();
    let mut tls = match TlsClient::connect(StdStream(tcp), host, &roots, now, entropy, privk) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERR:{e}");
            std::process::exit(2);
        }
    };
    tls.write_all(b"ping-openssl").unwrap();
    let mut reply = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let n = tls.read(&mut buf).unwrap();
        if n == 0 { break; }
        reply.extend_from_slice(&buf[..n]);
    }
    assert_eq!(reply, b"pong-tls13");
    let _ = tls.close_notify();
    println!("INTEROP_OK");
}
"""


def run_openssl_interop(work: Path, compiler: str, tls_lib: Path) -> None:
    probe_src = work / 'probe.rs'
    probe_bin = work / 'probe'
    probe_src.write_text(PROBE_RS)
    subprocess.run([
        compiler, '--edition=2021', '-O', '-Dwarnings',
        '--extern', f'dunit_tls_core={tls_lib}',
        str(probe_src), '-o', str(probe_bin),
    ], check=True)

    ca_key = work / 'ca.key'
    ca_pem = work / 'ca.pem'
    ca_der = work / 'ca.der'
    subprocess.run(['openssl', 'ecparam', '-name', 'prime256v1', '-genkey', '-noout', '-out', str(ca_key)], check=True)
    subprocess.run([
        'openssl', 'req', '-x509', '-new', '-key', str(ca_key), '-sha256', '-days', '3650',
        '-subj', '/CN=Dunit Root CA',
        '-addext', 'basicConstraints=critical,CA:TRUE',
        '-addext', 'keyUsage=critical,keyCertSign,cRLSign',
        '-out', str(ca_pem),
    ], check=True)
    subprocess.run(['openssl', 'x509', '-in', str(ca_pem), '-outform', 'DER', '-out', str(ca_der)], check=True)

    for kind in ('rsa2048', 'ecdsa_p256'):
        srv_key = work / f'{kind}.key'
        srv_csr = work / f'{kind}.csr'
        srv_pem = work / f'{kind}.pem'
        ext = work / f'{kind}.ext'
        ext.write_text('subjectAltName=DNS:https.du.net.test,IP:192.0.2.2\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n')
        if kind == 'rsa2048':
            subprocess.run(['openssl', 'genrsa', '-out', str(srv_key), '2048'], check=True, stderr=subprocess.DEVNULL)
        else:
            subprocess.run(['openssl', 'ecparam', '-name', 'prime256v1', '-genkey', '-noout', '-out', str(srv_key)], check=True)
        subprocess.run(['openssl', 'req', '-new', '-key', str(srv_key), '-subj', '/CN=https.du.net.test', '-out', str(srv_csr)], check=True)
        subprocess.run([
            'openssl', 'x509', '-req', '-in', str(srv_csr), '-CA', str(ca_pem), '-CAkey', str(ca_key),
            '-CAcreateserial', '-out', str(srv_pem), '-days', '365', '-sha256', '-extfile', str(ext),
        ], check=True, stderr=subprocess.DEVNULL)

        for _ in range(1):
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            ctx.minimum_version = ssl.TLSVersion.TLSv1_3
            ctx.maximum_version = ssl.TLSVersion.TLSv1_3
            ctx.load_cert_chain(str(srv_pem), str(srv_key))

            srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            srv.bind(('127.0.0.1', 0))
            srv.listen(2)
            port = srv.getsockname()[1]

            def serve_one():
                try:
                    conn, _ = srv.accept()
                    with ctx.wrap_socket(conn, server_side=True) as tls:
                        data = tls.recv(1024)
                        assert data == b'ping-openssl', data
                        tls.sendall(b'pong-tls13')
                        tls.unwrap()
                except Exception as exc:
                    print(f'server err ({kind}): {exc}')

            t = threading.Thread(target=serve_one, daemon=True)
            t.start()
            now = str(int(time.time()))
            res = subprocess.run([str(probe_bin), f'127.0.0.1:{port}', 'https.du.net.test', str(ca_der), now],
                                 capture_output=True, text=True, timeout=15)
            t.join(timeout=5)
            srv.close()
            assert res.returncode == 0 and 'INTEROP_OK' in res.stdout, (kind, res.returncode, res.stdout, res.stderr)

    # Check hostname mismatch & expired rejections cleanly.
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_3
    ctx.load_cert_chain(str(work / 'ecdsa_p256.pem'), str(work / 'ecdsa_p256.key'))
    for bad_host, bad_now, want_err in (
        ('wrong.example.test', str(int(time.time())), 'invalid X.509 certificate'),
        ('https.du.net.test', '2500000000', 'certificate expired'),
    ):
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.bind(('127.0.0.1', 0))
        srv.listen(1)
        port = srv.getsockname()[1]

        def serve_reject():
            try:
                conn, _ = srv.accept()
                with ctx.wrap_socket(conn, server_side=True):
                    pass
            except Exception:
                pass

        t = threading.Thread(target=serve_reject, daemon=True)
        t.start()
        res = subprocess.run([str(probe_bin), f'127.0.0.1:{port}', bad_host, str(ca_der), bad_now],
                             capture_output=True, text=True, timeout=15)
        assert res.returncode == 2 and want_err in res.stderr, (res.returncode, res.stderr)
        t.join(timeout=5)
        srv.close()
    print('OpenSSL TLS 1.3 interop (ECDSA P-256 + RSA-2048-PSS + ChaCha20 + AES-128-GCM + rejections): OK')


if __name__ == '__main__':
    with tempfile.TemporaryDirectory(prefix='dunit-tls-core-') as folder:
        work = Path(folder)
        compiler = os.environ.get('RUSTC', 'rustc')
        source = str(ROOT / 'userspace/tls-core/src/lib.rs')
        tls_lib = work / 'libdunit_tls_core.rlib'
        binary = work / 'tls-core-tests'
        subprocess.run([
            compiler, '--edition=2021', '--crate-type=rlib', '--crate-name=dunit_tls_core',
            '-Dwarnings', source, '-o', str(tls_lib),
        ], check=True)
        subprocess.run([
            compiler, '--edition=2021', '--test', '-O',
            '-Dwarnings', '-Coverflow-checks=yes', '-Cdebug-assertions=yes',
            source, '-o', str(binary),
        ], check=True)
        subprocess.run([str(binary), '--test-threads=1'], check=True, timeout=60)
        run_openssl_interop(work, compiler, tls_lib)
