//! T0.5 — production `AppHandle<MockRuntime>` event-emission regression test.
//!
//! This intentionally does not use the `EventSink` fake. It proves the
//! production `AppHandle` implementation routes a typed §9.2 payload through
//! Tauri's listener machinery without needing a native window or permission.

use std::{sync::mpsc, time::Duration};

use serde_json::json;
use tauri::{
    test::{mock_builder, mock_context, noop_assets},
    Listener,
};
use whisperspree_lib::ipc::events;

#[test]
fn fr_0_5_app_handle_emit_reaches_mockruntime_listener_with_exact_wire_payload() {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("MockRuntime app must build for the §9.2 event route test");
    let handle = app.handle().clone();
    let (sender, receiver) = mpsc::sync_channel(1);

    let listener = handle.listen(events::EVENT_SESSION_STATE, move |event| {
        sender
            .send(event.payload().to_owned())
            .expect("bounded listener channel remains open for one event");
    });

    events::emit_session_state(
        &handle,
        events::SessionStatePayload {
            session_id: "session-1".into(),
            state: events::SessionState::Listening,
            engine: Some("local".into()),
            style_id: None,
            notice: None,
        },
    )
    .expect("AppHandle event emission must succeed in MockRuntime");

    let received = receiver
        .recv_timeout(Duration::from_millis(250))
        .expect("AppHandle listener must receive session:state before the bounded timeout");
    handle.unlisten(listener);

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&received)
            .expect("Tauri listener payload must be JSON"),
        json!({
            "sessionId": "session-1",
            "state": "listening",
            "engine": "local",
        }),
        "§9.2 production AppHandle route must preserve the exact camelCase payload and omit styleId"
    );
}
