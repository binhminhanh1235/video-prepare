use thiserror::Error;

use crate::{
    OmniVoiceClient, OmniVoiceConnection, PexelsProvider, ProviderConnection, RateLimitMetadata,
    StockProvider,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionTestTarget {
    Pexels,
    OmniVoice,
}

impl ConnectionTestTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pexels => "Pexels",
            Self::OmniVoice => "OmniVoice",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PexelsConnectionReport {
    pub provider: String,
    pub rate_limit: RateLimitMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceConnectionReport {
    pub base_url: String,
    pub service: Option<String>,
    pub project_import_endpoint: String,
    pub generate_project_endpoint: String,
    pub jobs_endpoint: Option<String>,
    pub artifact_content_download: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionTestReport {
    Pexels(PexelsConnectionReport),
    OmniVoice(OmniVoiceConnectionReport),
}

impl ConnectionTestReport {
    pub fn target(&self) -> ConnectionTestTarget {
        match self {
            Self::Pexels(_) => ConnectionTestTarget::Pexels,
            Self::OmniVoice(_) => ConnectionTestTarget::OmniVoice,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConnectionTestError {
    #[error("Pexels connection test failed: {0}")]
    Pexels(String),

    #[error("OmniVoice connection test failed: {0}")]
    OmniVoice(String),
}

pub fn test_pexels_connection(api_key: &str) -> Result<ConnectionTestReport, ConnectionTestError> {
    let provider = PexelsProvider::new(api_key)
        .map_err(|error| ConnectionTestError::Pexels(error.to_string()))?;
    let connection = provider
        .test_connection()
        .map_err(|error| ConnectionTestError::Pexels(error.to_string()))?;
    Ok(ConnectionTestReport::Pexels(pexels_report(connection)))
}

pub fn test_omnivoice_connection(
    base_url: &str,
    token: Option<String>,
) -> Result<ConnectionTestReport, ConnectionTestError> {
    let client = OmniVoiceClient::new(base_url, token)
        .map_err(|error| ConnectionTestError::OmniVoice(error.to_string()))?;
    let connection = client
        .test_connection()
        .map_err(|error| ConnectionTestError::OmniVoice(error.to_string()))?;
    Ok(ConnectionTestReport::OmniVoice(omnivoice_report(connection)))
}

fn pexels_report(connection: ProviderConnection) -> PexelsConnectionReport {
    PexelsConnectionReport {
        provider: connection.provider.to_owned(),
        rate_limit: connection.rate_limit,
    }
}

fn omnivoice_report(connection: OmniVoiceConnection) -> OmniVoiceConnectionReport {
    OmniVoiceConnectionReport {
        base_url: connection.base_url,
        service: connection.service,
        project_import_endpoint: connection.project_import_endpoint,
        generate_project_endpoint: connection.generate_project_endpoint,
        jobs_endpoint: connection.jobs_endpoint,
        artifact_content_download: connection.artifact_content_download,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_pexels_key_fails_without_secret_echo() {
        let error = test_pexels_connection("").unwrap_err();
        assert!(matches!(error, ConnectionTestError::Pexels(_)));
        assert!(!error.to_string().contains("Authorization"));
    }

    #[test]
    fn invalid_omnivoice_url_fails_locally_without_token_echo() {
        let token = "super-secret-token".to_owned();
        let error = test_omnivoice_connection("not a url", Some(token.clone())).unwrap_err();
        let message = error.to_string();
        assert!(matches!(error, ConnectionTestError::OmniVoice(_)));
        assert!(!message.contains(&token));
    }

    #[test]
    fn pexels_success_report_contains_only_safe_metadata() {
        let report = pexels_report(ProviderConnection {
            provider: "pexels",
            rate_limit: RateLimitMetadata {
                limit: Some(200),
                remaining: Some(199),
                reset_unix: Some(1234567890),
            },
        });
        assert_eq!(report.provider, "pexels");
        assert_eq!(report.rate_limit.remaining, Some(199));
    }

    #[test]
    fn omnivoice_success_report_contains_capability_summary_only() {
        let report = omnivoice_report(OmniVoiceConnection {
            base_url: "https://voice.example".to_owned(),
            service: Some("omnivoice-studio".to_owned()),
            project_import_endpoint: "/api/v1/projects/import".to_owned(),
            generate_project_endpoint: "/api/v1/projects/{project_id}/generate".to_owned(),
            jobs_endpoint: Some("/api/v1/jobs".to_owned()),
            artifact_content_endpoint: Some("/api/v1/artifacts/{artifact_id}/content".to_owned()),
            artifact_content_download: true,
        });
        assert_eq!(report.base_url, "https://voice.example");
        assert_eq!(report.service.as_deref(), Some("omnivoice-studio"));
        assert!(report.artifact_content_download);
        let debug = format!("{report:?}");
        assert!(!debug.to_ascii_lowercase().contains("token"));
        assert!(!debug.to_ascii_lowercase().contains("api_key"));
    }
}
