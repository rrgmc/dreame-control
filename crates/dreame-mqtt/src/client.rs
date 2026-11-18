//! The connection task: connect, subscribe, read pushes, keep alive, reconnect.

use crate::config::{MqttConfig, TokenSource};
use crate::decode::{decode_message, Push};
use crate::packet::{self, FrameError, Incoming};
use futures_core::Stream;
use rustls::pki_types::ServerName;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Instant};
use tokio_rustls::TlsConnector;

/// Time allowed for TCP connect, TLS handshake, CONNACK and SUBACK.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(120);
/// Pushes buffered for a slow consumer before the reader waits.
const CHANNEL_SIZE: usize = 256;

/// Connection state, published through [`MqttHandle::state`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// Getting a token or opening the connection. `attempt` counts from 1
    /// and resets after each successful subscription.
    Connecting { attempt: u32 },
    /// CONNACK accepted and SUBACK granted; pushes flow.
    Connected,
    /// The last attempt or connection failed; the next one starts after `retry_in`.
    /// `reason` never contains the token or ids.
    Waiting { retry_in: Duration, reason: String },
    /// Stopped for good: [`MqttHandle::stop`] was called, the handle or the
    /// stream was dropped.
    Stopped,
}

/// Entry point. See the crate documentation for an example.
pub struct MqttClient;

impl MqttClient {
    /// Starts the connection task and returns its handle and the push stream.
    ///
    /// Must be called inside a tokio runtime. The task connects at once and
    /// keeps reconnecting with exponential backoff (1 s doubling up to 120 s,
    /// reset after a successful subscription). Before each attempt it asks
    /// `tokens` for the access token. After CONNACK 4 or 5 it asks for a
    /// refreshed one.
    ///
    /// The task stops when [`MqttHandle::stop`] is called, or when the handle
    /// or the stream is dropped.
    pub fn connect(cfg: MqttConfig, tokens: impl TokenSource) -> (MqttHandle, PushStream) {
        let (tx, rx) = mpsc::channel(CHANNEL_SIZE);
        let (stop_tx, stop_rx) = watch::channel(false);
        let (state_tx, state_rx) = watch::channel(ConnectionState::Connecting { attempt: 1 });
        let task = tokio::spawn(run(cfg, Arc::new(tokens), tx, stop_rx, state_tx));
        (MqttHandle { stop: stop_tx, state: state_rx, task: Some(task) }, PushStream { rx })
    }
}

/// Controls a running connection task.
pub struct MqttHandle {
    stop: watch::Sender<bool>,
    state: watch::Receiver<ConnectionState>,
    task: Option<JoinHandle<()>>,
}

impl fmt::Debug for MqttHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttHandle").field("state", &*self.state.borrow()).finish()
    }
}

impl MqttHandle {
    /// A receiver for connection state changes.
    pub fn state(&self) -> watch::Receiver<ConnectionState> {
        self.state.clone()
    }

    /// Sends DISCONNECT if connected, closes the connection and waits for
    /// the task to end. The push stream then ends.
    pub async fn stop(mut self) {
        let _ = self.stop.send(true);
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}

impl Drop for MqttHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

/// The decoded pushes, in arrival order. Ends when the task stops.
#[derive(Debug)]
pub struct PushStream {
    rx: mpsc::Receiver<Push>,
}

impl PushStream {
    /// The next push, or `None` once the task has stopped.
    pub async fn recv(&mut self) -> Option<Push> {
        self.rx.recv().await
    }
}

impl Stream for PushStream {
    type Item = Push;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Push>> {
        self.rx.poll_recv(cx)
    }
}

/// Why a session ended without a stop request.
#[derive(Debug)]
enum SessionError {
    /// CONNACK with a non-zero return code.
    Refused(u8),
    Failed(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(c) => write!(f, "connection refused, CONNACK code {c}"),
            Self::Failed(m) => f.write_str(m),
        }
    }
}

fn failed(what: &str, e: impl fmt::Display) -> SessionError {
    SessionError::Failed(format!("{what}: {e}"))
}

async fn run(
    cfg: MqttConfig,
    tokens: Arc<dyn TokenSource>,
    tx: mpsc::Sender<Push>,
    mut stop: watch::Receiver<bool>,
    state: watch::Sender<ConnectionState>,
) {
    let tls = match crate::tls::client_config() {
        Ok(c) => TlsConnector::from(Arc::new(c)),
        Err(e) => {
            tracing::error!("mqtt: TLS setup failed: {e}");
            let _ = state.send(ConnectionState::Stopped);
            return;
        }
    };
    let mut backoff = BACKOFF_MIN;
    let mut attempt = 1;
    let mut refresh = false;
    loop {
        let _ = state.send(ConnectionState::Connecting { attempt });
        let token = tokio::select! {
            _ = stopped(&mut stop) => break,
            _ = tx.closed() => break,
            t = tokens.token(refresh) => t,
        };
        refresh = false;
        let reason = match token {
            Err(e) => format!("token source: {e}"),
            Ok(token) => {
                let mut subscribed = false;
                match session(&cfg, &tls, &token, &tx, &mut stop, &state, &mut subscribed).await {
                    Ok(()) => break,
                    Err(e) => {
                        if subscribed {
                            backoff = BACKOFF_MIN;
                            attempt = 0;
                        }
                        if matches!(e, SessionError::Refused(4 | 5)) {
                            refresh = true;
                        }
                        e.to_string()
                    }
                }
            }
        };
        let jitter = backoff.mul_f64(rand::random::<f64>() * 0.25);
        let retry_in = backoff + jitter;
        tracing::warn!("mqtt: {reason}; retrying in {retry_in:?}");
        let _ = state.send(ConnectionState::Waiting { retry_in, reason });
        tokio::select! {
            _ = stopped(&mut stop) => break,
            _ = tx.closed() => break,
            _ = tokio::time::sleep(retry_in) => {}
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
        attempt += 1;
    }
    let _ = state.send(ConnectionState::Stopped);
}

/// Resolves once a stop was requested or the handle was dropped.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await.map(|_| ());
}

/// One connection. Returns `Ok` when asked to stop (or the consumer is
/// gone) and `Err` when the connection failed or ended.
async fn session(
    cfg: &MqttConfig,
    tls: &TlsConnector,
    token: &str,
    tx: &mpsc::Sender<Push>,
    stop: &mut watch::Receiver<bool>,
    state: &watch::Sender<ConnectionState>,
    subscribed: &mut bool,
) -> Result<(), SessionError> {
    let name = ServerName::try_from(cfg.host.clone()).map_err(|e| failed("bad host name", e))?;
    let open = async {
        let tcp = TcpStream::connect((cfg.host.as_str(), cfg.port)).await.map_err(|e| failed("tcp connect", e))?;
        let _ = tcp.set_nodelay(true);
        tls.connect(name, tcp).await.map_err(|e| failed("tls handshake", e))
    };
    let mut stream = tokio::select! {
        _ = stopped(stop) => return Ok(()),
        r = timeout(HANDSHAKE_TIMEOUT, open) => r.map_err(|_| failed("connect", "timed out"))??,
    };

    let keep_alive = cfg.keep_alive.as_secs().clamp(10, u16::MAX as u64) as u16;
    stream
        .write_all(&packet::connect(&cfg.client_id, &cfg.uid, token, keep_alive))
        .await
        .map_err(|e| failed("write CONNECT", e))?;

    let topics = cfg.topics();
    let mut buf = Vec::with_capacity(8192);
    let mut chunk = vec![0u8; 8192];
    let handshake_deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut connacked = false;
    let ping_every = Duration::from_secs(keep_alive as u64 / 2);
    let mut ping = tokio::time::interval_at(Instant::now() + ping_every, ping_every);
    let mut last_rx = Instant::now();

    loop {
        tokio::select! {
            _ = stopped(stop) => {
                let _ = stream.write_all(&packet::DISCONNECT).await;
                let _ = stream.shutdown().await;
                return Ok(());
            }
            _ = tx.closed() => {
                let _ = stream.write_all(&packet::DISCONNECT).await;
                let _ = stream.shutdown().await;
                return Ok(());
            }
            _ = tokio::time::sleep_until(handshake_deadline), if !*subscribed => {
                return Err(failed(if connacked { "SUBACK" } else { "CONNACK" }, "timed out"));
            }
            _ = ping.tick() => {
                if last_rx.elapsed() > ping_every * 3 {
                    return Err(failed("keep-alive", "no data from broker"));
                }
                stream.write_all(&packet::PINGREQ).await.map_err(|e| failed("write PINGREQ", e))?;
            }
            r = stream.read(&mut chunk) => {
                let n = r.map_err(|e| failed("read", e))?;
                if n == 0 {
                    return Err(failed("read", "connection closed by broker"));
                }
                last_rx = Instant::now();
                buf.extend_from_slice(&chunk[..n]);
                while let Some(p) = packet::take_packet(&mut buf).map_err(|e| match e {
                    FrameError::TooLarge(n) => failed("packet", format!("too large ({n} bytes)")),
                    FrameError::Malformed(m) => failed("packet", m),
                })? {
                    match p {
                        Incoming::ConnAck { code: 0 } if !connacked => {
                            connacked = true;
                            stream.write_all(&packet::subscribe(1, &topics)).await.map_err(|e| failed("write SUBSCRIBE", e))?;
                        }
                        Incoming::ConnAck { code } if !connacked => return Err(SessionError::Refused(code)),
                        Incoming::SubAck { granted, .. } if connacked && !*subscribed => {
                            if granted.len() != topics.len() || granted.contains(&0x80) {
                                return Err(failed("SUBACK", "subscription refused"));
                            }
                            *subscribed = true;
                            tracing::info!("mqtt: connected and subscribed");
                            let _ = state.send(ConnectionState::Connected);
                        }
                        Incoming::Publish { payload, .. } => match decode_message(&payload) {
                            Ok(pushes) => {
                                for push in pushes {
                                    if tx.send(push).await.is_err() {
                                        let _ = stream.write_all(&packet::DISCONNECT).await;
                                        return Ok(());
                                    }
                                }
                            }
                            Err(e) => tracing::debug!("mqtt: undecodable payload of {} bytes: {e}", payload.len()),
                        },
                        Incoming::PingResp => {}
                        other => tracing::debug!("mqtt: ignored packet {other:?}"),
                    }
                }
            }
        }
    }
}
