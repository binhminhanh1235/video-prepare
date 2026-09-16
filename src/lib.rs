pub mod audio;
pub mod audio_artifacts;
pub mod desktop;
pub mod inspection;
pub mod project;
pub mod project_catalog;
pub mod providers;
pub mod script;
pub mod settings;
pub mod visual;

pub use audio::{
    audio_status_path, deterministic_remote_project_id, load_audio_status, omnivoice_source_hash,
    AudioAttemptStatus, AudioError, AudioExecutor, AudioFlowStatus, AudioGenerationSettings,
    AudioRunSummary, AUDIO_STATUS_SCHEMA_VERSION,
};
pub use audio_artifacts::{
    audio_artifact_proof_path, load_audio_artifact_proof, reconcile_local_audio_artifact,
    sync_latest_audio_artifact, AudioArtifactError, AudioArtifactProof, AudioArtifactSyncSummary,
    AUDIO_ARTIFACT_SCHEMA_VERSION,
};
pub use desktop::{run_desktop, VideoPrepareApp};
pub use inspection::{
    inspect_project, AudioAttemptInspection, AudioInspection, InspectionProblem, ProjectInspection,
    SceneInspection, VisualRequestInspection,
};
pub use project::{
    ProjectError, ProjectMetadata, ProjectStatus, ProjectStore, SceneRuntimeStatus, StoredProject,
    TaskState, PROJECT_SCHEMA_VERSION, STATUS_SCHEMA_VERSION,
};
pub use project_catalog::{
    create_project_from_script_path, discover_projects, open_project_from_data_root,
    ProjectActionError, ProjectCatalog, ProjectCatalogError, ProjectDiscoveryError, ProjectSummary,
};
pub use providers::{
    AssetKind, CreatorAttribution, GenerateProjectOptions, OmniVoiceArtifact,
    OmniVoiceArtifactDownload, OmniVoiceArtifactProvider, OmniVoiceArtifactTransport,
    OmniVoiceClient, OmniVoiceConnection, OmniVoiceError, OmniVoiceImportResult,
    OmniVoiceJobSubmission, OmniVoiceProvider, OmniVoiceRemoteJob, PexelsProvider,
    ProviderConnection, RateLimitMetadata, StockAssetCandidate, StockProvider, StockProviderError,
    StockRendition, StockSearchPage, StockSearchRequest,
};
pub use script::{
    parse_script, MediaKind, OmniVoiceScript, OmniVoiceSection, PreparedScript, SceneSpec,
    ScriptError, VisualRequest,
};
pub use settings::{
    normalize_omnivoice_url, QualityPreset, RuntimeSecrets, RuntimeSettingsDraft,
    RuntimeSettingsSnapshot, RuntimeSettingsStore, SafePreferences, SettingsError,
    DEFAULT_DOWNLOAD_CONCURRENCY, MAX_DOWNLOAD_CONCURRENCY,
};
pub use visual::{
    build_query_plan, load_visual_status, visual_status_path, AssetDownloadError, AssetDownloader,
    DownloadReceipt, HttpAssetDownloader, PersistedAssetKind, ProvenanceCreator,
    ProvenanceRendition, VisualAssetProvenance, VisualAssetStatus, VisualError, VisualExecutor,
    VisualFlowStatus, VisualRequestStatus, VisualRunSummary, VisualSceneStatus,
    PROVENANCE_SCHEMA_VERSION, VISUAL_STATUS_SCHEMA_VERSION,
};
