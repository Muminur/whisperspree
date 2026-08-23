//! Local Whisper model manifest and safe on-disk installation.

use crate::network::guard::{NetworkGuard, NetworkGuardError};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

pub const EMBEDDED_MANIFEST: &str = include_str!("../../resources/model_manifest.json");
const PART_SUFFIX: &str = ".part";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    pub id: String,
    pub received: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledModel {
    pub id: String,
    pub label: String,
    pub size_bytes: u64,
    pub installed: bool,
    pub path: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelManagerError {
    #[error("model manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("unknown model: {0}")]
    UnknownModel(String),
    #[error("model I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("refusing unsafe partial-download path: {0}")]
    UnsafePartPath(PathBuf),
    #[error("model download failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("model network policy rejected request: {0}")]
    Network(#[from] NetworkGuardError),
    #[error("checksum mismatch for {id}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        id: String,
        expected: String,
        actual: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelManifest {
    pub base_url: String,
    pub models: Vec<ModelSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelSpec {
    pub id: String,
    pub file: String,
    pub approx_bytes: u64,
    pub sha256: String,
}

impl ModelManifest {
    pub fn embedded() -> Result<Self, ModelManagerError> {
        Self::from_json(EMBEDDED_MANIFEST)
    }

    pub fn from_json(json: &str) -> Result<Self, ModelManagerError> {
        let manifest: Self = serde_json::from_str(json)
            .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))?;
        if manifest.base_url.trim().is_empty() || manifest.models.is_empty() {
            return Err(ModelManagerError::InvalidManifest(
                "base_url and models must be non-empty".to_string(),
            ));
        }
        validate_base_url(&manifest.base_url)?;
        let mut ids = HashSet::new();
        let mut files = HashSet::new();
        for model in &manifest.models {
            validate_spec(model)?;
            if !ids.insert(&model.id) || !files.insert(&model.file) {
                return Err(ModelManagerError::InvalidManifest(
                    "model IDs and destination files must be unique".to_string(),
                ));
            }
        }
        Ok(manifest)
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Result<Self, ModelManagerError> {
        let base_url = base_url.into();
        validate_base_url(&base_url)?;
        self.base_url = base_url;
        Ok(self)
    }
}

impl ModelSpec {
    pub fn download_url(&self, base_url: &str) -> Result<String, ModelManagerError> {
        let mut base = reqwest::Url::parse(base_url)
            .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))?;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        base.join(&self.file)
            .map(|url| url.to_string())
            .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))
    }
}

#[derive(Debug, Clone)]
pub struct ModelManager {
    models_dir: PathBuf,
    manifest: ModelManifest,
}

impl ModelManager {
    pub fn new(models_dir: impl Into<PathBuf>) -> Result<Self, ModelManagerError> {
        let manifest = ModelManifest::embedded()?;
        let manifest = match std::env::var("WHISPERSPREE_MODEL_BASE_URL") {
            Ok(base_url) if !base_url.trim().is_empty() => manifest.with_base_url(base_url)?,
            _ => manifest,
        };
        Ok(Self {
            models_dir: models_dir.into(),
            manifest,
        })
    }

    pub fn from_manifest_json(
        models_dir: impl Into<PathBuf>,
        json: &str,
    ) -> Result<Self, ModelManagerError> {
        Ok(Self {
            models_dir: models_dir.into(),
            manifest: ModelManifest::from_json(json)?,
        })
    }

    pub fn list_models(&self) -> Result<Vec<InstalledModel>, ModelManagerError> {
        self.manifest
            .models
            .iter()
            .map(|model| {
                let path = self.model_path(model)?;
                let installed = path.is_file();
                Ok(InstalledModel {
                    id: model.id.clone(),
                    label: label_for(&model.id),
                    size_bytes: model.approx_bytes,
                    installed,
                    path: installed.then_some(path),
                })
            })
            .collect()
    }

    /// Resolve only a verified, installed manifest entry for session startup.
    /// A model id never becomes a filesystem path directly, so settings cannot
    /// escape the manager's confined models directory.
    pub fn resolve_installed(&self, id: &str) -> Result<PathBuf, ModelManagerError> {
        let model = self.model(id)?;
        let path = self.model_path(model)?;
        if !path.is_file() {
            return Err(ModelManagerError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("model {id} is not installed"),
            )));
        }
        Ok(path)
    }

    /// Installs bytes supplied by an already-open source. Tests use this boundary
    /// with in-memory readers; production uses [`Self::download`] below.
    pub fn install_from_reader<R, F>(
        &self,
        id: &str,
        mut source: R,
        mut on_progress: F,
    ) -> Result<(), ModelManagerError>
    where
        R: Read,
        F: FnMut(DownloadProgress),
    {
        let model = self.model(id)?;
        fs::create_dir_all(&self.models_dir)?;
        let destination = self.model_path(model)?;
        let part = part_path(&destination);
        let mut file = create_new_part_file(&part)?;
        let result = (|| {
            let mut hash = Sha256::new();
            let mut received = 0_u64;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                let count = source.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                file.write_all(&buffer[..count])?;
                hash.update(&buffer[..count]);
                received += count as u64;
                on_progress(DownloadProgress {
                    id: model.id.clone(),
                    received,
                    total: model.approx_bytes,
                });
            }
            file.sync_all()?;
            let actual = format!("{:x}", hash.finalize());
            if actual != model.sha256 {
                return Err(ModelManagerError::ChecksumMismatch {
                    id: model.id.clone(),
                    expected: model.sha256.clone(),
                    actual,
                });
            }
            fs::rename(&part, destination)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&part);
        }
        result
    }

    /// Streams the verified remote model to disk. This is deliberately not used
    /// by unit tests: tests inject bytes through `install_from_reader` instead.
    pub async fn download<F>(&self, id: &str, mut on_progress: F) -> Result<(), ModelManagerError>
    where
        F: FnMut(DownloadProgress),
    {
        let model = self.model(id)?.clone();
        fs::create_dir_all(&self.models_dir)?;
        let destination = self.model_path(&model)?;
        let part = part_path(&destination);
        let mut file = create_new_part_file(&part)?;
        let result = async {
            let network = NetworkGuard::new()?;
            let response = network
                .request(&model.download_url(&self.manifest.base_url)?)?
                .send()
                .await?
                .error_for_status()?;
            let total = response.content_length().unwrap_or(model.approx_bytes);
            let mut stream = response;
            let mut hash = Sha256::new();
            let mut received = 0_u64;
            while let Some(chunk) = stream.chunk().await? {
                file.write_all(&chunk)?;
                hash.update(&chunk);
                received += chunk.len() as u64;
                on_progress(DownloadProgress {
                    id: model.id.clone(),
                    received,
                    total,
                });
            }
            file.sync_all()?;
            let actual = format!("{:x}", hash.finalize());
            if actual != model.sha256 {
                return Err(ModelManagerError::ChecksumMismatch {
                    id: model.id.clone(),
                    expected: model.sha256.clone(),
                    actual,
                });
            }
            fs::rename(&part, destination)?;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = fs::remove_file(&part);
        }
        result
    }

    pub fn delete(&self, id: &str) -> Result<(), ModelManagerError> {
        let destination = self.model_path(self.model(id)?)?;
        for path in [destination.clone(), part_path(&destination)] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Removes only an incomplete download. Cancellation must never delete a
    /// verified model that was already installed before the download started.
    pub fn remove_partial(&self, id: &str) -> Result<(), ModelManagerError> {
        let destination = self.model_path(self.model(id)?)?;
        match fs::remove_file(part_path(&destination)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn model(&self, id: &str) -> Result<&ModelSpec, ModelManagerError> {
        self.manifest
            .models
            .iter()
            .find(|model| model.id == id)
            .ok_or_else(|| ModelManagerError::UnknownModel(id.to_owned()))
    }

    fn model_path(&self, model: &ModelSpec) -> Result<PathBuf, ModelManagerError> {
        validate_spec(model)?;
        Ok(self.models_dir.join(&model.file))
    }
}

fn validate_spec(model: &ModelSpec) -> Result<(), ModelManagerError> {
    let file_path = Path::new(&model.file);
    let safe_file = file_path
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(&model.file))
        && model.file.starts_with("ggml-")
        && model.file.ends_with(".bin");
    let checksum =
        model.sha256.len() == 64 && model.sha256.bytes().all(|byte| byte.is_ascii_hexdigit());
    if model.id.is_empty() || !safe_file || !checksum || model.approx_bytes == 0 {
        return Err(ModelManagerError::InvalidManifest(format!(
            "invalid model entry {}",
            model.id
        )));
    }
    Ok(())
}

fn validate_base_url(base_url: &str) -> Result<(), ModelManagerError> {
    let url = reqwest::Url::parse(base_url)
        .map_err(|error| ModelManagerError::InvalidManifest(error.to_string()))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ModelManagerError::InvalidManifest(
            "base_url must be a credential-free HTTPS origin/path without query or fragment"
                .to_string(),
        ));
    }
    Ok(())
}

fn create_new_part_file(part: &Path) -> Result<File, ModelManagerError> {
    match fs::symlink_metadata(part) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(ModelManagerError::UnsafePartPath(part.to_path_buf()));
        }
        Ok(_) => fs::remove_file(part)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(part)?)
}

fn part_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}{}", path.display(), PART_SUFFIX))
}

fn label_for(id: &str) -> String {
    match id {
        "tiny" => "Tiny",
        "base" => "Base",
        "small" => "Small",
        "medium" => "Medium",
        "large-v3-turbo" => "Large v3 Turbo",
        other => other,
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_1_1_resolve_installed_requires_manifest_entry_and_regular_file() {
        let temp = tempfile::tempdir().unwrap();
        let manager = ModelManager::from_manifest_json(
            temp.path(),
            r#"{"base_url":"https://models.example/","models":[{"id":"tiny","file":"ggml-tiny.bin","approx_bytes":1,"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#,
        )
        .unwrap();
        assert!(matches!(
            manager.resolve_installed("tiny"),
            Err(ModelManagerError::Io(_))
        ));
        std::fs::write(temp.path().join("ggml-tiny.bin"), b"model").unwrap();
        assert_eq!(
            manager.resolve_installed("tiny").unwrap(),
            temp.path().join("ggml-tiny.bin")
        );
        assert!(matches!(
            manager.resolve_installed("../tiny"),
            Err(ModelManagerError::UnknownModel(_))
        ));
    }
    use std::{fs, io::Cursor};

    const FIXTURE_MANIFEST: &str = r#"{
      "base_url": "https://models.example.invalid/",
      "models": [
        {"id":"tiny","file":"ggml-tiny.bin","approx_bytes":4,"sha256":"03ac674216f3e15c761ee1a5e255f067953623c8b388b4459e13f978d7c846f4"}
      ]
    }"#;

    fn manager() -> (tempfile::TempDir, ModelManager) {
        let temp = tempfile::tempdir().expect("real model directory");
        let manager = ModelManager::from_manifest_json(temp.path(), FIXTURE_MANIFEST)
            .expect("fixture manifest is valid");
        (temp, manager)
    }

    #[test]
    fn model_manager_parses_manifest_and_uses_env_base_url_override() {
        let manifest = ModelManifest::from_json(FIXTURE_MANIFEST).expect("valid manifest");
        assert_eq!(manifest.models[0].id, "tiny");
        assert_eq!(
            manifest.models[0].download_url(&manifest.base_url).unwrap(),
            "https://models.example.invalid/ggml-tiny.bin"
        );
        assert_eq!(
            {
                let overridden = manifest
                    .with_base_url("https://mirror.example/models")
                    .unwrap();
                overridden.models[0]
                    .download_url(&overridden.base_url)
                    .unwrap()
            },
            "https://mirror.example/models/ggml-tiny.bin"
        );
    }

    #[test]
    fn fr_1_2_list_models_reports_manifest_entries_and_installed_path_only_after_verified_file() {
        let (temp, manager) = manager();
        let before = manager.list_models().expect("list models");
        assert_eq!(before[0].id, "tiny");
        assert!(!before[0].installed);
        assert_eq!(before[0].path, None);

        fs::write(temp.path().join("ggml-tiny.bin"), b"1234").expect("fixture model");
        let after = manager.list_models().expect("list models");
        assert!(after[0].installed);
        assert_eq!(after[0].path, Some(temp.path().join("ggml-tiny.bin")));
    }

    #[test]
    fn fr_1_2_download_streams_to_part_emits_cumulative_progress_hashes_and_atomically_promotes() {
        let (temp, manager) = manager();
        let mut progress = Vec::new();
        manager
            .install_from_reader("tiny", Cursor::new(b"1234"), |update| progress.push(update))
            .expect("correctly hashed model installs");

        assert_eq!(
            progress.last(),
            Some(&DownloadProgress {
                id: "tiny".into(),
                received: 4,
                total: 4
            })
        );
        assert_eq!(
            fs::read(temp.path().join("ggml-tiny.bin")).unwrap(),
            b"1234"
        );
        assert!(
            !temp.path().join("ggml-tiny.bin.part").exists(),
            "part must be renamed, not retained"
        );
    }

    #[test]
    fn ec_1_2_checksum_mismatch_removes_part_and_never_promotes_corrupt_model() {
        let (temp, manager) = manager();
        let error = manager
            .install_from_reader("tiny", Cursor::new(b"bad!"), |_| {})
            .expect_err("corrupt bytes must be rejected");
        assert!(matches!(error, ModelManagerError::ChecksumMismatch { .. }));
        assert!(!temp.path().join("ggml-tiny.bin.part").exists());
        assert!(!temp.path().join("ggml-tiny.bin").exists());
    }

    #[test]
    fn ec_1_2_unknown_or_unsafe_model_id_cannot_escape_models_directory() {
        let (temp, manager) = manager();
        assert!(matches!(
            manager.install_from_reader("../tiny", Cursor::new(b"1234"), |_| {}),
            Err(ModelManagerError::UnknownModel(_))
        ));
        assert!(!temp.path().parent().unwrap().join("ggml-tiny.bin").exists());
    }

    #[test]
    fn ec_1_2_existing_part_symlink_is_rejected_without_truncating_its_target() {
        use std::os::unix::fs::symlink;

        let (temp, manager) = manager();
        let victim = temp.path().join("victim.bin");
        fs::write(&victim, b"do not overwrite").unwrap();
        symlink(&victim, temp.path().join("ggml-tiny.bin.part")).unwrap();

        let error = manager
            .install_from_reader("tiny", Cursor::new(b"1234"), |_| {})
            .expect_err("a .part symlink must never be opened for truncation");
        assert!(matches!(error, ModelManagerError::UnsafePartPath(_)));
        assert_eq!(fs::read(victim).unwrap(), b"do not overwrite");
    }

    #[test]
    fn manifest_requires_https_base_url_without_credentials_or_query() {
        for base_url in [
            "http://models.example.invalid/",
            "https://user:password@models.example.invalid/",
            "https://models.example.invalid/?redirect=https://other.example/",
        ] {
            let invalid = FIXTURE_MANIFEST.replace("https://models.example.invalid/", base_url);
            assert!(
                matches!(
                    ModelManifest::from_json(&invalid),
                    Err(ModelManagerError::InvalidManifest(_))
                ),
                "must reject {base_url}"
            );
        }
    }

    #[test]
    fn fr_1_2_delete_removes_model_and_stale_part_but_rejects_unknown_id() {
        let (temp, manager) = manager();
        fs::write(temp.path().join("ggml-tiny.bin"), b"1234").unwrap();
        fs::write(temp.path().join("ggml-tiny.bin.part"), b"partial").unwrap();
        manager.delete("tiny").expect("known model deletes");
        assert!(!temp.path().join("ggml-tiny.bin").exists());
        assert!(!temp.path().join("ggml-tiny.bin.part").exists());
        assert!(matches!(
            manager.delete("other"),
            Err(ModelManagerError::UnknownModel(_))
        ));
    }

    #[test]
    fn ec_1_3_cancel_cleanup_removes_only_partial_download_and_keeps_installed_model() {
        let (temp, manager) = manager();
        let installed = temp.path().join("ggml-tiny.bin");
        let partial = temp.path().join("ggml-tiny.bin.part");
        fs::write(&installed, b"verified").unwrap();
        fs::write(&partial, b"incomplete").unwrap();

        manager
            .remove_partial("tiny")
            .expect("cancelling a download must remove the .part file");

        assert_eq!(fs::read(installed).unwrap(), b"verified");
        assert!(
            !partial.exists(),
            "cancellation cleanup must not retain a corrupt partial file"
        );
    }

    #[test]
    fn ec_1_3_preexisting_partial_symlink_is_never_followed_or_truncated() {
        use std::os::unix::fs::symlink;

        let (temp, manager) = manager();
        let victim = temp.path().join("private-data");
        fs::write(&victim, b"do not alter").unwrap();
        symlink(&victim, temp.path().join("ggml-tiny.bin.part")).unwrap();

        let error = manager
            .install_from_reader("tiny", Cursor::new(b"1234"), |_| {})
            .expect_err("an attacker-controlled partial symlink must not be opened");
        assert!(matches!(error, ModelManagerError::UnsafePartPath(_)));
        assert_eq!(fs::read(victim).unwrap(), b"do not alter");
    }

    #[test]
    fn ec_1_3_final_promotion_replaces_destination_symlink_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let (temp, manager) = manager();
        let victim = temp.path().join("private-data");
        let destination = temp.path().join("ggml-tiny.bin");
        fs::write(&victim, b"do not alter").unwrap();
        symlink(&victim, &destination).unwrap();

        manager
            .install_from_reader("tiny", Cursor::new(b"1234"), |_| {})
            .expect("promotion must replace a destination symlink itself");
        assert_eq!(fs::read(&victim).unwrap(), b"do not alter");
        assert_eq!(fs::read(destination).unwrap(), b"1234");
    }

    #[test]
    fn ec_1_3_manifest_rejects_duplicate_ids_and_filenames() {
        for duplicate in [
            r#"{"base_url":"https://models.example.invalid/","models":[
                {"id":"tiny","file":"ggml-tiny.bin","approx_bytes":4,"sha256":"03ac674216f3e15c761ee1a5e255f067953623c8b388b4459e13f978d7c846f4"},
                {"id":"tiny","file":"ggml-other.bin","approx_bytes":4,"sha256":"03ac674216f3e15c761ee1a5e255f067953623c8b388b4459e13f978d7c846f4"}
            ]}"#,
            r#"{"base_url":"https://models.example.invalid/","models":[
                {"id":"tiny","file":"ggml-tiny.bin","approx_bytes":4,"sha256":"03ac674216f3e15c761ee1a5e255f067953623c8b388b4459e13f978d7c846f4"},
                {"id":"base","file":"ggml-tiny.bin","approx_bytes":4,"sha256":"03ac674216f3e15c761ee1a5e255f067953623c8b388b4459e13f978d7c846f4"}
            ]}"#,
        ] {
            assert!(matches!(
                ModelManifest::from_json(duplicate),
                Err(ModelManagerError::InvalidManifest(_))
            ));
        }
    }

    #[test]
    fn ec_1_3_manifest_and_override_require_https_base_urls() {
        let insecure = FIXTURE_MANIFEST.replace(
            "https://models.example.invalid/",
            "http://models.example.invalid/",
        );
        assert!(matches!(
            ModelManifest::from_json(&insecure),
            Err(ModelManagerError::InvalidManifest(_))
        ));
        assert!(matches!(
            ModelManifest::from_json(FIXTURE_MANIFEST)
                .unwrap()
                .with_base_url("http://models.example.invalid/"),
            Err(ModelManagerError::InvalidManifest(_))
        ));
    }

    #[test]
    #[ignore = "requires an explicitly supplied local model; never downloads a model"]
    fn opt_in_real_model_path_is_a_regular_file() {
        let path = std::env::var_os("WHISPERSPREE_TEST_MODEL")
            .expect("set WHISPERSPREE_TEST_MODEL=/path/to/ggml-tiny.bin");
        assert!(std::path::Path::new(&path).is_file());
    }
}
