use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Image,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockSearchRequest {
    pub query: String,
    pub page: u32,
    pub per_page: u8,
}

impl StockSearchRequest {
    pub fn new(
        query: impl Into<String>,
        page: u32,
        per_page: u8,
    ) -> Result<Self, StockProviderError> {
        let query = query.into();
        if query.trim().is_empty() {
            return Err(StockProviderError::InvalidRequest(
                "query must not be empty".to_owned(),
            ));
        }
        if page == 0 {
            return Err(StockProviderError::InvalidRequest(
                "page must be at least 1".to_owned(),
            ));
        }
        if !(1..=80).contains(&per_page) {
            return Err(StockProviderError::InvalidRequest(
                "per_page must be between 1 and 80".to_owned(),
            ));
        }

        Ok(Self {
            query,
            page,
            per_page,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatorAttribution {
    pub provider_creator_id: Option<String>,
    pub name: String,
    pub profile_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StockRendition {
    pub label: String,
    pub url: String,
    pub mime_type: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StockAssetCandidate {
    pub provider: &'static str,
    pub provider_asset_id: String,
    pub kind: AssetKind,
    pub source_url: String,
    pub creator: CreatorAttribution,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_seconds: Option<u32>,
    pub preview_url: Option<String>,
    pub alt_text: Option<String>,
    pub renditions: Vec<StockRendition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RateLimitMetadata {
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub reset_unix: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StockSearchPage {
    pub page: u32,
    pub per_page: u32,
    pub total_results: u64,
    pub next_page: Option<String>,
    pub items: Vec<StockAssetCandidate>,
    pub rate_limit: RateLimitMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConnection {
    pub provider: &'static str,
    pub rate_limit: RateLimitMetadata,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StockProviderError {
    #[error("invalid stock-provider request: {0}")]
    InvalidRequest(String),

    #[error("stock-provider API key is empty or has an invalid header format")]
    InvalidApiKey,

    #[error("stock-provider authentication failed with HTTP {status}")]
    Authentication { status: u16 },

    #[error("stock-provider rate limit exceeded")]
    RateLimited { retry_after_seconds: Option<u64> },

    #[error("stock-provider returned HTTP {status}")]
    HttpStatus { status: u16 },

    #[error("stock-provider transport error: {message}")]
    Transport { message: String },

    #[error("stock-provider response decode error: {message}")]
    Decode { message: String },
}

pub trait StockProvider {
    fn provider_name(&self) -> &'static str;

    fn search_images(
        &self,
        request: &StockSearchRequest,
    ) -> Result<StockSearchPage, StockProviderError>;

    fn search_videos(
        &self,
        request: &StockSearchRequest,
    ) -> Result<StockSearchPage, StockProviderError>;

    fn test_connection(&self) -> Result<ProviderConnection, StockProviderError> {
        let request = StockSearchRequest::new("nature", 1, 1)?;
        let page = self.search_images(&request)?;
        Ok(ProviderConnection {
            provider: self.provider_name(),
            rate_limit: page.rate_limit,
        })
    }
}
