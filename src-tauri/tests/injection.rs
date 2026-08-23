//! T2.2 injection strategy contracts (FR-1.4).

use std::sync::{Arc, Mutex};

use whisperspree_lib::inject::{
    pasteboard_carries_restorable_text, restore_action, ClipboardContents, ClipboardSnapshot,
    InjectContext, InjectMethod, InjectionService, InjectionSettings, PasteVerifier, RestoreAction,
    Typist,
};

#[derive(Clone, Default)]
struct TestClipboard {
    inner: Arc<Mutex<ClipboardState>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClipboardState {
    contents: ClipboardContents,
    revision: u64,
    writes: Vec<String>,
    restored: usize,
    fail_snapshot: bool,
    fail_restore: bool,
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self {
            contents: ClipboardContents::Empty,
            revision: 0,
            writes: Vec::new(),
            restored: 0,
            fail_snapshot: false,
            fail_restore: false,
        }
    }
}

impl TestClipboard {
    fn with_contents(contents: ClipboardContents) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ClipboardState {
                contents,
                ..ClipboardState::default()
            })),
        }
    }

    fn user_copies(&self, text: &str) {
        let mut state = self.inner.lock().unwrap();
        state.contents = ClipboardContents::Text {
            plain: text.into(),
            rtf: None,
        };
        state.revision += 1;
    }

    fn fail_restore(&self) {
        self.inner.lock().unwrap().fail_restore = true;
    }
}

impl whisperspree_lib::inject::Clipboard for TestClipboard {
    fn snapshot(&self) -> Result<ClipboardSnapshot, whisperspree_lib::error::Error> {
        let state = self.inner.lock().unwrap();
        if state.fail_snapshot {
            return Err(whisperspree_lib::error::Error::InjFail(
                "snapshot failed".into(),
            ));
        }
        Ok(ClipboardSnapshot {
            contents: state.contents.clone(),
            revision: state.revision,
        })
    }

    fn set_text(&self, text: &str) -> Result<(), whisperspree_lib::error::Error> {
        let mut state = self.inner.lock().unwrap();
        state.contents = ClipboardContents::Text {
            plain: text.into(),
            rtf: None,
        };
        state.revision += 1;
        state.writes.push(text.into());
        Ok(())
    }

    fn restore(&self, snapshot: &ClipboardSnapshot) -> Result<(), whisperspree_lib::error::Error> {
        let mut state = self.inner.lock().unwrap();
        if state.fail_restore {
            return Err(whisperspree_lib::error::Error::InjFail(
                "restore failed".into(),
            ));
        }
        state.contents = snapshot.contents.clone();
        state.revision += 1;
        state.restored += 1;
        Ok(())
    }
}

#[derive(Clone, Default)]
struct TestTypist {
    typed: Arc<Mutex<Vec<String>>>,
    paste_count: Arc<Mutex<usize>>,
    type_failure: Arc<Mutex<bool>>,
}

impl Typist for TestTypist {
    fn type_text(
        &self,
        text: &str,
        _inter_key_delay_ms: u64,
    ) -> Result<(), whisperspree_lib::error::Error> {
        if *self.type_failure.lock().unwrap() {
            return Err(whisperspree_lib::error::Error::InjFail(
                "type unavailable".into(),
            ));
        }
        self.typed.lock().unwrap().push(text.into());
        Ok(())
    }

    fn paste(&self) -> Result<(), whisperspree_lib::error::Error> {
        *self.paste_count.lock().unwrap() += 1;
        Ok(())
    }
}

#[derive(Clone, Default)]
struct TestVerifier(Arc<Mutex<Option<bool>>>);

impl PasteVerifier for TestVerifier {
    fn paste_succeeded(&self, _text: &str) -> Result<Option<bool>, whisperspree_lib::error::Error> {
        Ok(*self.0.lock().unwrap())
    }
}

fn normal_context() -> InjectContext {
    InjectContext::default()
}

fn service(
    clipboard: TestClipboard,
    typist: TestTypist,
    verifier: TestVerifier,
) -> InjectionService<TestClipboard, TestTypist, TestVerifier> {
    InjectionService::new(
        clipboard,
        typist,
        verifier,
        InjectionSettings {
            restore_clipboard_delay_ms: 0,
            ..InjectionSettings::default()
        },
    )
}

#[test]
fn strategy_short_text_types_without_touching_clipboard() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::Text {
        plain: "keep me".into(),
        rtf: Some("{\\rtf1 keep me}".into()),
    });
    let typist = TestTypist::default();
    let result = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject("hello Slack", &normal_context())
        .unwrap();

    assert_eq!(result, InjectMethod::Type);
    assert_eq!(*typist.typed.lock().unwrap(), vec!["hello Slack"]);
    assert_eq!(clipboard.inner.lock().unwrap().writes, Vec::<String>::new());
}

#[test]
fn strategy_exactly_threshold_chars_types() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::Text {
        plain: "original".into(),
        rtf: None,
    });
    let typist = TestTypist::default();
    let result = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject(&"x".repeat(200), &normal_context())
        .unwrap();

    assert_eq!(result, InjectMethod::Type);
    assert_eq!(*typist.typed.lock().unwrap(), vec!["x".repeat(200)]);
    assert_eq!(clipboard.inner.lock().unwrap().writes, Vec::<String>::new());
}

#[test]
fn strategy_empty_text_types_without_clipboard_touch() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::Text {
        plain: "original".into(),
        rtf: None,
    });
    let typist = TestTypist::default();
    let result = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject("", &normal_context())
        .unwrap();

    assert_eq!(result, InjectMethod::Type);
    assert_eq!(*typist.typed.lock().unwrap(), vec![String::new()]);
    assert_eq!(clipboard.inner.lock().unwrap().writes, Vec::<String>::new());
}

#[test]
fn strategy_long_text_pastes_and_restores_plain_text_and_rtf() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::Text {
        plain: "original".into(),
        rtf: Some("{\\rtf1 original}".into()),
    });
    let typist = TestTypist::default();
    let result = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject(&"x".repeat(201), &normal_context())
        .unwrap();

    assert_eq!(result, InjectMethod::Paste);
    assert_eq!(*typist.paste_count.lock().unwrap(), 1);
    let state = clipboard.inner.lock().unwrap();
    assert_eq!(
        state.contents,
        ClipboardContents::Text {
            plain: "original".into(),
            rtf: Some("{\\rtf1 original}".into())
        }
    );
    assert_eq!(state.restored, 1);
}

#[test]
fn strategy_non_text_clipboard_forces_type_and_preserves_contents() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::NonText);
    let typist = TestTypist::default();
    let result = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject(&"x".repeat(201), &normal_context())
        .unwrap();

    assert_eq!(result, InjectMethod::Type);
    assert_eq!(*typist.typed.lock().unwrap(), vec!["x".repeat(201)]);
    assert_eq!(
        clipboard.inner.lock().unwrap().contents,
        ClipboardContents::NonText
    );
}

#[test]
fn strategy_non_text_clipboard_with_unavailable_typing_returns_inj_fail_without_overwrite() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::NonText);
    let typist = TestTypist::default();
    let error = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject(
            &"x".repeat(201),
            &InjectContext {
                typing_available: false,
                ..normal_context()
            },
        )
        .unwrap_err();

    assert_eq!(error.code(), "INJ-FAIL");
    assert!(typist.typed.lock().unwrap().is_empty());
    assert_eq!(*typist.paste_count.lock().unwrap(), 0);
    let state = clipboard.inner.lock().unwrap();
    assert_eq!(state.contents, ClipboardContents::NonText);
    assert!(state.writes.is_empty());
}

#[test]
fn strategy_non_text_clipboard_with_typist_failure_returns_inj_fail_without_overwrite() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::NonText);
    let typist = TestTypist::default();
    *typist.type_failure.lock().unwrap() = true;
    let error = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject(&"x".repeat(201), &normal_context())
        .unwrap_err();

    assert_eq!(error.code(), "INJ-FAIL");
    assert!(typist.typed.lock().unwrap().is_empty());
    assert_eq!(*typist.paste_count.lock().unwrap(), 0);
    let state = clipboard.inner.lock().unwrap();
    assert_eq!(state.contents, ClipboardContents::NonText);
    assert!(state.writes.is_empty());
}

#[test]
fn strategy_secure_field_is_clipboard_only_without_key_synthesis_or_restore() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::Text {
        plain: "original".into(),
        rtf: None,
    });
    let typist = TestTypist::default();
    let result = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject(
            "secret-safe copy",
            &InjectContext {
                secure_input: true,
                ..normal_context()
            },
        )
        .unwrap();

    assert_eq!(result, InjectMethod::ClipboardOnly);
    assert!(typist.typed.lock().unwrap().is_empty());
    assert_eq!(*typist.paste_count.lock().unwrap(), 0);
    let state = clipboard.inner.lock().unwrap();
    assert_eq!(
        state.contents,
        ClipboardContents::Text {
            plain: "secret-safe copy".into(),
            rtf: None
        }
    );
    assert_eq!(state.restored, 0);
}

#[test]
fn strategy_no_target_uses_clipboard_only() {
    let clipboard = TestClipboard::default();
    let typist = TestTypist::default();
    let result = service(clipboard.clone(), typist.clone(), TestVerifier::default())
        .inject(
            "copy me",
            &InjectContext {
                target_is_valid: false,
                ..normal_context()
            },
        )
        .unwrap();

    assert_eq!(result, InjectMethod::ClipboardOnly);
    assert!(typist.typed.lock().unwrap().is_empty());
    assert_eq!(*typist.paste_count.lock().unwrap(), 0);
}

#[test]
fn strategy_failed_ax_readback_falls_back_to_clipboard_only() {
    let clipboard = TestClipboard::default();
    let typist = TestTypist::default();
    let verifier = TestVerifier(Arc::new(Mutex::new(Some(false))));
    let result = service(clipboard.clone(), typist.clone(), verifier)
        .inject(&"x".repeat(201), &normal_context())
        .unwrap();

    assert_eq!(result, InjectMethod::ClipboardOnly);
    assert_eq!(*typist.paste_count.lock().unwrap(), 1);
    assert_eq!(clipboard.inner.lock().unwrap().restored, 0);
}

#[test]
fn strategy_user_copy_during_restore_window_is_not_overwritten() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::Text {
        plain: "original".into(),
        rtf: None,
    });
    let typist = TestTypist::default();
    let injector = service(clipboard.clone(), typist, TestVerifier::default());
    let pending = injector
        .begin_inject(&"x".repeat(201), &normal_context())
        .unwrap();
    clipboard.user_copies("new user copy");
    let result = injector.finish_inject(pending).unwrap();

    assert_eq!(result, InjectMethod::Paste);
    let state = clipboard.inner.lock().unwrap();
    assert_eq!(
        state.contents,
        ClipboardContents::Text {
            plain: "new user copy".into(),
            rtf: None
        }
    );
    assert_eq!(state.restored, 0);
}

#[test]
fn ec_1_4_restore_failure_surfaces_injection_error() {
    let clipboard = TestClipboard::with_contents(ClipboardContents::Text {
        plain: "original".into(),
        rtf: None,
    });
    let injector = service(
        clipboard.clone(),
        TestTypist::default(),
        TestVerifier::default(),
    );
    let pending = injector
        .begin_inject(&"x".repeat(201), &normal_context())
        .unwrap();
    clipboard.fail_restore();
    let error = injector.finish_inject(pending).unwrap_err();
    assert_eq!(error.code(), "INJ-FAIL");
}

#[test]
fn pasteboard_types_with_utf8_plain_text_are_restorable() {
    assert!(pasteboard_carries_restorable_text(&[
        "public.rtf",
        "public.utf8-plain-text",
    ]));
    assert!(pasteboard_carries_restorable_text(&[
        "public.utf8-plain-text"
    ]));
    // The legacy NSString flavor is still a plain-UTF-8 text payload.
    assert!(pasteboard_carries_restorable_text(&["NSStringPboardType"]));
}

#[test]
fn pasteboard_types_without_utf8_take_the_non_text_guard() {
    // RTF-only, HTML-only, images, and file URLs are opaque to us: FR-1.4
    // forbids overwriting them with a synthetic paste.
    assert!(!pasteboard_carries_restorable_text(&["public.rtf"]));
    assert!(!pasteboard_carries_restorable_text(&[
        "public.html",
        "public.rtf",
    ]));
    assert!(!pasteboard_carries_restorable_text(&["public.tiff"]));
    assert!(!pasteboard_carries_restorable_text(&["public.file-url"]));
}

#[test]
fn pasteboard_empty_type_list_is_an_empty_clipboard() {
    assert!(!pasteboard_carries_restorable_text(&[]));
}

#[test]
fn restore_action_clears_an_empty_snapshot_instead_of_writing_empty_string() {
    let snapshot = ClipboardSnapshot {
        contents: ClipboardContents::Empty,
        revision: 7,
    };
    assert_eq!(restore_action(&snapshot).unwrap(), RestoreAction::Clear);
}

#[test]
fn restore_action_replays_captured_plain_and_rtf() {
    let snapshot = ClipboardSnapshot {
        contents: ClipboardContents::Text {
            plain: "original".into(),
            rtf: Some("{\\rtf1 original}".into()),
        },
        revision: 3,
    };
    assert_eq!(
        restore_action(&snapshot).unwrap(),
        RestoreAction::SetText {
            plain: "original".into(),
            rtf: Some("{\\rtf1 original}".into()),
        }
    );
}

#[test]
fn restore_action_refuses_opaque_non_text_data() {
    let snapshot = ClipboardSnapshot {
        contents: ClipboardContents::NonText,
        revision: 1,
    };
    let error = restore_action(&snapshot).unwrap_err();
    assert_eq!(error.code(), "INJ-FAIL");
}

#[test]
fn pasteboard_mixed_text_and_non_text_takes_the_guard() {
    // A board declaring both a text flavor and image/file data would let
    // restore destroy the non-text payload; FR-1.4 forbids that.
    assert!(!pasteboard_carries_restorable_text(&[
        "public.utf8-plain-text",
        "public.tiff",
    ]));
    assert!(!pasteboard_carries_restorable_text(&[
        "NSStringPboardType",
        "public.file-url",
    ]));
}
