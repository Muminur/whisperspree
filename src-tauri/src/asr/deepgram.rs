//! Deepgram live-streaming recognizer boundary (PRD §10.2).
//!
//! The websocket implementation is intentionally behind [`SocketConnector`].
//! It keeps parser and reconnect behavior deterministic under tests and leaves
//! actual OS/session construction to the runtime integration task.

use super::{AsrConfig, AsrEvent, FinalTranscript, SpeechRecognizer};
use crate::{error::Error, ipc::events::WordTiming};
use async_trait::async_trait;
use serde_json::Value;
use std::{collections::VecDeque, time::Duration};
use tokio::{sync::mpsc, time::sleep};

const SAMPLE_RATE_HZ: usize = 16_000;
const MAX_REPLAY_SAMPLES: usize = SAMPLE_RATE_HZ * 10;
const RECONNECT_BACKOFF: Duration = Duration::from_millis(300);
const KEEPALIVE_IDLE: Duration = Duration::from_secs(8);
const KEEPALIVE: &str = r#"{"type":"KeepAlive"}"#;
const CLOSE_STREAM: &str = r#"{"type":"CloseStream"}"#;

/// The provider websocket boundary. Production construction belongs outside the
/// recognizer; test doubles use this exact interface and never touch a network.
pub trait DeepgramSocket: Send {
    fn send_binary(&mut self, bytes: &[u8]) -> Result<(), Error>;
    fn send_text(&mut self, text: &str) -> Result<(), Error>;
    fn try_receive(&mut self) -> Result<Option<String>, Error>;
}

/// Opens a TLS websocket with the supplied endpoint and Authorization value.
/// Implementations must not log `authorization` (PRD §12 P-3).
pub trait SocketConnector: Send + Sync {
    fn connect(&self, request: &str, authorization: &str)
        -> Result<Box<dyn DeepgramSocket>, Error>;
}

/// Cloud-only inputs not present in the generic ASR trait configuration.
#[derive(Debug, Clone, Default)]
pub struct DeepgramOptions {
    keywords: Vec<String>,
}

impl DeepgramOptions {
    pub fn new(keywords: Vec<String>) -> Self {
        Self { keywords }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    Disconnected,
    Connected,
    ReconnectPending,
}

/// Cloud recognizer implementing the normative §9.3 trait.
pub struct DeepgramEngine<C: SocketConnector> {
    connector: C,
    api_key: String,
    options: DeepgramOptions,
    socket: Option<Box<dyn DeepgramSocket>>,
    state: SocketState,
    retried: bool,
    replay: VecDeque<f32>,
    transcript: FinalTranscript,
    sender: Option<mpsc::Sender<AsrEvent>>,
    request: Option<String>,
    sent_control: Vec<String>,
    replayed_samples: usize,
}

// Manual Debug: the API key must never reach any formatter (PRD §12 P-3).
impl<C: SocketConnector> std::fmt::Debug for DeepgramEngine<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeepgramEngine")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl<C: SocketConnector> DeepgramEngine<C> {
    pub fn new(connector: C, api_key: impl Into<String>, options: DeepgramOptions) -> Self {
        Self {
            connector,
            api_key: api_key.into(),
            options,
            socket: None,
            state: SocketState::Disconnected,
            retried: false,
            replay: VecDeque::new(),
            transcript: FinalTranscript {
                text: String::new(),
                words: Vec::new(),
                language: None,
            },
            sender: None,
            request: None,
            sent_control: Vec::new(),
            replayed_samples: 0,
        }
    }

    pub fn socket_state(&self) -> SocketState {
        self.state
    }
    pub fn replayed_samples_for_test(&self) -> usize {
        self.replayed_samples
    }
    pub fn sent_text_for_test(&self) -> Vec<&str> {
        self.sent_control.iter().map(String::as_str).collect()
    }

    /// Service incoming provider frames and a pending reconnect. The session
    /// runtime calls this while listening; `feed` itself remains non-blocking.
    pub async fn service(&mut self) -> Result<(), Error> {
        self.service_after_idle(Duration::ZERO).await
    }

    /// Sends a keepalive once a caller has observed at least eight seconds of
    /// silence. Exposed so the runtime can use its authoritative audio clock.
    pub async fn service_after_idle(&mut self, idle: Duration) -> Result<(), Error> {
        if self.state == SocketState::ReconnectPending {
            sleep(RECONNECT_BACKOFF).await;
            self.reconnect_and_replay().await?;
        }
        if idle >= KEEPALIVE_IDLE && self.state == SocketState::Connected {
            self.send_control(KEEPALIVE)?;
        }
        self.drain_inbound().await
    }

    fn connect(&mut self) -> Result<(), Error> {
        let request = self
            .request
            .as_deref()
            .ok_or_else(|| Error::NetStream("Deepgram session was not started".into()))?;
        let authorization = format!("Token {}", self.api_key);
        self.socket = Some(self.connector.connect(request, &authorization)?);
        self.state = SocketState::Connected;
        Ok(())
    }

    fn send_control(&mut self, value: &str) -> Result<(), Error> {
        let socket = self
            .socket
            .as_mut()
            .ok_or_else(|| Error::NetStream("Deepgram socket is not connected".into()))?;
        socket.send_text(value)?;
        self.sent_control.push(value.to_owned());
        Ok(())
    }

    fn record_audio(&mut self, samples: &[f32]) {
        self.replay.extend(samples.iter().copied());
        while self.replay.len() > MAX_REPLAY_SAMPLES {
            self.replay.pop_front();
        }
    }

    fn transport_failed(&mut self, error: Error) -> Result<(), Error> {
        self.socket = None;
        if !self.retried {
            self.retried = true;
            self.state = SocketState::ReconnectPending;
            return Ok(());
        }
        self.state = SocketState::Disconnected;
        Err(error)
    }

    async fn reconnect_and_replay(&mut self) -> Result<(), Error> {
        if let Err(error) = self.connect() {
            return self.surface_terminal_failure(error).await;
        }
        let samples: Vec<f32> = self.replay.iter().copied().collect();
        self.replayed_samples = samples.len();
        let bytes = pcm_to_linear16(&samples);
        if let Some(socket) = self.socket.as_mut() {
            if let Err(error) = socket.send_binary(&bytes) {
                return self.surface_terminal_failure(error).await;
            }
        }
        Ok(())
    }

    async fn surface_terminal_failure(&mut self, error: Error) -> Result<(), Error> {
        self.state = SocketState::Disconnected;
        if let Some(sender) = &self.sender {
            let _ = sender
                .send(AsrEvent::Error {
                    error: Error::NetStream(error.to_string()),
                })
                .await;
        }
        Ok(())
    }

    async fn drain_inbound(&mut self) -> Result<(), Error> {
        loop {
            let received = match self.socket.as_mut() {
                Some(socket) => socket.try_receive(),
                None => return Ok(()),
            };
            match received {
                Ok(Some(json)) => self.apply_message(&json).await?,
                Ok(None) => return Ok(()),
                Err(error) => {
                    if self
                        .transport_failed(Error::NetStream(error.to_string()))
                        .is_err()
                    {
                        self.surface_terminal_failure(error).await?;
                    }
                    return Ok(());
                }
            }
        }
    }

    async fn apply_message(&mut self, json: &str) -> Result<(), Error> {
        let value: Value = serde_json::from_str(json)
            .map_err(|error| Error::NetStream(format!("invalid Deepgram JSON: {error}")))?;
        if value.get("type").and_then(Value::as_str) != Some("Results") {
            return Ok(());
        }
        let Some(alt) = value.pointer("/channel/alternatives/0") else {
            return Ok(());
        };
        let text = alt
            .get("transcript")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let is_final = value
            .get("is_final")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if is_final {
            let words = parse_words(alt.get("words"));
            if !text.is_empty() {
                if !self.transcript.text.is_empty() {
                    self.transcript.text.push(' ');
                }
                self.transcript.text.push_str(&text);
                self.transcript.words.extend(words.clone());
            }
            if let Some(sender) = &self.sender {
                let _ = sender.send(AsrEvent::Segment { text, words }).await;
            }
            if self.transcript.language.is_none() {
                if let Some(code) = value
                    .pointer("/channel/detected_language")
                    .or_else(|| value.get("detected_language"))
                    .and_then(Value::as_str)
                {
                    self.transcript.language = Some(code.to_owned());
                    if let Some(sender) = &self.sender {
                        let _ = sender
                            .send(AsrEvent::LanguageDetected {
                                code: code.to_owned(),
                                confidence: 1.0,
                            })
                            .await;
                    }
                }
            }
        } else if let Some(sender) = &self.sender {
            let _ = sender.send(AsrEvent::Partial { text }).await;
        }
        Ok(())
    }
}

#[async_trait]
impl<C: SocketConnector> SpeechRecognizer for DeepgramEngine<C> {
    async fn start(&mut self, cfg: AsrConfig, tx: mpsc::Sender<AsrEvent>) -> Result<(), Error> {
        self.socket = None;
        self.state = SocketState::Disconnected;
        self.retried = false;
        self.replay.clear();
        self.replayed_samples = 0;
        self.sent_control.clear();
        self.transcript = FinalTranscript {
            text: String::new(),
            words: Vec::new(),
            language: cfg.language.clone(),
        };
        self.sender = Some(tx);
        self.request = Some(build_request(
            cfg.language.as_deref(),
            &self.options.keywords,
        ));
        self.connect()
    }

    fn feed(&mut self, pcm_16k_mono: &[f32]) -> Result<(), Error> {
        self.record_audio(pcm_16k_mono);
        if self.state != SocketState::Connected {
            return Ok(());
        }
        let bytes = pcm_to_linear16(pcm_16k_mono);
        if let Some(socket) = self.socket.as_mut() {
            if let Err(error) = socket.send_binary(&bytes) {
                self.transport_failed(error)?;
            }
        }
        Ok(())
    }

    async fn finalize(&mut self) -> Result<FinalTranscript, Error> {
        if self.state == SocketState::Connected {
            if let Err(error) = self.send_control(CLOSE_STREAM) {
                self.surface_terminal_failure(error).await?;
            }
        }
        self.drain_inbound().await?;
        Ok(self.transcript.clone())
    }

    async fn abort(&mut self) {
        self.socket = None;
        self.state = SocketState::Disconnected;
        self.replay.clear();
        self.sender = None;
    }
}

fn build_request(language: Option<&str>, keywords: &[String]) -> String {
    let mut query = vec![
        "model=nova-2".to_owned(),
        "encoding=linear16".to_owned(),
        "sample_rate=16000".to_owned(),
        "channels=1".to_owned(),
        "interim_results=true".to_owned(),
        "smart_format=false".to_owned(),
        "punctuate=true".to_owned(),
    ];
    if let Some(language) = language {
        query.push(format!("language={}", percent_encode(language)));
    } else {
        query.push("detect_language=true".to_owned());
    }
    query.extend(
        keywords
            .iter()
            .map(|keyword| format!("keywords={}", percent_encode(keyword))),
    );
    format!("wss://api.deepgram.com/v1/listen?{}", query.join("&"))
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn pcm_to_linear16(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| {
            let scaled = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            scaled.to_le_bytes()
        })
        .collect()
}

fn parse_words(value: Option<&Value>) -> Vec<WordTiming> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|word| {
            Some(WordTiming {
                w: word.get("word")?.as_str()?.to_owned(),
                s: seconds_to_ms(word.get("start")?.as_f64()?),
                e: seconds_to_ms(word.get("end")?.as_f64()?),
            })
        })
        .collect()
}

fn seconds_to_ms(seconds: f64) -> u32 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (seconds * 1000.0).round().min(u32::MAX as f64) as u32
}

/// Real TLS connector (T3.1): opens `wss://api.deepgram.com` sockets through
/// tokio-tungstenite with rustls native roots. Endpoint validation mirrors the
/// network guard policy and runs BEFORE any I/O so off-host or cleartext
/// targets fail fast with the closed §14 code.
pub struct TlsSocketConnector;

impl Default for TlsSocketConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl TlsSocketConnector {
    pub fn new() -> Self {
        Self
    }

    /// Validates the endpoint without performing network I/O.
    pub fn validate_endpoint(&self, url: &str) -> Result<(), Error> {
        let rest = url
            .strip_prefix("wss://")
            .ok_or_else(|| Error::NetStream("deepgram endpoint must use wss://".into()))?;
        let host = rest.split('/').next().unwrap_or_default();
        if host != "api.deepgram.com" {
            return Err(Error::NetStream(format!(
                "refusing non-Deepgram websocket host: {host}"
            )));
        }
        Ok(())
    }
}

/// Constructs the cloud engine from the keychain (T3.1): a missing Deepgram
/// key is the closed `ASR-NOMODEL` condition, never a network attempt.
pub fn engine_from_store<C: SocketConnector>(
    store: &dyn crate::store::keychain::KeyStore,
    connector: C,
    options: DeepgramOptions,
) -> Result<DeepgramEngine<C>, Error> {
    let account = crate::store::keychain::Provider::Deepgram.account_name();
    let key = store
        .get(account)?
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| Error::AsrNoModel("no Deepgram key in keychain".into()))?;
    Ok(DeepgramEngine::new(connector, key, options))
}

/// Sync facade over the spawned tungstenite pump. Outbound frames funnel
/// through a tokio unbounded channel; inbound text drains a std queue so
/// `try_receive` stays non-blocking exactly like the deterministic double.
struct LiveSocket {
    outbound: tokio::sync::mpsc::UnboundedSender<Frame>,
    inbound: std::sync::mpsc::Receiver<Result<String, Error>>,
}

enum Frame {
    Binary(Vec<u8>),
    Text(String),
    #[allow(dead_code)] // reserved for the CloseStream drain path in slice 3
    Close,
}

impl DeepgramSocket for LiveSocket {
    fn send_binary(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.outbound
            .send(Frame::Binary(bytes.to_vec()))
            .map_err(|_| Error::NetStream("websocket pump is gone".into()))
    }

    fn send_text(&mut self, text: &str) -> Result<(), Error> {
        self.outbound
            .send(Frame::Text(text.to_string()))
            .map_err(|_| Error::NetStream("websocket pump is gone".into()))
    }

    fn try_receive(&mut self) -> Result<Option<String>, Error> {
        match self.inbound.try_recv() {
            Ok(message) => message.map(Some),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(Error::NetStream("websocket closed".into()))
            }
        }
    }
}

impl SocketConnector for TlsSocketConnector {
    fn connect(
        &self,
        request: &str,
        authorization: &str,
    ) -> Result<Box<dyn DeepgramSocket>, Error> {
        self.validate_endpoint(request)?;
        // Caller must already sit inside the session's Tokio runtime
        // (MacSessionRuntime owns one); we only spawn from here.
        let _handle = tokio::runtime::Handle::try_current()
            .map_err(|_| Error::NetStream("cloud connector requires the session runtime".into()))?;
        let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::unbounded_channel::<Frame>();
        let (inbound_tx, inbound_rx) = std::sync::mpsc::channel::<Result<String, Error>>();
        let request = request.to_string();
        let authorization = authorization.to_string();
        tokio::spawn(async move {
            use futures_util::{SinkExt, StreamExt};
            let ws_request = match tungstenite_request(&request, &authorization) {
                Ok(request) => request,
                Err(error) => {
                    let _ = inbound_tx.send(Err(error));
                    return;
                }
            };
            let (mut ws, _) = match tokio_tungstenite::connect_async(ws_request).await {
                Ok(pair) => pair,
                Err(error) => {
                    let _ = inbound_tx.send(Err(Error::NetStream(format!(
                        "deepgram connect failed: {error}"
                    ))));
                    return;
                }
            };
            loop {
                tokio::select! {
                    frame = outbound_rx.recv() => {
                        match frame {
                            Some(Frame::Binary(bytes)) => {
                                if ws.send(tokio_tungstenite::tungstenite::Message::Binary(bytes)).await.is_err() {
                                    break;
                                }
                            }
                            Some(Frame::Text(text)) => {
                                if ws.send(tokio_tungstenite::tungstenite::Message::Text(text)).await.is_err() {
                                    break;
                                }
                            }
                            Some(Frame::Close) | None => {
                                let _ = ws.close(None).await;
                                break;
                            }
                        }
                    }
                    message = ws.next() => {
                        match message {
                            Some(Ok(msg)) => {
                                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                                    if inbound_tx.send(Ok(text)).is_err() {
                                        break;
                                    }
                                }
                            }
                            Some(Err(error)) => {
                                let _ = inbound_tx.send(Err(Error::NetStream(format!(
                                    "deepgram stream failed: {error}"
                                ))));
                                break;
                            }
                            None => break,
                        }
                    }
                }
            }
        });
        Ok(Box::new(LiveSocket {
            outbound: outbound_tx,
            inbound: inbound_rx,
        }))
    }
}

fn tungstenite_request(
    url: &str,
    authorization: &str,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, Error> {
    tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url)
        .map_err(|e| Error::NetStream(format!("invalid websocket request: {e}")))
        .map(|mut request| {
            request.headers_mut().insert(
                "Authorization",
                tokio_tungstenite::tungstenite::http::HeaderValue::from_str(authorization)
                    .map_err(|e| Error::NetStream(format!("bad authorization header: {e}")))?,
            );
            Ok(request)
        })?
}
