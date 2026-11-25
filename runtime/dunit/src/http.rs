//! Bounded HTTP/1.1 client over native `dunit::net` TCP and DNS handles.
use crate::{
    io::{Read, Write},
    net::{NetGrant, TcpStream},
    time::Duration,
};
use dunit_http_core::{Connector, Session, Transport};
pub use dunit_http_core::{
    Header, HttpError, Limits, Method, Request, Response, SessionStats, Url,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

struct TcpTransport(TcpStream);

impl Transport for TcpTransport {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, HttpError> {
        self.0
            .read(buf)
            .map_err(|err| HttpError::Transport(err.raw_os_error()))
    }

    fn write_all(&mut self, buf: &[u8]) -> core::result::Result<(), HttpError> {
        self.0
            .write_all(buf)
            .map_err(|err| HttpError::Transport(err.raw_os_error()))
    }
}

struct NetConnector<'a> {
    grant: &'a NetGrant,
    timeout: Duration,
}

impl<'a> Connector for NetConnector<'a> {
    type Conn = TcpTransport;

    fn connect(&mut self, host: &str, port: u16) -> core::result::Result<Self::Conn, HttpError> {
        let mut stream = TcpStream::connect_host_timeout(self.grant, host, port, self.timeout)
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|err| HttpError::Transport(err.raw_os_error()))?;
        Ok(TcpTransport(stream))
    }
}

pub struct Client<'a> {
    session: Session<NetConnector<'a>>,
}

impl<'a> Client<'a> {
    pub fn new(grant: &'a NetGrant) -> Self {
        Self::with_config(grant, Limits::default(), DEFAULT_TIMEOUT)
    }

    pub fn with_config(grant: &'a NetGrant, limits: Limits, timeout: Duration) -> Self {
        Self {
            session: Session::new(NetConnector { grant, timeout }, limits),
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
