//! The HTTP layer, behind a small trait so tests can replay recorded replies
//! and a fingerprint-copying client can replace `reqwest` later.

use dreame_core::{Error, Result};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// A boxed, sendable future. Used instead of `async fn` in the trait so the
/// trait stays object safe.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// One HTTP request. Header names are lowercase.
#[derive(Clone)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    /// The first value of a header, by lowercase name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// The URL path, without scheme, host and query.
    pub fn path(&self) -> &str {
        let rest = self.url.split_once("://").map(|(_, r)| r).unwrap_or(&self.url);
        let path = rest.find('/').map(|i| &rest[i..]).unwrap_or("/");
        path.split('?').next().unwrap_or(path)
    }
}

/// Redacts secret headers and the body, which can hold a password hash or a
/// refresh token. Signed OSS URLs are not printed either.
impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(k, v)| match k.as_str() {
                "dreame-auth" | "authorization" => (k.as_str(), "<redacted>"),
                _ => (k.as_str(), v.as_str()),
            })
            .collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("path", &self.path())
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// One HTTP response, with the body already read and decompressed.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Sends HTTP requests. Implementations must not log request bodies or the
/// `dreame-auth` and `authorization` headers.
pub trait HttpTransport: Send + Sync {
    fn send(&self, req: HttpRequest) -> BoxFuture<'_, Result<HttpResponse>>;
}

/// The default transport: `reqwest` with `rustls`.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::Http(format!("cannot build HTTP client: {e}")))?;
        Ok(Self { client })
    }

    /// Uses an existing `reqwest` client, for example one with a proxy.
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl HttpTransport for ReqwestTransport {
    fn send(&self, req: HttpRequest) -> BoxFuture<'_, Result<HttpResponse>> {
        Box::pin(async move {
            let mut b = match req.method {
                Method::Get => self.client.get(&req.url),
                Method::Post => self.client.post(&req.url),
            };
            for (k, v) in &req.headers {
                b = b.header(k.as_str(), v.as_str());
            }
            if !req.body.is_empty() {
                b = b.body(req.body);
            }
            // `without_url` keeps signed URLs out of error messages.
            let resp = b.send().await.map_err(|e| Error::Http(e.without_url().to_string()))?;
            let status = resp.status().as_u16();
            let body = resp.bytes().await.map_err(|e| Error::Http(e.without_url().to_string()))?.to_vec();
            Ok(HttpResponse { status, body })
        })
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! A transport that replays queued replies and records requests.

    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MockTransport {
        replies: Mutex<VecDeque<(u16, String)>>,
        pub requests: Mutex<Vec<HttpRequest>>,
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn push(&self, status: u16, body: impl Into<String>) -> &Self {
            self.replies.lock().unwrap().push_back((status, body.into()));
            self
        }

        pub fn requests(&self) -> Vec<HttpRequest> {
            self.requests.lock().unwrap().clone()
        }

        pub fn remaining(&self) -> usize {
            self.replies.lock().unwrap().len()
        }
    }

    impl HttpTransport for MockTransport {
        fn send(&self, req: HttpRequest) -> BoxFuture<'_, Result<HttpResponse>> {
            self.requests.lock().unwrap().push(req);
            let next = self.replies.lock().unwrap().pop_front();
            Box::pin(async move {
                let (status, body) = next.ok_or_else(|| Error::Http("mock: no reply queued".into()))?;
                // Status 0 stands for a transport error, such as a dropped connection.
                if status == 0 {
                    return Err(Error::Http(body));
                }
                Ok(HttpResponse { status, body: body.into_bytes() })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_debug_redacts() {
        let r = HttpRequest {
            method: Method::Post,
            url: "https://us.iot.dreame.tech:13267/dreame-auth/oauth/token?x=1".into(),
            headers: vec![
                ("dreame-auth".into(), "SECRET-TOKEN".into()),
                ("authorization".into(), "Basic SECRET".into()),
                ("tenant-id".into(), "000000".into()),
            ],
            body: b"password=SECRET-HASH".to_vec(),
        };
        let s = format!("{r:?}");
        assert!(!s.contains("SECRET"), "{s}");
        assert!(s.contains("/dreame-auth/oauth/token"));
        assert_eq!(r.path(), "/dreame-auth/oauth/token");
        assert_eq!(r.header("tenant-id"), Some("000000"));
    }
}
