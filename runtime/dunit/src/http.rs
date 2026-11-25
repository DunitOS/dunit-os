//! Bounded HTTP/1.1 & HTTPS (TLS 1.3) client over native `dunit::net`, `dunit::rand`, and `dunit::cert`.
use crate::{
    cert,
    io::{Read, Write},
    net::{NetGrant, TcpStream},
    rand,
    time::{realtime_secs, Duration},
};
use alloc::vec::Vec;
pub use dunit_http_core::{
    Header, HttpError, Limits, Method, Request, Response, Scheme, SessionStats, TlsFailure, Url,
};
use dunit_http_core::{Connector, Session, Transport};
use dunit_tls_core::{ByteStream, Certificate, TlsClient, TlsError};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

struct TcpByteStream(TcpStream);

impl ByteStream for TcpByteStream {
    fn read_bytes(&mut self, buf: &mut [u8]) -> core::result::Result<usize, TlsError> {
        self.0
            .read(buf)
            .map_err(|err| TlsError::Transport(err.raw_os_error()))
    }

    fn write_all_bytes(&mut self, buf: &[u8]) -> core::result::Result<(), TlsError> {
        self.0
            .write_all(buf)
            .map_err(|err| TlsError::Transport(err.raw_os_error()))
    }
}

enum HttpTransport {
    Plain(TcpStream),
    Tls(TlsClient<TcpByteStream>),
}

impl Transport for HttpTransport {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, HttpError> {
        match self {
            Self::Plain(tcp) => tcp
                .read(buf)
                .map_err(|err| HttpError::Transport(err.raw_os_error())),
            Self::Tls(tls) => tls.read(buf).map_err(map_tls_error),
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> core::result::Result<(), HttpError> {
        match self {
            Self::Plain(tcp) => tcp
                .write_all(buf)
                .map_err(|err| HttpError::Transport(err.raw_os_error())),
            Self::Tls(tls) => tls.write_all(buf).map_err(map_tls_error),
        }
    }
}

fn map_tls_error(err: TlsError) -> HttpError {
    match err {
        TlsError::InvalidCertificate => HttpError::Tls(TlsFailure::InvalidCertificate),
        TlsError::CertificateExpired => HttpError::Tls(TlsFailure::CertificateExpired),
        TlsError::UntrustedRoot => HttpError::Tls(TlsFailure::UntrustedRoot),
        TlsError::BadSignature => HttpError::Tls(TlsFailure::BadSignature),
        TlsError::Transport(code) => HttpError::Transport(code),
        _ => HttpError::Tls(TlsFailure::Handshake),
    }
}

struct NetConnector<'a> {
    grant: &'a NetGrant,
    timeout: Duration,
    roots: Vec<Certificate>,
}

impl<'a> Connector for NetConnector<'a> {
    type Conn = HttpTransport;

    fn connect(
        &mut self,
        scheme: Scheme,
        host: &str,
        port: u16,
    ) -> core::result::Result<Self::Conn, HttpError> {
        let mut stream = TcpStream::connect_host_timeout(self.grant, host, port, self.timeout)
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
        match scheme {
            Scheme::Http => Ok(HttpTransport::Plain(stream)),
            Scheme::Https => {
                let now_unix = realtime_secs()
                    .map_err(|_| HttpError::Tls(TlsFailure::ClockUnavailable))?;
                let entropy = rand::bytes::<32>()
                    .map_err(|_| HttpError::Tls(TlsFailure::EntropyUnavailable))?;
                let eph_priv = rand::bytes::<32>()
                    .map_err(|_| HttpError::Tls(TlsFailure::EntropyUnavailable))?;
                let tls = TlsClient::connect(
                    TcpByteStream(stream),
                    host,
                    &self.roots,
                    now_unix,
                    entropy,
                    eph_priv,
                )
                .map_err(map_tls_error)?;
                Ok(HttpTransport::Tls(tls))
            }
        }
    }
}

pub struct Client<'a> {
    session: Session<NetConnector<'a>>,
}

impl<'a> Client<'a> {
    pub fn new(grant: &'a NetGrant) -> Self {
        Self::with_roots(grant, cert::load_default_roots(), Limits::default(), DEFAULT_TIMEOUT)
    }

    pub fn with_config(grant: &'a NetGrant, limits: Limits, timeout: Duration) -> Self {
        Self::with_roots(grant, cert::load_default_roots(), limits, timeout)
    }

    pub fn with_roots(
        grant: &'a NetGrant,
        roots: Vec<Certificate>,
        limits: Limits,
        timeout: Duration,
    ) -> Self {
        Self {
            session: Session::new(
                NetConnector {
                    grant,
                    timeout,
                    roots,
                },
                limits,
            ),
        }
    }

    pub fn get(&mut self, url: &str) -> core::result::Result<Response, HttpError> {
        let req = Request::new(Method::Get, url)?;
        self.request(&req)
    }

    pub fn request(&mut self, req: &Request) -> core::result::Result<Response, HttpError> {
        self.session.send(req)
    }

    pub fn stats(&self) -> SessionStats {
        self.session.stats()
    }

    pub fn idle_connections(&self) -> usize {
        self.session.idle_connections()
    }
}
