//! T2.5 — FR-5.4 permission probes behind the stable command surface.

mod common;

use serde_json::json;
use std::sync::Arc;
use whisperspree_lib::{
    error::Error,
    ipc::commands,
    pipeline::DictationRuntime,
    store::{keychain::KeyStore, settings::SettingsStore},
    testutil::mocks::InMemoryKeyStore,
};

struct NoopRuntime;
impl DictationRuntime for NoopRuntime {
    fn start_dictation(&self) -> Result<(), Error> {
        Ok(())
    }
    fn stop_dictation(&self) -> Result<(), Error> {
        Ok(())
    }
    fn cancel_dictation(&self) -> Result<(), Error> {
        Ok(())
    }
}

struct FixedProbe(commands::PermissionState);

impl commands::PermissionProbe for FixedProbe {
    fn snapshot(&self) -> commands::PermissionState {
        self.0
    }
}

fn harness(probe: FixedProbe) -> common::IpcHarness {
    let temp = tempfile::tempdir().unwrap();
    let state = commands::IpcState::with_runtime(
        SettingsStore::new(temp.path().to_path_buf()),
        Arc::new(InMemoryKeyStore::new()) as Arc<dyn KeyStore>,
        Arc::new(NoopRuntime),
    )
    .with_permission_probe(Arc::new(probe));
    common::IpcHarness::with_state(state)
}

#[test]
fn fr_5_4_av_authorization_status_maps_to_permission_states() {
    // AVAuthorizationStatus: notDetermined/denied/restricted/authorized.
    assert_eq!(
        commands::av_status_to_permission(0),
        commands::PermissionState::Undetermined
    );
    assert_eq!(
        commands::av_status_to_permission(1),
        commands::PermissionState::Denied
    );
    assert_eq!(
        commands::av_status_to_permission(2),
        commands::PermissionState::Denied
    );
    assert_eq!(
        commands::av_status_to_permission(3),
        commands::PermissionState::Granted
    );
    assert_eq!(
        commands::av_status_to_permission(99),
        commands::PermissionState::Undetermined
    );
}

#[test]
fn fr_5_4_check_permissions_reports_the_installed_probe() {
    let h = harness(FixedProbe(commands::PermissionState::Granted));
    let granted = h
        .invoke(commands::COMMAND_CHECK_PERMISSIONS, json!({}))
        .expect("check_permissions must succeed");
    assert_eq!(granted["accessibility"], "granted");

    let h2 = harness(FixedProbe(commands::PermissionState::Denied));
    let denied = h2
        .invoke(commands::COMMAND_CHECK_PERMISSIONS, json!({}))
        .expect("check_permissions must succeed");
    assert_eq!(denied["inputMonitoring"], "denied");
}
