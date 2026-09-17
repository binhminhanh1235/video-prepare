pub mod audio;
pub mod audio_artifacts;
pub mod connection_tests;
pub mod desktop;
pub mod diagnostics;
pub mod inspection;
pub mod manual_visual;
pub mod observability;
pub mod orchestration;
pub mod project;
pub mod project_catalog;
pub mod providers;
pub mod reconciliation;
pub mod remote_reconciliation;
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
pub use connection_tests::{
    test_omnivoice_connection, test_pexels_connection, ConnectionTestError, ConnectionTestReport,
    ConnectionTestTarget, OmniVoiceConnectionReport, PexelsConnectionReport,
};
pub use desktop::{run_desktop, VideoPrepareApp};
pub use diagnostics::{
    append_run_log_event, read_run_log, run_log_path, Diagnostic, DiagnosticCategory, RunLogError,
    RunLogEvent, RUN_LOG_RELATIVE_PATH, RUN_LOG_SCHEMA_VERSION,
};
pub use inspection::{
    inspect_project, AudioAttemptInspection, AudioInspection, InspectionProblem, ProjectInspection,
    SceneInspection, VisualRequestInspection,
};
pub use manual_visual::{
    ManualVisualError, ManualVisualImportSummary, MANUAL_VISUAL_PROVENANCE_SCHEMA_VERSION,
};
pub use observability::{
    execute_flow_retry, execute_project_run, import_manual_visual_asset, reconcile_remote_audio,
};
pub use orchestration::{
    plan_audio_next_step, AudioNextStep, FlowRetryReport, FlowRunDisposition, FlowRunReport,
    FlowTarget, ProjectRunReport, RunAction,
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
pub use reconciliation::{
    reconcile_local_project, LocalReconciliationError, LocalReconciliationReport,
};
pub use remote_reconciliation::{
    RemoteAudioDisposition, RemoteAudioReconciliationError, RemoteAudioReconciliationReport,
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
