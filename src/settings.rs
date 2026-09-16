use std::{path::PathBuf, sync::Arc};

use reqwest::Url;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_DOWNLOAD_CONCURRENCY: u32 = 4;
pub const MAX_DOWNLOAD_CONCURRENCY: u32 = 32;

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
        serde_json::to_string_pretty(self).map_err(|error| SettingsError::Serialization(error.to_string()))
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
}

impl Default for RuntimeSettingsStore {
    fn default() -> Self {
        Self {
            current: Arc::new(RuntimeSettingsSnapshot::default()),
        }
    }
}

impl RuntimeSettingsStore {
    pub fn from_snapshot(snapshot: RuntimeSettingsSnapshot) -> Self {
        Self {
            current: Arc::new(snapshot),
        }
    }

    pub fn current(&self) -> Arc<RuntimeSettingsSnapshot> {
        Arc::clone(&self.current)
    }

    pub fn draft(&self) -> RuntimeSettingsDraft {
        RuntimeSettingsDraft::from_snapshot(&self.current)
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
        assert_eq!(applied.secrets.omnivoice_token.as_deref(), Some("secret-token"));
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
}
