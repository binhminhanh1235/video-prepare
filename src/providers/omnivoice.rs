use std::{collections::HashMap, fmt, time::Duration};

use reqwest::{
    blocking::{Client, RequestBuilder, Response},
    StatusCode, Url,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct OmniVoiceClient {
    base_url: Url,
    normalized_base_url: String,
    bearer_token: Option<String>,
    client: Client,
}

impl fmt::Debug for OmniVoiceClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OmniVoiceClient")
            .field("base_url", &self.normalized_base_url)
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "<redacted>"),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceConnection {
    pub base_url: String,
    pub service: Option<String>,
    pub project_import_endpoint: String,
    pub generate_project_endpoint: String,
    pub jobs_endpoint: Option<String>,
    pub artifact_content_endpoint: Option<String>,
    pub artifact_content_download: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceImportResult {
    pub project_id: String,
    pub source_hash: String,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateProjectOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sections: Option<Vec<String>>,
    pub resume: bool,
    pub strict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_preset: Option<String>,
}

impl Default for GenerateProjectOptions {
    fn default() -> Self {
        Self {
            voice_name: None,
            voice_variant: None,
            language: None,
            sections: None,
            resume: true,
            strict: false,
            quality_preset: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceJobSubmission {
    pub job_id: String,
    pub status: String,
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceRemoteJob {
    pub job_id: String,
    pub status: String,
    pub kind: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OmniVoiceError {
    #[error("OmniVoice base URL is empty or malformed")]
    InvalidUrl,

    #[error("OmniVoice base URL scheme `{0}` is not supported")]
    UnsupportedScheme(String),

    #[error("OmniVoice base URL must not contain embedded credentials")]
    EmbeddedCredentials,

    #[error(
        "OmniVoice base URL must point to the service root without query, fragment or extra path"
    )]
    InvalidBasePath,

    #[error("OmniVoice request timed out")]
    Timeout,

    #[error("OmniVoice transport error")]
    Transport,

    #[error("OmniVoice authentication failed")]
    Unauthorized,

    #[error("OmniVoice request is forbidden")]
    Forbidden,

    #[error("OmniVoice project import conflicts with an existing remote project")]
    ProjectConflict,

    #[error("OmniVoice returned HTTP {status}")]
    HttpStatus { status: u16 },

    #[error("OmniVoice returned an invalid JSON response")]
    Decode,

    #[error("OmniVoice is missing required capability `{0}`")]
    MissingCapability(String),

    #[error("OmniVoice capability endpoint `{0}` is missing")]
    MissingEndpoint(String),

    #[error("OmniVoice response identity is inconsistent: {0}")]
    IdentityMismatch(String),
}

impl OmniVoiceError {
    pub fn is_ambiguous_remote_submit(&self) -> bool {
        matches!(self, Self::Timeout | Self::Transport)
    }
}

impl OmniVoiceClient {
    pub fn new(
        base_url: impl AsRef<str>,
        bearer_token: Option<String>,
    ) -> Result<Self, OmniVoiceError> {
        Self::with_timeout(base_url, bearer_token, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(
        base_url: impl AsRef<str>,
        bearer_token: Option<String>,
        timeout: Duration,
    ) -> Result<Self, OmniVoiceError> {
        let raw = base_url.as_ref().trim();
        if raw.is_empty() {
            return Err(OmniVoiceError::InvalidUrl);
        }

        let mut parsed = Url::parse(raw).map_err(|_| OmniVoiceError::InvalidUrl)?;
        match parsed.scheme() {
            "http" | "https" => {}
            other => return Err(OmniVoiceError::UnsupportedScheme(other.to_owned())),
        }
        if parsed.host_str().is_none() {
            return Err(OmniVoiceError::InvalidUrl);
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(OmniVoiceError::EmbeddedCredentials);
        }
        if parsed.query().is_some()
            || parsed.fragment().is_some()
            || !matches!(parsed.path(), "" | "/")
        {
            return Err(OmniVoiceError::InvalidBasePath);
        }

        parsed.set_path("/");
        let normalized_base_url = parsed.as_str().trim_end_matches('/').to_owned();
        let bearer_token = bearer_token.and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        });
        let client = Client::builder()
            .user_agent(concat!("video-prepare/", env!("CARGO_PKG_VERSION")))
            .timeout(timeout)
            .build()
            .map_err(|_| OmniVoiceError::Transport)?;

        Ok(Self {
            base_url: parsed,
            normalized_base_url,
            bearer_token,
            client,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.normalized_base_url
    }

    pub fn test_connection(&self) -> Result<OmniVoiceConnection, OmniVoiceError> {
        let health: Value = self.get_json("health")?;
        let capabilities: CapabilitiesResponse = self.get_json("api/v1/capabilities")?;

        if capabilities.features.get("project_import") != Some(&true) {
            return Err(OmniVoiceError::MissingCapability(
                "project_import".to_owned(),
            ));
        }

        let project_import_endpoint = endpoint(&capabilities.endpoints, "project_import")?;
        let generate_project_endpoint = endpoint(&capabilities.endpoints, "generate_project")?;
        let artifact_content_endpoint =
            optional_endpoint(&capabilities.endpoints, "artifact_content");
        let artifact_content_download = capabilities
            .features
            .get("artifact_content_download")
            .copied()
            .unwrap_or(false)
            && artifact_content_endpoint.is_some();

        let service = health
            .get("service")
            .and_then(Value::as_str)
            .or_else(|| health.get("name").and_then(Value::as_str))
            .map(str::to_owned);

        Ok(OmniVoiceConnection {
            base_url: self.normalized_base_url.clone(),
            service,
            project_import_endpoint,
            generate_project_endpoint,
            jobs_endpoint: optional_endpoint(&capabilities.endpoints, "jobs"),
            artifact_content_endpoint,
            artifact_content_download,
        })
    }

    pub fn import_project(
        &self,
        project_id: &str,
        script: &str,
        speak_section_titles: bool,
        max_chunk_words: u32,
        max_chunk_chars: u32,
    ) -> Result<OmniVoiceImportResult, OmniVoiceError> {
        #[derive(Serialize)]
        struct ImportRequest<'a> {
            project_id: &'a str,
            script: &'a str,
            speak_section_titles: bool,
            max_chunk_words: u32,
            max_chunk_chars: u32,
        }

        #[derive(Deserialize)]
        struct ImportResponse {
            project_id: String,
            source_hash: String,
            created: bool,
        }

        let endpoint = self.url("api/v1/projects/import")?;
        let request = ImportRequest {
            project_id,
            script,
            speak_section_titles,
            max_chunk_words,
            max_chunk_chars,
        };
        let response = self
            .auth(self.client.post(endpoint))
            .json(&request)
            .send()
            .map_err(map_transport)?;
        let payload: ImportResponse =
            self.decode(response, &[StatusCode::OK, StatusCode::CREATED])?;
        Ok(OmniVoiceImportResult {
            project_id: payload.project_id,
            source_hash: payload.source_hash,
            created: payload.created,
        })
    }

    pub fn generate_project(
        &self,
        project_id: &str,
        options: &GenerateProjectOptions,
        idempotency_key: &str,
    ) -> Result<OmniVoiceJobSubmission, OmniVoiceError> {
        #[derive(Deserialize)]
        struct GenerateResponse {
            job_id: String,
            status: String,
            location: Option<String>,
        }

        let endpoint = self.url(&format!("api/v1/projects/{project_id}/generate"))?;
        let response = self
            .auth(self.client.post(endpoint))
            .header("Idempotency-Key", idempotency_key)
            .json(options)
            .send()
            .map_err(map_transport)?;
        let payload: GenerateResponse = self.decode(response, &[StatusCode::ACCEPTED])?;
        Ok(OmniVoiceJobSubmission {
            job_id: payload.job_id,
            status: payload.status,
            location: payload.location,
        })
    }

    pub fn get_job(&self, job_id: &str) -> Result<OmniVoiceRemoteJob, OmniVoiceError> {
        #[derive(Deserialize)]
        struct JobResponse {
            id: Option<String>,
            job_id: Option<String>,
            status: String,
            kind: Option<String>,
        }

        let payload: JobResponse = self.get_json(&format!("api/v1/jobs/{job_id}"))?;
        let remote_id = payload
            .id
            .or(payload.job_id)
            .unwrap_or_else(|| job_id.to_owned());
        Ok(OmniVoiceRemoteJob {
            job_id: remote_id,
            status: payload.status,
            kind: payload.kind,
        })
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        match self.bearer_token.as_deref() {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    fn url(&self, path: &str) -> Result<Url, OmniVoiceError> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .map_err(|_| OmniVoiceError::InvalidUrl)
    }

    fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, OmniVoiceError> {
        let response = self
            .auth(self.client.get(self.url(path)?))
            .send()
            .map_err(map_transport)?;
        self.decode(response, &[StatusCode::OK])
    }

    fn decode<T: DeserializeOwned>(
        &self,
        response: Response,
        expected: &[StatusCode],
    ) -> Result<T, OmniVoiceError> {
        let status = response.status();
        if !expected.contains(&status) {
            return Err(classify_status(status));
        }
        response.json().map_err(|_| OmniVoiceError::Decode)
    }
}

#[derive(Debug, Deserialize)]
struct CapabilitiesResponse {
    #[serde(default)]
    features: HashMap<String, bool>,
    #[serde(default)]
    endpoints: HashMap<String, Value>,
}

fn endpoint(endpoints: &HashMap<String, Value>, name: &str) -> Result<String, OmniVoiceError> {
    optional_endpoint(endpoints, name)
        .ok_or_else(|| OmniVoiceError::MissingEndpoint(name.to_owned()))
}

fn optional_endpoint(endpoints: &HashMap<String, Value>, name: &str) -> Option<String> {
    endpoints
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
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
        StatusCode::CONFLICT => OmniVoiceError::ProjectConflict,
        _ => OmniVoiceError::HttpStatus {
            status: status.as_u16(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, Mutex},
        thread,
    };

    #[test]
    fn normalizes_root_url_and_rejects_unsafe_forms() {
        let client = OmniVoiceClient::new(" https://example.com/ ", None).unwrap();
        assert_eq!(client.base_url(), "https://example.com");
        assert!(matches!(
            OmniVoiceClient::new("ftp://example.com", None),
            Err(OmniVoiceError::UnsupportedScheme(_))
        ));
        assert_eq!(
            OmniVoiceClient::new("https://user:pass@example.com", None).unwrap_err(),
            OmniVoiceError::EmbeddedCredentials
        );
        assert_eq!(
            OmniVoiceClient::new("https://example.com/api", None).unwrap_err(),
            OmniVoiceError::InvalidBasePath
        );
        assert_eq!(
            OmniVoiceClient::new("https://example.com/?x=1", None).unwrap_err(),
            OmniVoiceError::InvalidBasePath
        );
    }

    #[test]
    fn debug_redacts_bearer_token() {
        let client =
            OmniVoiceClient::new("https://example.com", Some("super-secret-token".to_owned()))
                .unwrap();
        let rendered = format!("{client:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("super-secret-token"));
    }

    #[test]
    fn connection_import_and_generate_follow_advertised_contract() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let responses = vec![
            json_response(200, r#"{"service":"omnivoice-studio"}"#),
            json_response(
                200,
                r#"{"features":{"project_import":true,"artifact_content_download":true},"endpoints":{"project_import":"/api/v1/projects/import","generate_project":"/api/v1/projects/{project_id}/generate","jobs":"/api/v1/jobs","artifact_content":"/api/v1/artifacts/{artifact_id}/content"}}"#,
            ),
            json_response(
                201,
                r#"{"project_id":"vp-demo-deadbeef","source_hash":"abc123","created":true}"#,
            ),
            json_response(
                202,
                r#"{"job_id":"job-1","status":"queued","location":"/api/v1/jobs/job-1"}"#,
            ),
            json_response(
                200,
                r#"{"id":"job-1","status":"running","kind":"generate_project"}"#,
            ),
        ];
        let base = spawn_server(responses, requests.clone());
        let client = OmniVoiceClient::new(base, Some("token-123".to_owned())).unwrap();

        let connection = client.test_connection().unwrap();
        assert!(connection.artifact_content_download);
        assert_eq!(
            connection.artifact_content_endpoint.as_deref(),
            Some("/api/v1/artifacts/{artifact_id}/content")
        );

        let script = "# Demo\n\n## S01 — 0:00–0:05\n[WARM] Keep this exact.\n";
        let imported = client
            .import_project("vp-demo-deadbeef", script, false, 24, 220)
            .unwrap();
        assert!(imported.created);
        let submitted = client
            .generate_project(
                "vp-demo-deadbeef",
                &GenerateProjectOptions::default(),
                "stable-key-1",
            )
            .unwrap();
        assert_eq!(submitted.job_id, "job-1");
        assert_eq!(client.get_job("job-1").unwrap().status, "running");

        let captured = requests.lock().unwrap();
        assert_eq!(captured.len(), 5);
        let import = &captured[2];
        assert_eq!(import.method, "POST");
        assert_eq!(import.path, "/api/v1/projects/import");
        assert_eq!(import.authorization.as_deref(), Some("Bearer token-123"));
        let body: Value = serde_json::from_slice(&import.body).unwrap();
        assert_eq!(body["script"].as_str(), Some(script));

        let generate = &captured[3];
        assert_eq!(generate.idempotency_key.as_deref(), Some("stable-key-1"));
    }

    #[test]
    fn connection_rejects_missing_project_import_capability() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let responses = vec![
            json_response(200, "{}"),
            json_response(200, r#"{"features":{},"endpoints":{}}"#),
        ];
        let base = spawn_server(responses, requests);
        let client = OmniVoiceClient::new(base, None).unwrap();
        assert_eq!(
            client.test_connection().unwrap_err(),
            OmniVoiceError::MissingCapability("project_import".to_owned())
        );
    }

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        authorization: Option<String>,
        idempotency_key: Option<String>,
        body: Vec<u8>,
    }

    fn spawn_server(responses: Vec<Vec<u8>>, requests: Arc<Mutex<Vec<CapturedRequest>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let captured = read_request(&mut stream);
                requests.lock().unwrap().push(captured);
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
            }
        });
        format!("http://{address}")
    }

    fn read_request(stream: &mut TcpStream) -> CapturedRequest {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end;
        loop {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0);
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(index) = find_bytes(&buffer, b"\r\n\r\n") {
                header_end = index + 4;
                break;
            }
        }
        let header = String::from_utf8_lossy(&buffer[..header_end]);
        let mut lines = header.lines();
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_owned();
        let path = request_parts.next().unwrap().to_owned();
        let mut content_length = 0_usize;
        let mut authorization = None;
        let mut idempotency_key = None;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => content_length = value.trim().parse().unwrap(),
                "authorization" => authorization = Some(value.trim().to_owned()),
                "idempotency-key" => idempotency_key = Some(value.trim().to_owned()),
                _ => {}
            }
        }
        while buffer.len() < header_end + content_length {
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0);
            buffer.extend_from_slice(&chunk[..read]);
        }
        CapturedRequest {
            method,
            path,
            authorization,
            idempotency_key,
            body: buffer[header_end..header_end + content_length].to_vec(),
        }
    }

    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    fn json_response(status: u16, body: &str) -> Vec<u8> {
        let reason = match status {
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            _ => "Error",
        };
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.as_bytes().len()
        )
        .into_bytes()
    }
}
