use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use reqwest::{blocking::RequestBuilder, StatusCode, Url};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use super::omnivoice::{OmniVoiceClient, OmniVoiceError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceArtifactTransport {
    pub artifacts_endpoint: String,
    pub artifact_content_endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OmniVoiceArtifact {
    pub id: String,
    pub kind: String,
    pub project_id: Option<String>,
    pub section_id: Option<String>,
    pub chunk_id: Option<String>,
    pub filename: String,
    pub relative_path: String,
    pub format: Option<String>,
    pub size_bytes: u64,
    pub duration_seconds: f64,
    pub sample_rate: u32,
    pub channels: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceArtifactDownload {
    pub bytes: u64,
    pub sha256: String,
}

pub trait OmniVoiceArtifactProvider {
    fn discover_artifact_transport(&self) -> Result<OmniVoiceArtifactTransport, OmniVoiceError>;

    fn list_artifacts(
        &self,
        transport: &OmniVoiceArtifactTransport,
        project_id: &str,
    ) -> Result<Vec<OmniVoiceArtifact>, OmniVoiceError>;

    fn download_artifact_atomic(
        &self,
        transport: &OmniVoiceArtifactTransport,
        artifact_id: &str,
        final_path: &Path,
    ) -> Result<OmniVoiceArtifactDownload, OmniVoiceError>;
}

impl OmniVoiceArtifactProvider for OmniVoiceClient {
    fn discover_artifact_transport(&self) -> Result<OmniVoiceArtifactTransport, OmniVoiceError> {
        #[derive(Deserialize)]
        struct Capabilities {
            #[serde(default)]
            features: std::collections::HashMap<String, bool>,
            #[serde(default)]
            endpoints: std::collections::HashMap<String, Value>,
        }

        let url = join_root(self.base_url(), "api/v1/capabilities")?;
        let response = authorize(self, self.http_client().get(url))
            .send()
            .map_err(map_transport)?;
        let status = response.status();
        if status != StatusCode::OK {
            return Err(classify_status(status));
        }
        let payload: Capabilities = response.json().map_err(|_| OmniVoiceError::Decode)?;
        if payload.features.get("artifact_content_download") != Some(&true) {
            return Err(OmniVoiceError::MissingCapability(
                "artifact_content_download".to_owned(),
            ));
        }
        let artifacts_endpoint = endpoint(&payload.endpoints, "artifacts")?;
        let artifact_content_endpoint = endpoint(&payload.endpoints, "artifact_content")?;
        if !artifact_content_endpoint.contains("{artifact_id}") {
            return Err(OmniVoiceError::IdentityMismatch(
                "artifact_content endpoint must contain {artifact_id}".to_owned(),
            ));
        }
        Ok(OmniVoiceArtifactTransport {
            artifacts_endpoint,
            artifact_content_endpoint,
        })
    }

    fn list_artifacts(
        &self,
        transport: &OmniVoiceArtifactTransport,
        project_id: &str,
    ) -> Result<Vec<OmniVoiceArtifact>, OmniVoiceError> {
        #[derive(Deserialize)]
        struct ArtifactList {
            items: Vec<OmniVoiceArtifact>,
        }

        let mut url = advertised_url(self.base_url(), &transport.artifacts_endpoint)?;
        url.query_pairs_mut().append_pair("project_id", project_id);
        let response = authorize(self, self.http_client().get(url))
            .send()
            .map_err(map_transport)?;
        let status = response.status();
        if status != StatusCode::OK {
            return Err(classify_status(status));
        }
        let payload: ArtifactList = response.json().map_err(|_| OmniVoiceError::Decode)?;
        Ok(payload.items)
    }

    fn download_artifact_atomic(
        &self,
        transport: &OmniVoiceArtifactTransport,
        artifact_id: &str,
        final_path: &Path,
    ) -> Result<OmniVoiceArtifactDownload, OmniVoiceError> {
        if !valid_artifact_id(artifact_id) {
            return Err(OmniVoiceError::IdentityMismatch(
                "artifact id must match art_<16 lowercase hex>".to_owned(),
            ));
        }
        let endpoint = transport
            .artifact_content_endpoint
            .replace("{artifact_id}", artifact_id);
        let url = advertised_url(self.base_url(), &endpoint)?;
        let mut response = authorize(self, self.http_client().get(url))
            .send()
            .map_err(map_transport)?;
        let status = response.status();
        if status != StatusCode::OK {
            return Err(classify_status(status));
        }
        let expected = response.content_length();
        let parent = final_path.parent().ok_or_else(|| {
            OmniVoiceError::IdentityMismatch("local artifact path has no parent".to_owned())
        })?;
        fs::create_dir_all(parent).map_err(|_| OmniVoiceError::Transport)?;
        let mut temp = NamedTempFile::new_in(parent).map_err(|_| OmniVoiceError::Transport)?;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = response
                .read(&mut buffer)
                .map_err(|_| OmniVoiceError::Transport)?;
            if read == 0 {
                break;
            }
            temp.as_file_mut()
                .write_all(&buffer[..read])
                .map_err(|_| OmniVoiceError::Transport)?;
            hasher.update(&buffer[..read]);
            total = total.saturating_add(read as u64);
        }
        if total == 0 {
            return Err(OmniVoiceError::IdentityMismatch(
                "artifact payload is empty".to_owned(),
            ));
        }
        if let Some(expected) = expected {
            if expected != total {
                return Err(OmniVoiceError::IdentityMismatch(format!(
                    "artifact content length mismatch: expected {expected}, received {total}"
                )));
            }
        }
        temp.as_file_mut()
            .sync_all()
            .map_err(|_| OmniVoiceError::Transport)?;
        temp.persist(final_path)
            .map_err(|_| OmniVoiceError::Transport)?;
        Ok(OmniVoiceArtifactDownload {
            bytes: total,
            sha256: format!("{:x}", hasher.finalize()),
        })
    }
}

fn authorize(client: &OmniVoiceClient, request: RequestBuilder) -> RequestBuilder {
    match client.bearer_token_for_request() {
        Some(token) => request.bearer_auth(token),
        None => request,
    }
}

fn join_root(base_url: &str, path: &str) -> Result<Url, OmniVoiceError> {
    let mut root = Url::parse(base_url).map_err(|_| OmniVoiceError::InvalidUrl)?;
    root.set_path("/");
    root.join(path.trim_start_matches('/'))
        .map_err(|_| OmniVoiceError::InvalidUrl)
}

fn advertised_url(base_url: &str, endpoint: &str) -> Result<Url, OmniVoiceError> {
    if !endpoint.starts_with('/') || endpoint.contains("..") {
        return Err(OmniVoiceError::IdentityMismatch(
            "advertised artifact endpoint must be an absolute service path".to_owned(),
        ));
    }
    join_root(base_url, endpoint)
}

fn endpoint(
    endpoints: &std::collections::HashMap<String, Value>,
    name: &str,
) -> Result<String, OmniVoiceError> {
    endpoints
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| OmniVoiceError::MissingEndpoint(name.to_owned()))
}

fn valid_artifact_id(value: &str) -> bool {
    value.len() == 20
        && value.starts_with("art_")
        && value[4..]
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
}

fn map_transport(error: reqwest::Error) -> OmniVoiceError {
    if error.is_timeout() {
        OmniVoiceError::Timeout
    } else {
        OmniVoiceError::Transport
    }
}

fn classify_status(status: StatusCode) -> OmniVoiceError {
    match status {
        StatusCode::UNAUTHORIZED => OmniVoiceError::Unauthorized,
        StatusCode::FORBIDDEN => OmniVoiceError::Forbidden,
        _ => OmniVoiceError::HttpStatus {
            status: status.as_u16(),
        },
    }
}

pub fn verify_local_artifact(path: &Path, expected_sha256: &str) -> Result<u64, OmniVoiceError> {
    let mut file = fs::File::open(path).map_err(|_| OmniVoiceError::Transport)?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| OmniVoiceError::Transport)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    if total == 0 || format!("{:x}", hasher.finalize()) != expected_sha256 {
        return Err(OmniVoiceError::IdentityMismatch(
            "local artifact checksum verification failed".to_owned(),
        ));
    }
    Ok(total)
}

pub fn safe_local_filename(filename: &str) -> String {
    let path = PathBuf::from(filename);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("full.wav");
    if name.is_empty() {
        "full.wav".to_owned()
    } else {
        name.to_owned()
    }
}
