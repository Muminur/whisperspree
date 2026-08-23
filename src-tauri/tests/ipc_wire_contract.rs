//! T0.5 — Real Rust↔frontend wire-contract tests for PRD §9.1–§9.2.
//!
//! Unlike the Tauri wrapper tests, these exercise Serde itself. The frontend
//! receives JSON, so Rust field spelling and optional-field behavior are part
//! of the public contract.

use serde_json::json;
use whisperspree_lib::{
    error::Error,
    ipc::{commands, events},
    store::keychain::Provider,
};

/// AC: PRD §8.2 / §9.2 — word timestamps are a closed, typed wire value, not
/// arbitrary JSON. Both segment and final events must carry this exact shape.
#[test]
fn fr_0_5_word_timing_round_trips_for_segment_and_final_and_rejects_malformed_json() {
    let wire = json!({ "w": "hello", "s": 120, "e": 480 });
    let timing: events::WordTiming = serde_json::from_value(wire.clone())
        .expect("PRD §8.2 WordTiming must accept {w:string,s:u32,e:u32}");
    assert_eq!(serde_json::to_value(&timing).unwrap(), wire);

    for malformed in [
        json!({ "w": "hello", "s": 120 }),
        json!({ "w": "hello", "s": "120", "e": 480 }),
        json!({ "w": "hello", "s": 120, "e": -1 }),
    ] {
        assert!(
            serde_json::from_value::<events::WordTiming>(malformed).is_err(),
            "malformed §8.2 WordTiming must be rejected"
        );
    }

    let segment = events::TranscriptSegmentPayload {
        session_id: "session-1".into(),
        text: "hello".into(),
        words: vec![timing.clone()],
    };
    let final_payload = events::TranscriptFinalPayload {
        session_id: "session-1".into(),
        raw_text: "hello world".into(),
        words: vec![timing],
        language: Some("en".into()),
    };

    assert_eq!(
        serde_json::to_value(segment).unwrap(),
        json!({ "sessionId": "session-1", "text": "hello", "words": [wire.clone()] }),
    );
    assert_eq!(
        serde_json::to_value(final_payload).unwrap(),
        json!({
            "sessionId": "session-1",
            "rawText": "hello world",
            "words": [wire],
            "language": "en",
        }),
    );
}

/// AC: PRD §9.1 / §9.2 — domains exposed to the frontend must reject unknown
/// strings instead of silently accepting a state or provider introduced by a
/// malformed client payload.
#[test]
fn fr_0_5_closed_ipc_domains_round_trip_only_prd_values() {
    for state in [
        "idle",
        "arming",
        "listening",
        "finalizing",
        "post_processing",
        "injecting",
        "cancelled",
        "error",
    ] {
        let parsed: events::SessionState = serde_json::from_value(json!(state)).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), json!(state));
    }
    assert!(serde_json::from_value::<events::SessionState>(json!("paused")).is_err());

    for permission in ["granted", "denied", "undetermined"] {
        let parsed: commands::PermissionState = serde_json::from_value(json!(permission)).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), json!(permission));
    }
    assert!(serde_json::from_value::<commands::PermissionState>(json!("prompting")).is_err());

    for kind in ["template", "persona"] {
        let parsed: commands::ReprocessKind = serde_json::from_value(json!(kind)).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), json!(kind));
    }
    assert!(serde_json::from_value::<commands::ReprocessKind>(json!("custom")).is_err());

    for (provider, wire) in [
        (Provider::Anthropic, "anthropic"),
        (Provider::Deepgram, "deepgram"),
    ] {
        assert_eq!(serde_json::to_value(provider).unwrap(), json!(wire));
        let parsed: Provider = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(parsed, provider);
    }
    assert!(serde_json::from_value::<Provider>(json!("other-provider")).is_err());
}

/// P-3 / PRD §9.2 / §14 — app:error is derived exclusively from the internal
/// error taxonomy. The client cannot select its own code, recoverability, or
/// unredacted message.
#[test]
fn p3_app_error_safe_constructor_redacts_every_error_variant_and_uses_stable_metadata() {
    let cases = [
        (
            Error::MicPerm("sk-ant-MICPERM Token mic-token".into()),
            "MIC-PERM",
            false,
        ),
        (
            Error::MicDev("sk-ant-MICDEV Token mic-token".into()),
            "MIC-DEV",
            true,
        ),
        (
            Error::HkPerm("sk-ant-HKPERM Token hotkey-token".into()),
            "HK-PERM",
            false,
        ),
        (
            Error::AxPerm("sk-ant-AXPERM Token ax-token".into()),
            "AX-PERM",
            true,
        ),
        (
            Error::AsrNoModel("sk-ant-NOMODEL Token model-token".into()),
            "ASR-NOMODEL",
            false,
        ),
        (
            Error::AsrSlow("sk-ant-SLOW Token slow-token".into()),
            "ASR-SLOW",
            true,
        ),
        (
            Error::AsrLoad("sk-ant-LOAD Token load-token".into()),
            "ASR-LOAD",
            false,
        ),
        (
            Error::NetStream("sk-ant-NET Token network-token".into()),
            "NET-STREAM",
            true,
        ),
        (
            Error::LlmAuth("sk-ant-AUTH Token auth-token".into()),
            "LLM-AUTH",
            true,
        ),
        (
            Error::LlmTimeout("sk-ant-TIMEOUT Token timeout-token".into()),
            "LLM-TIMEOUT",
            true,
        ),
        (
            Error::LlmVerify("sk-ant-VERIFY Token verify-token".into()),
            "LLM-VERIFY",
            true,
        ),
        (
            Error::InjFail("sk-ant-INJECT Token inject-token".into()),
            "INJ-FAIL",
            true,
        ),
        (
            Error::DbIo("sk-ant-DB Token db-token".into()),
            "DB-IO",
            true,
        ),
        (
            Error::SecField("sk-ant-SEC Token secure-token".into()),
            "SEC-FIELD",
            true,
        ),
    ];

    for (error, code, recoverable) in cases {
        let payload = events::AppErrorPayload::from_error(&error);
        let wire = serde_json::to_value(payload)
            .expect("P-3 app:error payload must remain serializable at the IPC boundary");
        assert!(wire["code"] == code, "§14 code must come from Error");
        assert!(
            wire["recoverable"] == recoverable,
            "§14 recoverability must come from Error"
        );
        let message_is_safe = wire["message"].as_str().is_some_and(|message| {
            !message.contains("sk-ant-")
                && !message.contains("-token")
                && message.contains("[REDACTED]")
        });
        assert!(
            message_is_safe,
            "P-3 app:error must contain a redaction marker and no secret-shaped text"
        );
    }
}

#[test]
fn fr_0_5_reprocess_options_round_trip_uses_prd_camel_case_ref_id() {
    let wire = json!({
        "id": "dictation-1",
        "kind": "template",
        "refId": "follow-up-email",
    });

    let options: commands::ReprocessOptions = serde_json::from_value(wire.clone())
        .expect("T0.5 §9.1 reprocess_dictation must accept the frontend's refId field");

    assert_eq!(
        serde_json::to_value(options).unwrap(),
        wire,
        "T0.5 §9.1 ReprocessOptions must serialize refId, not Rust ref_id"
    );
}

#[test]
fn fr_0_5_model_info_round_trips_prd_shape_and_omits_absent_path() {
    let installed = json!({
        "id": "small",
        "label": "Small",
        "sizeBytes": 466_000_000_u64,
        "installed": true,
        "path": "/Library/Application Support/WhisperSpree/models/small.bin",
    });
    let not_installed = json!({
        "id": "large-v3",
        "label": "Large v3",
        "sizeBytes": 3_100_000_000_u64,
        "installed": false,
    });

    for wire in [installed, not_installed] {
        let model: commands::ModelInfo = serde_json::from_value(wire.clone())
            .expect("T0.5 §9.1 list_models must accept {id,label,sizeBytes,installed,path?}");
        assert_eq!(
            serde_json::to_value(model).unwrap(),
            wire,
            "T0.5 §9.1 ModelInfo must preserve camelCase fields and omit a missing path"
        );
    }
}

#[test]
fn fr_0_5_permission_snapshot_round_trip_uses_input_monitoring() {
    let wire = json!({
        "microphone": "granted",
        "accessibility": "denied",
        "inputMonitoring": "undetermined",
    });

    let permissions: commands::PermissionStateSet = serde_json::from_value(wire.clone())
        .expect("T0.5 §9.1 check_permissions must accept inputMonitoring");

    assert_eq!(
        serde_json::to_value(permissions).unwrap(),
        wire,
        "T0.5 §9.1 PermissionStateSet must serialize inputMonitoring"
    );
}

#[test]
fn fr_0_5_list_dictations_query_round_trip_preserves_before_id() {
    let wire = json!({
        "q": "project update",
        "limit": 50,
        "beforeId": "dictation-previous-page",
    });

    let query: commands::ListDictationsQuery = serde_json::from_value(wire.clone())
        .expect("T0.5 §9.1 list_dictations must accept the beforeId cursor");

    assert_eq!(
        serde_json::to_value(query).unwrap(),
        wire,
        "T0.5 §9.1 ListDictationsQuery must serialize beforeId"
    );
}

#[test]
fn fr_0_5_event_payloads_serialize_exact_prd_camel_case_shapes() {
    let word = json!({ "w": "hello", "s": 120, "e": 480 });
    let timing = events::WordTiming {
        w: "hello".into(),
        s: 120,
        e: 480,
    };
    let payloads = [
        (
            "session:state",
            serde_json::to_value(events::SessionStatePayload {
                session_id: "session-1".into(),
                state: events::SessionState::Listening,
                engine: Some("local".into()),
                style_id: Some("professional".into()),
                notice: None,
            })
            .unwrap(),
            json!({
                "sessionId": "session-1",
                "state": "listening",
                "engine": "local",
                "styleId": "professional",
            }),
        ),
        (
            "transcript:partial",
            serde_json::to_value(events::TranscriptPartialPayload {
                session_id: "session-1".into(),
                text: "hello…".into(),
            })
            .unwrap(),
            json!({ "sessionId": "session-1", "text": "hello…" }),
        ),
        (
            "transcript:segment",
            serde_json::to_value(events::TranscriptSegmentPayload {
                session_id: "session-1".into(),
                text: "hello".into(),
                words: vec![timing.clone()],
            })
            .unwrap(),
            json!({ "sessionId": "session-1", "text": "hello", "words": [word.clone()] }),
        ),
        (
            "transcript:final",
            serde_json::to_value(events::TranscriptFinalPayload {
                session_id: "session-1".into(),
                raw_text: "hello world".into(),
                words: vec![timing],
                language: Some("en".into()),
            })
            .unwrap(),
            json!({
                "sessionId": "session-1",
                "rawText": "hello world",
                "words": [word.clone()],
                "language": "en",
            }),
        ),
        (
            "postprocess:done",
            serde_json::to_value(events::PostprocessDonePayload {
                session_id: "session-1".into(),
                text: "Hello, world.".into(),
                persona_id: "professional".into(),
                fallback_used: false,
                latency_ms: 321,
            })
            .unwrap(),
            json!({
                "sessionId": "session-1",
                "text": "Hello, world.",
                "personaId": "professional",
                "fallbackUsed": false,
                "latencyMs": 321,
            }),
        ),
        (
            "inject:done",
            serde_json::to_value(events::InjectDonePayload {
                session_id: "session-1".into(),
                method: "paste".into(),
            })
            .unwrap(),
            json!({ "sessionId": "session-1", "method": "paste" }),
        ),
        (
            "audio:level",
            serde_json::to_value(events::AudioLevelPayload {
                rms: 0.25,
                peak: 0.75,
            })
            .unwrap(),
            json!({ "rms": 0.25, "peak": 0.75 }),
        ),
        (
            "language:detected",
            serde_json::to_value(events::LanguageDetectedPayload {
                code: "en".into(),
                confidence: 0.5,
            })
            .unwrap(),
            json!({ "code": "en", "confidence": 0.5 }),
        ),
        (
            "model:download:progress",
            serde_json::to_value(events::ModelDownloadProgressPayload {
                id: "small".into(),
                received: 123,
                total: 456,
            })
            .unwrap(),
            json!({ "id": "small", "received": 123, "total": 456 }),
        ),
        (
            "app:error",
            serde_json::to_value(events::AppErrorPayload::from_error(&Error::NetStream(
                "connection lost".into(),
            )))
            .unwrap(),
            json!({
                "code": "NET-STREAM",
                "message": "connection lost",
                "recoverable": true,
            }),
        ),
    ];

    for (event, actual, expected) in payloads {
        assert_eq!(actual, expected, "T0.5 §9.2 {event} payload shape");
    }
}

#[test]
fn fr_0_5_session_state_omits_absent_optional_engine_and_style_id() {
    let payload = events::SessionStatePayload {
        session_id: "session-1".into(),
        state: events::SessionState::Idle,
        engine: None,
        style_id: None,
        notice: None,
    };

    assert_eq!(
        serde_json::to_value(payload).unwrap(),
        json!({
            "sessionId": "session-1",
            "state": "idle",
        }),
        "T0.5 §9.2 optional session-state fields must be absent, not null"
    );
}
