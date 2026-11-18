//! A minimal HTTP server that offers one file on the local network, for a map restore from a
//! file the client holds (property 6.10 takes a URL). It answers `GET` and `HEAD` for one path,
//! supports single byte ranges, and logs every request so a caller can see whether and how the
//! robot fetched the file. Requests from any address other than the expected peer get 403.

use anyhow::{anyhow, Context, Result};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// One request as the server saw it.
#[derive(Debug, Clone)]
pub struct Request {
    pub from_peer: bool,
    pub method: String,
    pub path: String,
    pub range: Option<String>,
    pub user_agent: Option<String>,
    pub status: u16,
    pub body_bytes: usize,
}

/// A running server. Dropping it stops it.
pub struct FileServer {
    url: String,
    log: Arc<Mutex<Vec<Request>>>,
    body: Arc<Mutex<Arc<Vec<u8>>>>,
    task: JoinHandle<()>,
}

impl Drop for FileServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The local address this machine uses to reach `peer`. Connecting a UDP socket sends nothing.
pub fn local_addr_toward(peer: IpAddr) -> Result<IpAddr> {
    let bind = if peer.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
    let s = UdpSocket::bind(bind)?;
    s.connect(SocketAddr::new(peer, 9))?;
    Ok(s.local_addr()?.ip())
}

impl FileServer {
    /// Serves `body` at `/<name>` on the interface that faces `peer`, on `port` (0 picks a free
    /// one). A fixed port lets one firewall rule cover every build of a client.
    pub async fn start(body: Vec<u8>, name: &str, peer: IpAddr, port: u16) -> Result<Self> {
        let ip = local_addr_toward(peer).context("no local address toward the robot")?;
        let listener = TcpListener::bind(SocketAddr::new(ip, port))
            .await
            .with_context(|| format!("cannot listen on port {port}"))?;
        let port = listener.local_addr()?.port();
        let path = format!("/{name}");
        let url = match ip {
            IpAddr::V4(v4) => format!("http://{v4}:{port}{path}"),
            IpAddr::V6(v6) => format!("http://[{v6}]:{port}{path}"),
        };
        let log = Arc::new(Mutex::new(Vec::new()));
        let body = Arc::new(Mutex::new(Arc::new(body)));
        let (l2, b2, path2) = (log.clone(), body.clone(), path.clone());
        let task = tokio::spawn(async move {
            while let Ok((stream, addr)) = listener.accept().await {
                let current = b2.lock().map(|b| b.clone()).unwrap_or_default();
                let (log, body, path) = (l2.clone(), current, path2.clone());
                tokio::spawn(async move {
                    let _ = serve(stream, addr.ip() == peer, &path, &body, &log).await;
                });
            }
        });
        Ok(Self { url, log, body, task })
    }

    /// Replaces the file served from now on. A caller can start the server early, before the
    /// file exists, so that a firewall prompt for the port shows while other work runs.
    pub fn set_body(&self, body: Vec<u8>) {
        if let Ok(mut b) = self.body.lock() {
            *b = Arc::new(body);
        }
    }

    /// The URL to give the robot. It holds this machine's LAN address; do not print it.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The port the server listens on.
    pub fn port(&self) -> u16 {
        self.url.rsplit_once(':').and_then(|(_, rest)| rest.split('/').next()?.parse().ok()).unwrap_or(0)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

async fn serve(mut s: TcpStream, from_peer: bool, path: &str, body: &[u8], log: &Mutex<Vec<Request>>) -> Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        if buf.len() > 16 * 1024 {
            return Err(anyhow!("request head too long"));
        }
        let n = tokio::time::timeout(std::time::Duration::from_secs(20), s.read(&mut chunk)).await??;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf).to_string();
    let r = respond(&head, from_peer, path, body.len());
    let mut out = format!("HTTP/1.1 {} {}\r\nConnection: close\r\n", r.status, reason(r.status));
    out.push_str(&format!("Content-Length: {}\r\n", r.body.as_ref().map_or(0, |b| b.1 - b.0)));
    if r.status == 200 || r.status == 206 {
        out.push_str("Content-Type: application/octet-stream\r\nAccept-Ranges: bytes\r\n");
    }
    if let Some((a, b)) = r.body.filter(|_| r.status == 206) {
        out.push_str(&format!("Content-Range: bytes {a}-{}/{}\r\n", b - 1, body.len()));
    }
    out.push_str("\r\n");
    s.write_all(out.as_bytes()).await?;
    let mut sent = 0;
    if let (Some((a, b)), false) = (r.body, r.request.method == "HEAD") {
        s.write_all(&body[a..b]).await?;
        sent = b - a;
    }
    s.shutdown().await.ok();
    let mut req = r.request;
    req.body_bytes = sent;
    if let Ok(mut l) = log.lock() {
        l.push(req);
    }
    Ok(())
}

struct Response {
    status: u16,
    /// Byte range `[start, end)` of the body to send.
    body: Option<(usize, usize)>,
    request: Request,
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// Parses `bytes=a-b`, `bytes=a-` or `bytes=-n` for a body of `len` bytes.
fn parse_range(v: &str, len: usize) -> Option<(usize, usize)> {
    let spec = v.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: usize = n.parse().ok()?;
            (len.saturating_sub(n), len)
        }
        (a, "") => (a.parse().ok()?, len),
        (a, b) => (a.parse().ok()?, b.parse::<usize>().ok()?.saturating_add(1).min(len)),
    };
    (start < end).then_some((start, end))
}

fn respond(head: &str, from_peer: bool, path: &str, len: usize) -> Response {
    let first = head.lines().next().unwrap_or_default();
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let range = header(head, "range").map(str::to_string);
    let request = Request {
        from_peer,
        method: method.clone(),
        path: target.clone(),
        range: range.clone(),
        user_agent: header(head, "user-agent").map(str::to_string),
        status: 0,
        body_bytes: 0,
    };
    let (status, body) = if !from_peer {
        (403, None)
    } else if method != "GET" && method != "HEAD" {
        (405, None)
    } else if target.split('?').next() != Some(path) {
        (404, None)
    } else {
        match range {
            None => (200, Some((0, len))),
            Some(r) => match parse_range(&r, len) {
                Some(span) => (206, Some(span)),
                None => (416, None),
            },
        }
    };
    Response { status, body, request: Request { status, ..request } }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 10)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 100)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 100)));
        assert_eq!(parse_range("bytes=95-200", 100), Some((95, 100)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
    }

    #[test]
    fn responses() {
        let get = "GET /a.tbz2 HTTP/1.1\r\nHost: x\r\nRange: bytes=10-\r\nUser-Agent: robot\r\n\r\n";
        let r = respond(get, true, "/a.tbz2", 100);
        assert_eq!((r.status, r.body), (206, Some((10, 100))));
        assert_eq!(r.request.user_agent.as_deref(), Some("robot"));
        assert_eq!(respond(get, false, "/a.tbz2", 100).status, 403);
        assert_eq!(respond("GET /b HTTP/1.1\r\n\r\n", true, "/a.tbz2", 100).status, 404);
        assert_eq!(respond("GET /a.tbz2?x=1 HTTP/1.1\r\n\r\n", true, "/a.tbz2", 100).body, Some((0, 100)));
        assert_eq!(respond("POST /a.tbz2 HTTP/1.1\r\n\r\n", true, "/a.tbz2", 100).status, 405);
    }

    #[tokio::test]
    async fn serves_the_file_to_the_peer() {
        let body: Vec<u8> = (0..=255).collect();
        let srv = FileServer::start(body.clone(), "f.bin", "127.0.0.1".parse().unwrap(), 0).await.unwrap();
        assert_ne!(srv.port(), 0);
        let addr = srv.url().trim_start_matches("http://").split('/').next().unwrap().to_string();
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /f.bin HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        let mut got = Vec::new();
        s.read_to_end(&mut got).await.unwrap();
        let split = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert!(got.starts_with(b"HTTP/1.1 200 OK"));
        assert_eq!(&got[split..], &body[..]);
        let log = srv.requests();
        assert_eq!(log.len(), 1);
        assert_eq!((log[0].status, log[0].body_bytes), (200, 256));
    }

    #[tokio::test]
    async fn serves_a_body_set_after_start() {
        let srv = FileServer::start(Vec::new(), "f.bin", "127.0.0.1".parse().unwrap(), 0).await.unwrap();
        srv.set_body(b"late".to_vec());
        let addr = srv.url().trim_start_matches("http://").split('/').next().unwrap().to_string();
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(b"GET /f.bin HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        let mut got = Vec::new();
        s.read_to_end(&mut got).await.unwrap();
        assert!(got.ends_with(b"\r\n\r\nlate"));
    }
}
