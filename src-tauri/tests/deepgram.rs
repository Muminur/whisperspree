//! T3.1 Deepgram boundary contract tests.  The socket is a deterministic
//! network-boundary double: no test opens a connection or sends provider traffic.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc;
use whisperspree_lib::asr::{
    deepgram::{DeepgramEngine, DeepgramOptions, DeepgramSocket, SocketConnector, SocketState},
    AsrConfig, AsrEvent, SpeechRecognizer,
};
use whisperspree_lib::error::Error;

const INTERIM: &str = include_str!("fixtures/deepgram/interim.json");
const FINAL: &str = include_str!("fixtures/deepgram/final.json");

#[derive(Default)]
struct ScriptedSocket {
    inbound: VecDeque<Result<String, Error>>,
    sent_binary: Vec<Vec<u8>>,
    sent_text: Vec<String>,
}

impl DeepgramSocket for ScriptedSocket {
    fn send_binary(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.sent_binary.push(bytes.to_vec());
        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<(), Error> {
        self.sent_text.push(text.to_owned());
        Ok(())
    }

    fn try_receive(&mut self) -> Result<Option<String>, Error> {
        self.inbound.pop_front().transpose()
    }
}

#[derive(Clone, Default)]
struct ScriptedConnector {
    sockets: Arc<Mutex<VecDeque<Result<ScriptedSocket, Error>>>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl SocketConnector for ScriptedConnector {
    fn connect(
        &self,
        request: &str,
        _authorization: &str,
    ) -> Result<Box<dyn DeepgramSocket>, Error> {
        self.requests.lock().unwrap().push(request.to_owned());
        self.sockets
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Err(Error::NetStream("no scripted socket".into())))
            .map(|socket| Box::new(socket) as Box<dyn DeepgramSocket>)
    }
}

fn engine(connector: ScriptedConnector) -> DeepgramEngine<ScriptedConnector> {
    DeepgramEngine::new(connector, "test-key", DeepgramOptions::default())
}

mod deepgram {
    use super::*;

    #[tokio::test]
    async fn parse_recorded_interim_and_final_maps_events_and_word_milliseconds() {
        let connector = ScriptedConnector {
            sockets: Arc::new(Mutex::new(VecDeque::from([Ok(ScriptedSocket {
                inbound: VecDeque::from([Ok(INTERIM.into()), Ok(FINAL.into())]),
                ..Default::default()
            })]))),
            ..Default::default()
        };
        let mut recognizer = engine(connector);
        let (tx, mut rx) = mpsc::channel(8);

        recognizer
            .start(AsrConfig::local("unused"), tx)
            .await
            .unwrap();
        recognizer.service().await.unwrap();

        assert!(matches!(rx.recv().await, Some(AsrEvent::Partial { text }) if text == "hello wor"));
        assert!(
            matches!(rx.recv().await, Some(AsrEvent::Segment { text, words })
            if text == "hello world" && words.iter().map(|word| (word.w.as_str(), word.s, word.e)).collect::<Vec<_>>() == vec![("hello", 125, 500), ("world", 550, 1125)])
        );
        assert!(
            matches!(rx.recv().await, Some(AsrEvent::LanguageDetected { code, confidence }) if code == "en" && confidence == 1.0)
        );
    }
}

#[tokio::test]
async fn request_has_pinned_parameters_optional_language_and_repeated_keywords() {
    let connector = ScriptedConnector {
        sockets: Arc::new(Mutex::new(VecDeque::from([Ok(ScriptedSocket::default())]))),
        ..Default::default()
    };
    let mut recognizer = DeepgramEngine::new(
        connector.clone(),
        "test-key",
        DeepgramOptions::new(vec!["Rust:2".into(), "Tauri:2".into()]),
    );
    let (tx, _rx) = mpsc::channel(1);
    let mut config = AsrConfig::local("unused");
    config.language = Some("es".into());
    recognizer.start(config, tx).await.unwrap();
    let request = connector.requests.lock().unwrap()[0].clone();

    assert!(request.contains("model=nova-2&encoding=linear16&sample_rate=16000&channels=1&interim_results=true&smart_format=false&punctuate=true&language=es"));
    assert!(!request.contains("detect_language=true"));
    assert!(request.contains("keywords=Rust%3A2") && request.contains("keywords=Tauri%3A2"));
}

#[tokio::test]
async fn auto_language_request_enables_detection_and_first_final_sets_language() {
    let connector = ScriptedConnector {
        sockets: Arc::new(Mutex::new(VecDeque::from([Ok(ScriptedSocket {
            inbound: VecDeque::from([Ok(FINAL.into())]),
            ..Default::default()
        })]))),
        ..Default::default()
    };
    let mut recognizer = engine(connector.clone());
    let (tx, _rx) = mpsc::channel(8);
    recognizer
        .start(AsrConfig::local("unused"), tx)
        .await
        .unwrap();
    recognizer.service().await.unwrap();
    assert!(connector.requests.lock().unwrap()[0].contains("detect_language=true"));
    assert_eq!(
        recognizer.finalize().await.unwrap().language.as_deref(),
        Some("en")
    );
}

#[tokio::test]
async fn reconnect_waits_300ms_replays_at_most_ten_seconds_and_only_retries_once() {
    let failing = ScriptedSocket {
        inbound: VecDeque::from([Err(Error::NetStream("socket dropped".into()))]),
        ..Default::default()
    };
    let replay = ScriptedSocket {
        inbound: VecDeque::from([Err(Error::NetStream("second drop".into()))]),
        ..Default::default()
    };
    let connector = ScriptedConnector {
        sockets: Arc::new(Mutex::new(VecDeque::from([Ok(failing), Ok(replay)]))),
        ..Default::default()
    };
    let mut recognizer = engine(connector.clone());
    let (tx, mut rx) = mpsc::channel(8);
    recognizer
        .start(AsrConfig::local("unused"), tx)
        .await
        .unwrap();
    recognizer.feed(&vec![0.25; 16_000 * 12]).unwrap();
    recognizer.service().await.unwrap();
    assert_eq!(recognizer.socket_state(), SocketState::ReconnectPending);
    recognizer.service().await.unwrap();
    assert_eq!(connector.requests.lock().unwrap().len(), 2);
    assert_eq!(recognizer.replayed_samples_for_test(), 160_000);
    assert!(matches!(
        rx.recv().await,
        Some(AsrEvent::Error {
            error: Error::NetStream(_)
        })
    ));
}

#[tokio::test]
async fn idle_keepalive_and_finalize_close_stream_then_drains_results() {
    let socket = ScriptedSocket {
        inbound: VecDeque::from([Ok(FINAL.into())]),
        ..Default::default()
    };
    let connector = ScriptedConnector {
        sockets: Arc::new(Mutex::new(VecDeque::from([Ok(socket)]))),
        ..Default::default()
    };
    let mut recognizer = engine(connector);
    let (tx, _rx) = mpsc::channel(8);
    recognizer
        .start(AsrConfig::local("unused"), tx)
        .await
        .unwrap();
    recognizer
        .service_after_idle(std::time::Duration::from_secs(8))
        .await
        .unwrap();
    let final_transcript = recognizer.finalize().await.unwrap();
    assert_eq!(final_transcript.text, "hello world");
    assert_eq!(
        recognizer.sent_text_for_test(),
        vec!["{\"type\":\"KeepAlive\"}", "{\"type\":\"CloseStream\"}"]
    );
}
