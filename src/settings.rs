use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_DOWNLOAD_CONCURRENCY: u32 = 4;
pub const MAX_DOWNLOAD_CONCURRENCY: u32 = 32;
pub const SETTINGS_FILE_NAME: &str = "preferences.json";
pub const SETTINGS_DIR_NAME: &str = "video-prepare";
pub const SETTINGS_DIR_OVERRIDE_ENV: &str = "VIDEO_PREPARE_CONFIG_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QualityPreset {
    Safe,
    Balanced,
    Fast,
}

impl QualityPreset {
    pub const ALL: [Self; 3] = [Self::Safe, Self::Balanced, Self::Fast];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "SAFE",
            Self::Balanced => "BALANCED",
            Self::Fast => "FAST",
        }
    }
}

impl Default for QualityPreset {
    fn default() -> Self {
        Self::Balanced
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafePreferences {
    pub data_root: PathBuf,
    pub visual_flow_enabled: bool,
    pub audio_flow_enabled: bool,
    pub omnivoice_url: String,
    pub voice_name: String,
    pub voice_variant: String,
    pub language: String,
    pub quality_preset: QualityPreset,
    pub read_section_titles: bool,
    pub download_concurrency: u32,
}

impl Default for SafePreferences {
    fn default() -> Self {
        Self {
            data_root: PathBuf::from("video-prepare-data"),
            visual_flow_enabled: false,
            audio_flow_enabled: false,
            omnivoice_url: String::new(),
            voice_name: "Narrator".to_owned(),
            voice_variant: "AUTO".to_owned(),
            language: "en".to_owned(),
            quality_preset: QualityPreset::Balanced,
            read_section_titles: false,
            download_concurrency: DEFAULT_DOWNLOAD_CONCURRENCY,
        }
    }
}

impl SafePreferences {
    pub fn to_json_pretty(&self) -> Result<String, SettingsError> {
        serde_json::to_string_pretty(self)
            .map_err(|error| SettingsError::Serialization(error.to_string()))
    }

    pub fn from_json(input: &str) -> Result<Self, SettingsError> {
        serde_json::from_str(input).map_err(|error| SettingsError::Serialization(error.to_string()))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeSecrets {
    pub pexels_api_key: Option<String>,
    pub omnivoice_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSettingsSnapshot {
    pub revision: u64,
    pub safe: SafePreferences,
    pub secrets: RuntimeSecrets,
}

impl Default for RuntimeSettingsSnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            safe: SafePreferences::default(),
            secrets: RuntimeSecrets::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSettingsDraft {
    pub data_root: String,
    pub visual_flow_enabled: bool,
    pub audio_flow_enabled: bool,
    pub pexels_api_key: String,
    pub omnivoice_url: String,
    pub omnivoice_token: String,
    pub voice_name: String,
    pub voice_variant: String,
    pub language: String,
    pub quality_preset: QualityPreset,
    pub read_section_titles: bool,
    pub download_concurrency: u32,
}

impl RuntimeSettingsDraft {
    pub fn from_snapshot(snapshot: &RuntimeSettingsSnapshot) -> Self {
        Self {
            data_root: snapshot.safe.data_root.to_string_lossy().into_owned(),
            visual_flow_enabled: snapshot.safe.visual_flow_enabled,
            audio_flow_enabled: snapshot.safe.audio_flow_enabled,
            pexels_api_key: snapshot.secrets.pexels_api_key.clone().unwrap_or_default(),
            omnivoice_url: snapshot.safe.omnivoice_url.clone(),
            omnivoice_token: snapshot.secrets.omnivoice_token.clone().unwrap_or_default(),
            voice_name: snapshot.safe.voice_name.clone(),
            voice_variant: snapshot.safe.voice_variant.clone(),
            language: snapshot.safe.language.clone(),
            quality_preset: snapshot.safe.quality_preset,
            read_section_titles: snapshot.safe.read_section_titles,
            download_concurrency: snapshot.safe.download_concurrency,
        }
    }

    fn validated(&self) -> Result<(SafePreferences, RuntimeSecrets), SettingsError> {
        let data_root = self.data_root.trim();
        if data_root.is_empty() {
            return Err(SettingsError::EmptyDataRoot);
        }
        if !(1..=MAX_DOWNLOAD_CONCURRENCY).contains(&self.download_concurrency) {
            return Err(SettingsError::InvalidDownloadConcurrency {
                value: self.download_concurrency,
                max: MAX_DOWNLOAD_CONCURRENCY,
            });
        }

        let omnivoice_url = normalize_omnivoice_url(&self.omnivoice_url, self.audio_flow_enabled)?;
        let voice_name = non_empty_or_default(&self.voice_name, "Narrator");
        let voice_variant = non_empty_or_default(&self.voice_variant, "AUTO");
        let language = non_empty_or_default(&self.language, "en");

        let safe = SafePreferences {
            data_root: PathBuf::from(data_root),
            visual_flow_enabled: self.visual_flow_enabled,
            audio_flow_enabled: self.audio_flow_enabled,
            omnivoice_url,
            voice_name,
            voice_variant,
            language,
            quality_preset: self.quality_preset,
            read_section_titles: self.read_section_titles,
            download_concurrency: self.download_concurrency,
        };
        let secrets = RuntimeSecrets {
            pexels_api_key: secret(&self.pexels_api_key),
            omnivoice_token: secret(&self.omnivoice_token),
        };
        Ok((safe, secrets))
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeSettingsStore {
    current: Arc<RuntimeSettingsSnapshot>,
    persistence_path: Option<PathBuf>,
}

impl Default for RuntimeSettingsStore {
    fn default() -> Self {
        Self {
            current: Arc::new(RuntimeSettingsSnapshot::default()),
            persistence_path: None,
        }
    }
}

impl RuntimeSettingsStore {
    pub fn from_snapshot(snapshot: RuntimeSettingsSnapshot) -> Self {
        Self {
            current: Arc::new(snapshot),
            persistence_path: None,
        }
    }

    pub fn with_persistence_path(path: impl Into<PathBuf>) -> Self {
        Self {
            current: Arc::new(RuntimeSettingsSnapshot::default()),
            persistence_path: Some(path.into()),
        }
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, SettingsError> {
        let path = path.as_ref().to_path_buf();
        let safe = match fs::read_to_string(&path) {
            Ok(raw) => {
                let parsed: SafePreferences = serde_json::from_str(&raw).map_err(|error| {
                    SettingsError::PersistenceDecode {
                        path: path.clone(),
                        message: error.to_string(),
                    }
                })?;
                validate_loaded_safe_preferences(parsed)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => SafePreferences::default(),
            Err(error) => {
                return Err(SettingsError::PersistenceRead {
                    path,
                    message: error.to_string(),
                });
            }
        };

        Ok(Self {
            current: Arc::new(RuntimeSettingsSnapshot {
                revision: 0,
                safe,
                secrets: RuntimeSecrets::default(),
            }),
            persistence_path: Some(path),
        })
    }

    pub fn load_persistent_or_default() -> (Self, Option<SettingsError>) {
        match default_preferences_path() {
            Ok(path) => match Self::load_from_path(&path) {
                Ok(store) => (store, None),
                Err(error) => (Self::with_persistence_path(path), Some(error)),
            },
            Err(error) => (Self::default(), Some(error)),
        }
    }

    pub fn current(&self) -> Arc<RuntimeSettingsSnapshot> {
        Arc::clone(&self.current)
    }

    pub fn draft(&self) -> RuntimeSettingsDraft {
        RuntimeSettingsDraft::from_snapshot(&self.current)
    }

    pub fn persistence_path(&self) -> Option<&Path> {
        self.persistence_path.as_deref()
    }

    pub fn apply(
        &mut self,
        draft: &RuntimeSettingsDraft,
    ) -> Result<Arc<RuntimeSettingsSnapshot>, SettingsError> {
        let (safe, secrets) = draft.validated()?;
        let revision = self
            .current
            .revision
            .checked_add(1)
            .ok_or(SettingsError::RevisionOverflow)?;

        if let Some(path) = &self.persistence_path {
            persist_safe_preferences(path, &safe)?;
        }

        let next = Arc::new(RuntimeSettingsSnapshot {
            revision,
            safe,
            secrets,
        });
        self.current = Arc::clone(&next);
        Ok(next)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SettingsError {
    #[error("Data Root must not be empty")]
    EmptyDataRoot,

    #[error("download concurrency must be between 1 and {max}, got {value}")]
    InvalidDownloadConcurrency { value: u32, max: u32 },

    #[error("OmniVoice URL is required while Audio Flow is enabled")]
    MissingOmniVoiceUrl,

    #[error("OmniVoice URL is invalid")]
    InvalidOmniVoiceUrl,

    #[error("OmniVoice URL scheme must be http or https, got `{0}`")]
    UnsupportedOmniVoiceScheme(String),

    #[error("OmniVoice URL must not contain embedded credentials")]
    EmbeddedOmniVoiceCredentials,

    #[error("OmniVoice URL must be a service root without path, query, or fragment")]
    InvalidOmniVoiceBasePath,

    #[error("settings revision overflow")]
    RevisionOverflow,

    #[error("safe preference serialization failed: {0}")]
    Serialization(String),

    #[error("failed to resolve settings location: {0}")]
    PersistenceLocation(String),

    #[error("failed to read persisted settings at {path}: {message}")]
    PersistenceRead { path: PathBuf, message: String },

    #[error("failed to decode persisted settings at {path}: {message}")]
    PersistenceDecode { path: PathBuf, message: String },

    #[error("failed to write persisted settings at {path}: {message}")]
    PersistenceWrite { path: PathBuf, message: String },
}

pub fn default_preferences_path() -> Result<PathBuf, SettingsError> {
    if let Some(path) = env::var_os(SETTINGS_DIR_OVERRIDE_ENV).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path).join(SETTINGS_FILE_NAME));
    }

    #[cfg(target_os = "windows")]
    if let Some(path) = env::var_os("APPDATA").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path)
            .join(SETTINGS_DIR_NAME)
            .join(SETTINGS_FILE_NAME));
    }

    #[cfg(target_os = "macos")]
    if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join(SETTINGS_DIR_NAME)
            .join(SETTINGS_FILE_NAME));
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(path) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(path)
                .join(SETTINGS_DIR_NAME)
                .join(SETTINGS_FILE_NAME));
        }
        if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(home)
                .join(".config")
                .join(SETTINGS_DIR_NAME)
                .join(SETTINGS_FILE_NAME));
        }
    }

    env::current_dir()
        .map(|path| path.join(format!(".{SETTINGS_DIR_NAME}")).join(SETTINGS_FILE_NAME))
        .map_err(|error| SettingsError::PersistenceLocation(error.to_string()))
}

pub fn normalize_omnivoice_url(input: &str, required: bool) -> Result<String, SettingsError> {
    let raw = input.trim();
    if raw.is_empty() {
        return if required {
            Err(SettingsError::MissingOmniVoiceUrl)
        } else {
            Ok(String::new())
        };
    }

    let mut url = Url::parse(raw).map_err(|_| SettingsError::InvalidOmniVoiceUrl)?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(SettingsError::UnsupportedOmniVoiceScheme(other.to_owned())),
    }
    if url.host_str().is_none() {
        return Err(SettingsError::InvalidOmniVoiceUrl);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SettingsError::EmbeddedOmniVoiceCredentials);
    }
    if url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
        return Err(SettingsError::InvalidOmniVoiceBasePath);
    }
    url.set_path("/");
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn validate_loaded_safe_preferences(
    mut safe: SafePreferences,
) -> Result<SafePreferences, SettingsError> {
    if safe.data_root.as_os_str().is_empty() {
        return Err(SettingsError::EmptyDataRoot);
    }
    if !(1..=MAX_DOWNLOAD_CONCURRENCY).contains(&safe.download_concurrency) {
        return Err(SettingsError::InvalidDownloadConcurrency {
            value: safe.download_concurrency,
            max: MAX_DOWNLOAD_CONCURRENCY,
        });
    }
    safe.omnivoice_url = normalize_omnivoice_url(&safe.omnivoice_url, safe.audio_flow_enabled)?;
    safe.voice_name = non_empty_or_default(&safe.voice_name, "Narrator");
    safe.voice_variant = non_empty_or_default(&safe.voice_variant, "AUTO");
    safe.language = non_empty_or_default(&safe.language, "en");
    Ok(safe)
}

fn persist_safe_preferences(path: &Path, safe: &SafePreferences) -> Result<(), SettingsError> {
    let json = safe.to_json_pretty()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SettingsError::PersistenceWrite {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    fs::write(path, format!("{json}\n")).map_err(|error| SettingsError::PersistenceWrite {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn secret(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn non_empty_or_default(value: &str, default: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        default.to_owned()
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative_and_do_not_require_network_services() {
        let store = RuntimeSettingsStore::default();
        let current = store.current();
        assert_eq!(current.revision, 0);
        assert!(!current.safe.visual_flow_enabled);
        assert!(!current.safe.audio_flow_enabled);
        assert_eq!(current.safe.download_concurrency, 4);
        assert!(current.secrets.pexels_api_key.is_none());
        assert!(current.secrets.omnivoice_token.is_none());
        assert!(store.persistence_path().is_none());
    }

    #[test]
    fn apply_normalizes_url_and_replaces_snapshot_without_mutating_old_snapshot() {
        let mut store = RuntimeSettingsStore::default();
        let old = store.current();
        let mut draft = store.draft();
        draft.audio_flow_enabled = true;
        draft.omnivoice_url = " https://studio.example/ ".to_owned();
        draft.omnivoice_token = " secret-token ".to_owned();

        let applied = store.apply(&draft).unwrap();
        assert_eq!(applied.revision, 1);
        assert_eq!(applied.safe.omnivoice_url, "https://studio.example");
        assert_eq!(
            applied.secrets.omnivoice_token.as_deref(),
            Some("secret-token")
        );
        assert_eq!(old.revision, 0);
        assert!(old.safe.omnivoice_url.is_empty());
    }

    #[test]
    fn cancel_semantics_are_draft_from_last_applied_snapshot() {
        let mut store = RuntimeSettingsStore::default();
        let mut draft = store.draft();
        draft.data_root = "/tmp/data-a".to_owned();
        store.apply(&draft).unwrap();

        let mut edited = store.draft();
        edited.data_root = "/tmp/not-applied".to_owned();
        let cancelled = store.draft();
        assert_eq!(cancelled.data_root, "/tmp/data-a");
        assert_ne!(cancelled.data_root, edited.data_root);
        assert_eq!(store.current().revision, 1);
    }

    #[test]
    fn invalid_url_shapes_are_rejected() {
        assert_eq!(
            normalize_omnivoice_url("ftp://studio.example", true),
            Err(SettingsError::UnsupportedOmniVoiceScheme("ftp".to_owned()))
        );
        assert_eq!(
            normalize_omnivoice_url("https://user:pass@studio.example", true),
            Err(SettingsError::EmbeddedOmniVoiceCredentials)
        );
        assert_eq!(
            normalize_omnivoice_url("https://studio.example/api", true),
            Err(SettingsError::InvalidOmniVoiceBasePath)
        );
    }

    #[test]
    fn invalid_concurrency_is_rejected() {
        let mut store = RuntimeSettingsStore::default();
        let mut draft = store.draft();
        draft.download_concurrency = 0;
        assert_eq!(
            store.apply(&draft).unwrap_err(),
            SettingsError::InvalidDownloadConcurrency {
                value: 0,
                max: MAX_DOWNLOAD_CONCURRENCY,
            }
        );
    }

    #[test]
    fn safe_preference_json_never_contains_runtime_secrets() {
        let mut store = RuntimeSettingsStore::default();
        let mut draft = store.draft();
        draft.pexels_api_key = "pexels-secret-123".to_owned();
        draft.omnivoice_token = "omnivoice-secret-456".to_owned();
        let applied = store.apply(&draft).unwrap();
        let json = applied.safe.to_json_pretty().unwrap();
        assert!(!json.contains("pexels-secret-123"));
        assert!(!json.contains("omnivoice-secret-456"));
        assert!(!json.to_ascii_lowercase().contains("token"));
        assert!(!json.to_ascii_lowercase().contains("api_key"));
        let round_trip = SafePreferences::from_json(&json).unwrap();
        assert_eq!(round_trip, applied.safe);
    }

    #[test]
    fn persistent_store_reloads_safe_preferences_but_not_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config/preferences.json");
        let mut store = RuntimeSettingsStore::with_persistence_path(&path);
        let mut draft = store.draft();
        draft.data_root = temp.path().join("portable-data").display().to_string();
        draft.visual_flow_enabled = true;
        draft.audio_flow_enabled = true;
        draft.pexels_api_key = "pexels-secret".to_owned();
        draft.omnivoice_url = "https://studio.example/".to_owned();
        draft.omnivoice_token = "omnivoice-secret".to_owned();
        draft.voice_name = "Calm Narrator".to_owned();
        draft.voice_variant = "WARM".to_owned();
        draft.language = "vi".to_owned();
        draft.quality_preset = QualityPreset::Safe;
        draft.read_section_titles = true;
        draft.download_concurrency = 7;

        let applied = store.apply(&draft).unwrap();
        assert!(path.is_file());
        let persisted = fs::read_to_string(&path).unwrap();
        assert!(!persisted.contains("pexels-secret"));
        assert!(!persisted.contains("omnivoice-secret"));

        let reopened = RuntimeSettingsStore::load_from_path(&path).unwrap();
        let current = reopened.current();
        assert_eq!(current.revision, 0);
        assert_eq!(current.safe, applied.safe);
        assert!(current.secrets.pexels_api_key.is_none());
        assert!(current.secrets.omnivoice_token.is_none());
        assert_eq!(reopened.persistence_path(), Some(path.as_path()));
    }

    #[test]
    fn missing_persisted_file_starts_with_defaults_and_is_ready_to_save() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let store = RuntimeSettingsStore::load_from_path(&path).unwrap();

        assert_eq!(store.current().safe, SafePreferences::default());
        assert_eq!(store.persistence_path(), Some(path.as_path()));
    }

    #[test]
    fn persistence_failure_does_not_replace_the_active_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let blocked_parent = temp.path().join("not-a-directory");
        fs::write(&blocked_parent, "file").unwrap();
        let path = blocked_parent.join("preferences.json");
        let mut store = RuntimeSettingsStore::with_persistence_path(path);
        let before = store.current();
        let mut draft = store.draft();
        draft.visual_flow_enabled = true;

        let error = store.apply(&draft).unwrap_err();
        assert!(matches!(error, SettingsError::PersistenceWrite { .. }));
        assert_eq!(store.current().as_ref(), before.as_ref());
    }

    #[test]
    fn serde_defaults_allow_future_safe_preferences_to_add_fields_without_breaking_old_files() {
        let parsed = SafePreferences::from_json(r#"{"data_root":"custom-root"}"#).unwrap();
        assert_eq!(parsed.data_root, PathBuf::from("custom-root"));
        assert_eq!(parsed.voice_name, "Narrator");
        assert_eq!(parsed.download_concurrency, DEFAULT_DOWNLOAD_CONCURRENCY);
    }
}
