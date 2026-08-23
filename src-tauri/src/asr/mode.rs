//! Deterministic cloud/local engine selection (T3.2).

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineMode {
    Local,
    Cloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeAvailability {
    pub cloud_key: bool,
    pub local_model: bool,
}

pub fn resolve_mode(requested: &str, availability: ModeAvailability) -> Result<EngineMode, Error> {
    match requested {
        "auto" => {
            if availability.cloud_key {
                Ok(EngineMode::Cloud)
            } else if availability.local_model {
                Ok(EngineMode::Local)
            } else {
                Err(Error::AsrNoModel(
                    "no cloud credentials or local model available".into(),
                ))
            }
        }
        "local" if availability.local_model => Ok(EngineMode::Local),
        "cloud" if availability.cloud_key => Ok(EngineMode::Cloud),
        "local" => Err(Error::AsrNoModel("local model is not installed".into())),
        "cloud" => Err(Error::NetStream(
            "Deepgram credentials are unavailable".into(),
        )),
        other => Err(Error::DbIo(format!("unknown ASR mode: {other}"))),
    }
}
