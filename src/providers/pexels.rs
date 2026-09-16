use std::{collections::HashMap, fmt};

use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, AUTHORIZATION, RETRY_AFTER},
    StatusCode, Url,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::stock::{
    AssetKind, CreatorAttribution, RateLimitMetadata, StockAssetCandidate, StockProvider,
    StockProviderError, StockRendition, StockSearchPage, StockSearchRequest,
};

const PEXELS_PROVIDER: &str = "pexels";
const PEXELS_API_BASE: &str = "https://api.pexels.com/";

pub struct PexelsProvider {
    client: Client,
    base_url: Url,
    authorization: HeaderValue,
}

impl fmt::Debug for PexelsProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PexelsProvider")
            .field("base_url", &self.base_url)
            .field("authorization", &"[REDACTED]")
            .finish()
    }
}

impl PexelsProvider {
    pub fn new(api_key: impl AsRef<str>) -> Result<Self, StockProviderError> {
        Self::with_base_url(api_key, PEXELS_API_BASE)
    }

    fn with_base_url(api_key: impl AsRef<str>, base_url: &str) -> Result<Self, StockProviderError> {
        let key = api_key.as_ref().trim();
        if key.is_empty() {
            return Err(StockProviderError::InvalidApiKey);
        }
        let authorization =
            HeaderValue::from_str(key).map_err(|_| StockProviderError::InvalidApiKey)?;
        let base_url = Url::parse(base_url).map_err(|_| {
            StockProviderError::InvalidRequest("invalid Pexels base URL".to_owned())
        })?;
        let client = Client::builder()
            .user_agent(concat!("video-prepare/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(transport_error)?;

        Ok(Self {
            client,
            base_url,
            authorization,
        })
    }

    fn search<T: DeserializeOwned>(
        &self,
        path: &str,
        request: &StockSearchRequest,
    ) -> Result<(T, RateLimitMetadata), StockProviderError> {
        validate_request(request)?;
        let url = self.base_url.join(path).map_err(|_| {
            StockProviderError::InvalidRequest("invalid Pexels endpoint".to_owned())
        })?;
        let query = PexelsSearchQuery {
            query: request.query.trim(),
            page: request.page,
            per_page: request.per_page,
        };

        let response = self
            .client
            .get(url)
            .header(AUTHORIZATION, self.authorization.clone())
            .query(&query)
            .send()
            .map_err(transport_error)?;

        let status = response.status();
        let headers = response.headers().clone();
        if !status.is_success() {
            return Err(classify_status(status, &headers));
        }

        let rate_limit = parse_rate_limit(&headers);
        let bytes = response.bytes().map_err(transport_error)?;
        let payload =
            serde_json::from_slice(&bytes).map_err(|error| StockProviderError::Decode {
                message: error.to_string(),
            })?;
        Ok((payload, rate_limit))
    }
}

impl StockProvider for PexelsProvider {
    fn provider_name(&self) -> &'static str {
        PEXELS_PROVIDER
    }

    fn search_images(
        &self,
        request: &StockSearchRequest,
    ) -> Result<StockSearchPage, StockProviderError> {
        let (response, rate_limit): (PexelsPhotoSearchResponse, _) =
            self.search("v1/search", request)?;
        Ok(StockSearchPage {
            page: response.page,
            per_page: response.per_page,
            total_results: response.total_results,
            next_page: response.next_page,
            items: response.photos.into_iter().map(normalize_photo).collect(),
            rate_limit,
        })
    }

    fn search_videos(
        &self,
        request: &StockSearchRequest,
    ) -> Result<StockSearchPage, StockProviderError> {
        let (response, rate_limit): (PexelsVideoSearchResponse, _) =
            self.search("v1/videos/search", request)?;
        Ok(StockSearchPage {
            page: response.page,
            per_page: response.per_page,
            total_results: response.total_results,
            next_page: response.next_page,
            items: response.videos.into_iter().map(normalize_video).collect(),
            rate_limit,
        })
    }
}

fn validate_request(request: &StockSearchRequest) -> Result<(), StockProviderError> {
    if request.query.trim().is_empty() {
        return Err(StockProviderError::InvalidRequest(
            "query must not be empty".to_owned(),
        ));
    }
    if request.page == 0 {
        return Err(StockProviderError::InvalidRequest(
            "page must be at least 1".to_owned(),
        ));
    }
    if !(1..=80).contains(&request.per_page) {
        return Err(StockProviderError::InvalidRequest(
            "per_page must be between 1 and 80".to_owned(),
        ));
    }
    Ok(())
}

fn classify_status(status: StatusCode, headers: &HeaderMap) -> StockProviderError {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => StockProviderError::Authentication {
            status: status.as_u16(),
        },
        StatusCode::TOO_MANY_REQUESTS => StockProviderError::RateLimited {
            retry_after_seconds: parse_header_u64(headers, RETRY_AFTER.as_str()),
        },
        _ => StockProviderError::HttpStatus {
            status: status.as_u16(),
        },
    }
}

fn transport_error(error: reqwest::Error) -> StockProviderError {
    StockProviderError::Transport {
        message: error.without_url().to_string(),
    }
}

fn parse_rate_limit(headers: &HeaderMap) -> RateLimitMetadata {
    RateLimitMetadata {
        limit: parse_header_u64(headers, "x-ratelimit-limit"),
        remaining: parse_header_u64(headers, "x-ratelimit-remaining"),
        reset_unix: parse_header_u64(headers, "x-ratelimit-reset"),
    }
}

fn parse_header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.parse().ok()
}

#[derive(Serialize)]
struct PexelsSearchQuery<'a> {
    query: &'a str,
    page: u32,
    per_page: u8,
}

#[derive(Deserialize)]
struct PexelsPhotoSearchResponse {
    page: u32,
    per_page: u32,
    total_results: u64,
    next_page: Option<String>,
    photos: Vec<PexelsPhoto>,
}

#[derive(Deserialize)]
struct PexelsPhoto {
    id: u64,
    width: u32,
    height: u32,
    url: String,
    photographer: String,
    photographer_url: Option<String>,
    photographer_id: Option<u64>,
    #[serde(default)]
    src: HashMap<String, String>,
    alt: Option<String>,
}

#[derive(Deserialize)]
struct PexelsVideoSearchResponse {
    page: u32,
    per_page: u32,
    total_results: u64,
    next_page: Option<String>,
    videos: Vec<PexelsVideo>,
}

#[derive(Deserialize)]
struct PexelsVideo {
    id: u64,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<u32>,
    url: String,
    image: Option<String>,
    user: PexelsUser,
    #[serde(default)]
    video_files: Vec<PexelsVideoFile>,
}

#[derive(Deserialize)]
struct PexelsUser {
    id: Option<u64>,
    name: String,
    url: Option<String>,
}

#[derive(Deserialize)]
struct PexelsVideoFile {
    id: u64,
    quality: Option<String>,
    file_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f32>,
    link: String,
}

fn normalize_photo(photo: PexelsPhoto) -> StockAssetCandidate {
    let preview_url = photo
        .src
        .get("medium")
        .or_else(|| photo.src.get("landscape"))
        .or_else(|| photo.src.get("original"))
        .cloned();
    let mut renditions: Vec<StockRendition> = photo
        .src
        .into_iter()
        .map(|(label, url)| StockRendition {
            label,
            url,
            mime_type: None,
            width: None,
            height: None,
            fps: None,
        })
        .collect();
    renditions.sort_by(|left, right| left.label.cmp(&right.label));
    if let Some(original) = renditions.iter_mut().find(|item| item.label == "original") {
        original.width = Some(photo.width);
        original.height = Some(photo.height);
    }

    StockAssetCandidate {
        provider: PEXELS_PROVIDER,
        provider_asset_id: photo.id.to_string(),
        kind: AssetKind::Image,
        source_url: photo.url,
        creator: CreatorAttribution {
            provider_creator_id: photo.photographer_id.map(|value| value.to_string()),
            name: photo.photographer,
            profile_url: photo.photographer_url,
        },
        width: Some(photo.width),
        height: Some(photo.height),
        duration_seconds: None,
        preview_url,
        alt_text: photo.alt,
        renditions,
    }
}

fn normalize_video(video: PexelsVideo) -> StockAssetCandidate {
    let renditions = video
        .video_files
        .into_iter()
        .map(|file| StockRendition {
            label: file
                .quality
                .clone()
                .map(|quality| format!("{}-{quality}", file.id))
                .unwrap_or_else(|| file.id.to_string()),
            url: file.link,
            mime_type: file.file_type,
            width: file.width,
            height: file.height,
            fps: file.fps,
        })
        .collect();

    StockAssetCandidate {
        provider: PEXELS_PROVIDER,
        provider_asset_id: video.id.to_string(),
        kind: AssetKind::Video,
        source_url: video.url,
        creator: CreatorAttribution {
            provider_creator_id: video.user.id.map(|value| value.to_string()),
            name: video.user.name,
            profile_url: video.user.url,
        },
        width: video.width,
        height: video.height,
        duration_seconds: video.duration,
        preview_url: video.image,
        alt_text: None,
        renditions,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc::{self, Receiver},
        thread,
        time::Duration,
    };

    use super::*;

    fn serve_once(
        status: u16,
        response_headers: &[(&str, &str)],
        body: &str,
    ) -> (String, Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_owned();
        let response_headers: Vec<(String, String)> = response_headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = sender.send(String::from_utf8_lossy(&request).to_string());

            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                403 => "Forbidden",
                429 => "Too Many Requests",
                _ => "Error",
            };
            let mut response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (name, value) in response_headers {
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str("\r\n");
            response.push_str(&body);
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        (format!("http://{address}/"), receiver)
    }

    fn provider_for(base_url: &str) -> PexelsProvider {
        PexelsProvider::with_base_url("secret-key", base_url).unwrap()
    }

    #[test]
    fn photo_search_uses_current_endpoint_auth_and_query_encoding() {
        let body = r#"{
          "page": 1,
          "per_page": 2,
          "total_results": 1,
          "photos": [{
            "id": 123,
            "width": 1920,
            "height": 1080,
            "url": "https://www.pexels.com/photo/demo-123/",
            "photographer": "Alice",
            "photographer_url": "https://www.pexels.com/@alice",
            "photographer_id": 7,
            "src": {
              "original": "https://images.pexels.com/photos/123/original.jpeg",
              "medium": "https://images.pexels.com/photos/123/medium.jpeg"
            },
            "alt": "Calm person near a window"
          }]
        }"#;
        let (base_url, request_rx) = serve_once(
            200,
            &[
                ("X-Ratelimit-Limit", "20000"),
                ("X-Ratelimit-Remaining", "19999"),
                ("X-Ratelimit-Reset", "1900000000"),
            ],
            body,
        );
        let provider = provider_for(&base_url);
        let request = StockSearchRequest::new("calm man & window", 1, 2).unwrap();
        let result = provider.search_images(&request).unwrap();

        let raw_request = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let lower = raw_request.to_ascii_lowercase();
        assert!(raw_request.starts_with("GET /v1/search?"));
        assert!(raw_request.contains("query=calm+man+%26+window"));
        assert!(raw_request.contains("page=1"));
        assert!(raw_request.contains("per_page=2"));
        assert!(lower.contains("authorization: secret-key"));

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].provider_asset_id, "123");
        assert_eq!(result.items[0].kind, AssetKind::Image);
        assert_eq!(result.items[0].creator.name, "Alice");
        assert_eq!(result.items[0].renditions.len(), 2);
        assert_eq!(result.rate_limit.limit, Some(20000));
        assert_eq!(result.rate_limit.remaining, Some(19999));
        assert_eq!(result.rate_limit.reset_unix, Some(1900000000));
    }

    #[test]
    fn video_search_uses_v1_videos_path_and_normalizes_files() {
        let body = r#"{
          "page": 1,
          "per_page": 1,
          "total_results": 1,
          "videos": [{
            "id": 99,
            "width": 1920,
            "height": 1080,
            "duration": 12,
            "url": "https://www.pexels.com/video/demo-99/",
            "image": "https://images.pexels.com/videos/99/preview.jpeg",
            "user": {"id": 8, "name": "Bob", "url": "https://www.pexels.com/@bob"},
            "video_files": [{
              "id": 501,
              "quality": "hd",
              "file_type": "video/mp4",
              "width": 1920,
              "height": 1080,
              "fps": 29.97,
              "link": "https://videos.pexels.com/video-files/99/501.mp4"
            }]
          }]
        }"#;
        let (base_url, request_rx) = serve_once(200, &[], body);
        let provider = provider_for(&base_url);
        let request = StockSearchRequest::new("city", 1, 1).unwrap();
        let result = provider.search_videos(&request).unwrap();

        let raw_request = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(raw_request.starts_with("GET /v1/videos/search?"));
        assert_eq!(result.items[0].kind, AssetKind::Video);
        assert_eq!(result.items[0].duration_seconds, Some(12));
        assert_eq!(result.items[0].creator.name, "Bob");
        assert_eq!(
            result.items[0].renditions[0].mime_type.as_deref(),
            Some("video/mp4")
        );
        assert_eq!(result.items[0].renditions[0].fps, Some(29.97));
    }

    #[test]
    fn authentication_and_rate_limit_errors_are_typed() {
        let (base_url, _) = serve_once(401, &[], "{}");
        let provider = provider_for(&base_url);
        let request = StockSearchRequest::new("nature", 1, 1).unwrap();
        assert_eq!(
            provider.search_images(&request).unwrap_err(),
            StockProviderError::Authentication { status: 401 }
        );

        let (base_url, _) = serve_once(429, &[("Retry-After", "60")], "{}");
        let provider = provider_for(&base_url);
        assert_eq!(
            provider.search_images(&request).unwrap_err(),
            StockProviderError::RateLimited {
                retry_after_seconds: Some(60)
            }
        );
    }

    #[test]
    fn malformed_success_json_is_decode_error() {
        let (base_url, _) = serve_once(200, &[], "{not-json");
        let provider = provider_for(&base_url);
        let request = StockSearchRequest::new("nature", 1, 1).unwrap();
        assert!(matches!(
            provider.search_images(&request),
            Err(StockProviderError::Decode { .. })
        ));
    }

    #[test]
    fn api_key_is_redacted_from_debug_and_errors() {
        let provider =
            PexelsProvider::with_base_url("super-secret-key", "http://127.0.0.1:1/").unwrap();
        assert!(!format!("{provider:?}").contains("super-secret-key"));

        let request = StockSearchRequest::new("nature", 1, 1).unwrap();
        let error = provider.search_images(&request).unwrap_err();
        assert!(!error.to_string().contains("super-secret-key"));
    }

    #[test]
    fn rejects_invalid_search_parameters_before_network() {
        assert!(StockSearchRequest::new("", 1, 1).is_err());
        assert!(StockSearchRequest::new("nature", 0, 1).is_err());
        assert!(StockSearchRequest::new("nature", 1, 0).is_err());
        assert!(StockSearchRequest::new("nature", 1, 81).is_err());
        assert!(PexelsProvider::new(" ").is_err());
    }
}
