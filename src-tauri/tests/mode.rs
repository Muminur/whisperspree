use whisperspree_lib::asr::mode::{resolve_mode, EngineMode, ModeAvailability};

#[test]
fn auto_prefers_cloud_when_key_and_stream_are_available() {
    assert_eq!(
        resolve_mode(
            "auto",
            ModeAvailability {
                cloud_key: true,
                local_model: true
            }
        )
        .unwrap(),
        EngineMode::Cloud
    );
}

#[test]
fn auto_falls_back_to_local_without_cloud_key() {
    assert_eq!(
        resolve_mode(
            "auto",
            ModeAvailability {
                cloud_key: false,
                local_model: true
            }
        )
        .unwrap(),
        EngineMode::Local
    );
}

#[test]
fn explicit_modes_are_respected_and_unavailable_modes_error() {
    assert_eq!(
        resolve_mode(
            "local",
            ModeAvailability {
                cloud_key: true,
                local_model: true
            }
        )
        .unwrap(),
        EngineMode::Local
    );
    assert_eq!(
        resolve_mode(
            "cloud",
            ModeAvailability {
                cloud_key: true,
                local_model: false
            }
        )
        .unwrap(),
        EngineMode::Cloud
    );
    assert!(resolve_mode(
        "local",
        ModeAvailability {
            cloud_key: true,
            local_model: false
        }
    )
    .is_err());
}
