use std::collections::HashSet;

use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const SCENES_MARKER: &str = "--- SCENES ---";
pub const OMNIVOICE_MARKER: &str = "--- OMNIVOICE ---";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedScript {
    pub format_version: u32,
    pub scenes: Vec<SceneSpec>,
    pub omnivoice: OmniVoiceScript,
    pub input_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneSpec {
    pub id: String,
    pub visuals: Vec<VisualRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Either,
}

impl MediaKind {
    fn parse(value: &str, scene_id: &str, visual_id: &str) -> Result<Self, ScriptError> {
        match value {
            "image" => Ok(Self::Image),
            "video" => Ok(Self::Video),
            "either" => Ok(Self::Either),
            other => Err(ScriptError::InvalidMedia {
                scene_id: scene_id.to_owned(),
                visual_id: visual_id.to_owned(),
                media: other.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualRequest {
    pub id: String,
    pub media: MediaKind,
    pub queries: Vec<String>,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceScript {
    /// Exact content after the `--- OMNIVOICE ---` marker line.
    pub raw_markdown: String,
    pub title: String,
    pub sections: Vec<OmniVoiceSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVoiceSection {
    pub id: String,
    pub start_time: String,
    pub end_time: String,
    pub start_seconds: u64,
    pub end_seconds: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScriptError {
    #[error("expected exactly one `{marker}` marker, found {found}")]
    MarkerCount { marker: &'static str, found: usize },

    #[error("`--- SCENES ---` must appear before `--- OMNIVOICE ---`")]
    MarkerOrder,

    #[error("only whitespace is allowed before `--- SCENES ---`")]
    ContentBeforeScenes,

    #[error("SCENES YAML is empty")]
    EmptyScenesPayload,

    #[error("OMNIVOICE Markdown is empty")]
    EmptyOmniVoicePayload,

    #[error("invalid SCENES YAML: {message}")]
    InvalidScenesYaml { message: String },

    #[error("format_version must be 1, found {found}")]
    UnsupportedFormatVersion { found: u32 },

    #[error("at least one scene is required")]
    EmptyScenes,

    #[error("scene id `{id}` must match `S` followed by at least two digits")]
    InvalidSceneId { id: String },

    #[error("duplicate scene id `{id}`")]
    DuplicateSceneId { id: String },

    #[error("scene `{scene_id}` requires at least one visual request")]
    EmptyVisuals { scene_id: String },

    #[error("visual id `{scene_id}/{visual_id}` must match `V` followed by at least two digits")]
    InvalidVisualId { scene_id: String, visual_id: String },

    #[error("duplicate visual id `{scene_id}/{visual_id}`")]
    DuplicateVisualId { scene_id: String, visual_id: String },

    #[error("{scene_id}/{visual_id} media `{media}` must be image, video, or either")]
    InvalidMedia {
        scene_id: String,
        visual_id: String,
        media: String,
    },

    #[error("{scene_id}/{visual_id} requires at least one non-empty query")]
    EmptyQuery { scene_id: String, visual_id: String },

    #[error("{scene_id}/{visual_id} count must be at least 1, found {count}")]
    InvalidCount {
        scene_id: String,
        visual_id: String,
        count: i64,
    },

    #[error("OMNIVOICE requires exactly one H1 project title, found {found}")]
    InvalidTitleCount { found: usize },

    #[error("OMNIVOICE contains no valid section headers")]
    NoOmniVoiceSections,

    #[error("invalid OmniVoice section header: `{line}`")]
    InvalidSectionHeader { line: String },

    #[error("section id `{id}` must match `S` followed by at least two digits")]
    InvalidSectionId { id: String },

    #[error("duplicate OmniVoice section id `{id}`")]
    DuplicateSectionId { id: String },

    #[error("unsupported timestamp `{value}`")]
    InvalidTimestamp { value: String },

    #[error("section {id} start {start} must be before end {end}")]
    InvalidSectionRange {
        id: String,
        start: String,
        end: String,
    },

    #[error("{current_id} starts before {previous_id} ends")]
    TimelineOverlap {
        previous_id: String,
        current_id: String,
    },

    #[error("scene/section mismatch: {message}")]
    SceneSectionMismatch { message: String },
}

impl ScriptError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MarkerCount { .. } => "SCRIPT_MARKER_COUNT",
            Self::MarkerOrder => "SCRIPT_MARKER_ORDER",
            Self::ContentBeforeScenes => "SCRIPT_CONTENT_BEFORE_SCENES",
            Self::EmptyScenesPayload => "SCRIPT_EMPTY_SCENES",
            Self::EmptyOmniVoicePayload => "SCRIPT_EMPTY_OMNIVOICE",
            Self::InvalidScenesYaml { .. } => "SCRIPT_INVALID_SCENES_YAML",
            Self::UnsupportedFormatVersion { .. } => "SCRIPT_FORMAT_VERSION",
            Self::EmptyScenes => "SCRIPT_EMPTY_SCENES",
            Self::InvalidSceneId { .. } => "SCRIPT_INVALID_SCENE_ID",
            Self::DuplicateSceneId { .. } => "SCRIPT_DUPLICATE_SCENE_ID",
            Self::EmptyVisuals { .. } => "SCRIPT_EMPTY_VISUALS",
            Self::InvalidVisualId { .. } => "SCRIPT_INVALID_VISUAL_ID",
            Self::DuplicateVisualId { .. } => "SCRIPT_DUPLICATE_VISUAL_ID",
            Self::InvalidMedia { .. } => "SCRIPT_INVALID_MEDIA",
            Self::EmptyQuery { .. } => "SCRIPT_EMPTY_QUERY",
            Self::InvalidCount { .. } => "SCRIPT_INVALID_COUNT",
            Self::InvalidTitleCount { .. } => "SCRIPT_INVALID_TITLE",
            Self::NoOmniVoiceSections => "SCRIPT_NO_OMNIVOICE_SECTIONS",
            Self::InvalidSectionHeader { .. } => "SCRIPT_INVALID_SECTION_HEADER",
            Self::InvalidSectionId { .. } => "SCRIPT_INVALID_SECTION_ID",
            Self::DuplicateSectionId { .. } => "SCRIPT_DUPLICATE_SECTION_ID",
            Self::InvalidTimestamp { .. } => "SCRIPT_INVALID_TIMESTAMP",
            Self::InvalidSectionRange { .. } => "SCRIPT_INVALID_SECTION_RANGE",
            Self::TimelineOverlap { .. } => "SCRIPT_TIMELINE_OVERLAP",
            Self::SceneSectionMismatch { .. } => "SCRIPT_SCENE_SECTION_MISMATCH",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenesDocument {
    format_version: u32,
    scenes: Vec<RawScene>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScene {
    id: String,
    visuals: Vec<RawVisual>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVisual {
    id: String,
    media: String,
    queries: Vec<String>,
    count: i64,
}

#[derive(Debug, Clone, Copy)]
struct MarkerLocation {
    line_start: usize,
    content_start: usize,
}

/// Parse and validate a `Video Prepare Script Format v1` document.
///
/// Video Prepare owns the SCENES block. The OMNIVOICE block is preserved
/// byte-for-byte after its marker line; this parser only scans its H1 and
/// section headers for local validation and never parses beats, chunks or
/// narration styles.
pub fn parse_script(input: &str) -> Result<PreparedScript, ScriptError> {
    let (scenes_location, omnivoice_location) = locate_markers(input)?;

    if !input[..scenes_location.line_start].trim().is_empty() {
        return Err(ScriptError::ContentBeforeScenes);
    }

    let scenes_payload = &input[scenes_location.content_start..omnivoice_location.line_start];
    if scenes_payload.trim().is_empty() {
        return Err(ScriptError::EmptyScenesPayload);
    }

    let omnivoice_raw = &input[omnivoice_location.content_start..];
    if omnivoice_raw.trim().is_empty() {
        return Err(ScriptError::EmptyOmniVoicePayload);
    }

    let scenes = parse_scenes(scenes_payload)?;
    let omnivoice = parse_omnivoice(omnivoice_raw)?;
    validate_scene_section_mapping(&scenes, &omnivoice.sections)?;

    let input_sha256 = format!("{:x}", Sha256::digest(input.as_bytes()));

    Ok(PreparedScript {
        format_version: 1,
        scenes,
        omnivoice,
        input_sha256,
    })
}

fn locate_markers(input: &str) -> Result<(MarkerLocation, MarkerLocation), ScriptError> {
    let mut scenes = Vec::new();
    let mut omnivoice = Vec::new();
    let mut offset = 0usize;

    for chunk in input.split_inclusive('\n') {
        let raw_line = chunk.strip_suffix('\n').unwrap_or(chunk);
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let line_start = offset;
        let content_start = offset + chunk.len();

        if line == SCENES_MARKER {
            scenes.push(MarkerLocation {
                line_start,
                content_start,
            });
        }
        if line == OMNIVOICE_MARKER {
            omnivoice.push(MarkerLocation {
                line_start,
                content_start,
            });
        }

        offset += chunk.len();
    }

    if scenes.len() != 1 {
        return Err(ScriptError::MarkerCount {
            marker: SCENES_MARKER,
            found: scenes.len(),
        });
    }
    if omnivoice.len() != 1 {
        return Err(ScriptError::MarkerCount {
            marker: OMNIVOICE_MARKER,
            found: omnivoice.len(),
        });
    }

    let scenes = scenes[0];
    let omnivoice = omnivoice[0];
    if scenes.line_start >= omnivoice.line_start {
        return Err(ScriptError::MarkerOrder);
    }

    Ok((scenes, omnivoice))
}

fn parse_scenes(payload: &str) -> Result<Vec<SceneSpec>, ScriptError> {
    let raw: RawScenesDocument =
        serde_yaml::from_str(payload).map_err(|error| ScriptError::InvalidScenesYaml {
            message: error.to_string(),
        })?;

    if raw.format_version != 1 {
        return Err(ScriptError::UnsupportedFormatVersion {
            found: raw.format_version,
        });
    }
    if raw.scenes.is_empty() {
        return Err(ScriptError::EmptyScenes);
    }

    let scene_id_re = Regex::new(r"^S\d{2,}$").expect("scene id regex is valid");
    let visual_id_re = Regex::new(r"^V\d{2,}$").expect("visual id regex is valid");
    let mut scene_ids = HashSet::new();
    let mut scenes = Vec::with_capacity(raw.scenes.len());

    for raw_scene in raw.scenes {
        if !scene_id_re.is_match(&raw_scene.id) {
            return Err(ScriptError::InvalidSceneId { id: raw_scene.id });
        }
        if !scene_ids.insert(raw_scene.id.clone()) {
            return Err(ScriptError::DuplicateSceneId { id: raw_scene.id });
        }
        if raw_scene.visuals.is_empty() {
            return Err(ScriptError::EmptyVisuals {
                scene_id: raw_scene.id,
            });
        }

        let mut visual_ids = HashSet::new();
        let mut visuals = Vec::with_capacity(raw_scene.visuals.len());
        for raw_visual in raw_scene.visuals {
            if !visual_id_re.is_match(&raw_visual.id) {
                return Err(ScriptError::InvalidVisualId {
                    scene_id: raw_scene.id.clone(),
                    visual_id: raw_visual.id,
                });
            }
            if !visual_ids.insert(raw_visual.id.clone()) {
                return Err(ScriptError::DuplicateVisualId {
                    scene_id: raw_scene.id.clone(),
                    visual_id: raw_visual.id,
                });
            }

            let media = MediaKind::parse(&raw_visual.media, &raw_scene.id, &raw_visual.id)?;
            let queries: Vec<String> = raw_visual
                .queries
                .into_iter()
                .map(|query| query.trim().to_owned())
                .filter(|query| !query.is_empty())
                .collect();
            if queries.is_empty() {
                return Err(ScriptError::EmptyQuery {
                    scene_id: raw_scene.id.clone(),
                    visual_id: raw_visual.id,
                });
            }
            if raw_visual.count < 1 || raw_visual.count > u32::MAX as i64 {
                return Err(ScriptError::InvalidCount {
                    scene_id: raw_scene.id.clone(),
                    visual_id: raw_visual.id,
                    count: raw_visual.count,
                });
            }

            visuals.push(VisualRequest {
                id: raw_visual.id,
                media,
                queries,
                count: raw_visual.count as u32,
            });
        }

        scenes.push(SceneSpec {
            id: raw_scene.id,
            visuals,
        });
    }

    Ok(scenes)
}

fn parse_omnivoice(raw_markdown: &str) -> Result<OmniVoiceScript, ScriptError> {
    let title_re = Regex::new(r"^#\s+(.+?)\s*$").expect("title regex is valid");
    let section_re = Regex::new(
        r"(?i)^##\s+(S\d+)\s*[—–-]\s*(\d{1,2}:\d{2}(?::\d{2})?)\s*[—–-]\s*(\d{1,2}:\d{2}(?::\d{2})?)\s*$",
    )
    .expect("section regex is valid");
    let section_id_re = Regex::new(r"^S\d{2,}$").expect("section id regex is valid");

    let mut titles = Vec::new();
    let mut sections: Vec<OmniVoiceSection> = Vec::new();
    let mut ids = HashSet::new();

    for raw_line in raw_markdown.lines() {
        let line = raw_line.trim();

        if let Some(captures) = title_re.captures(line) {
            titles.push(captures[1].trim().to_owned());
            continue;
        }

        if line.starts_with("## ") && !line.starts_with("### ") {
            let Some(captures) = section_re.captures(line) else {
                return Err(ScriptError::InvalidSectionHeader {
                    line: line.to_owned(),
                });
            };

            let id = captures[1].to_ascii_uppercase();
            if !section_id_re.is_match(&id) {
                return Err(ScriptError::InvalidSectionId { id });
            }
            if !ids.insert(id.clone()) {
                return Err(ScriptError::DuplicateSectionId { id });
            }

            let start_time = captures[2].to_owned();
            let end_time = captures[3].to_owned();
            let start_seconds = parse_timestamp(&start_time)?;
            let end_seconds = parse_timestamp(&end_time)?;
            if start_seconds >= end_seconds {
                return Err(ScriptError::InvalidSectionRange {
                    id,
                    start: start_time,
                    end: end_time,
                });
            }

            if let Some(previous) = sections.last() {
                if start_seconds < previous.end_seconds {
                    return Err(ScriptError::TimelineOverlap {
                        previous_id: previous.id.clone(),
                        current_id: id.clone(),
                    });
                }
            }

            sections.push(OmniVoiceSection {
                id,
                start_time,
                end_time,
                start_seconds,
                end_seconds,
            });
        }
    }

    if titles.len() != 1 {
        return Err(ScriptError::InvalidTitleCount {
            found: titles.len(),
        });
    }
    if sections.is_empty() {
        return Err(ScriptError::NoOmniVoiceSections);
    }

    Ok(OmniVoiceScript {
        raw_markdown: raw_markdown.to_owned(),
        title: titles.remove(0),
        sections,
    })
}

fn parse_timestamp(value: &str) -> Result<u64, ScriptError> {
    let parts: Vec<&str> = value.split(':').collect();
    let parse = |part: &str| {
        part.parse::<u64>()
            .map_err(|_| ScriptError::InvalidTimestamp {
                value: value.to_owned(),
            })
    };

    let seconds = match parts.as_slice() {
        [minutes, seconds] => {
            let minutes = parse(minutes)?;
            let seconds = parse(seconds)?;
            if seconds >= 60 {
                return Err(ScriptError::InvalidTimestamp {
                    value: value.to_owned(),
                });
            }
            minutes * 60 + seconds
        }
        [hours, minutes, seconds] => {
            let hours = parse(hours)?;
            let minutes = parse(minutes)?;
            let seconds = parse(seconds)?;
            if minutes >= 60 || seconds >= 60 {
                return Err(ScriptError::InvalidTimestamp {
                    value: value.to_owned(),
                });
            }
            hours * 3600 + minutes * 60 + seconds
        }
        _ => {
            return Err(ScriptError::InvalidTimestamp {
                value: value.to_owned(),
            })
        }
    };

    Ok(seconds)
}

fn validate_scene_section_mapping(
    scenes: &[SceneSpec],
    sections: &[OmniVoiceSection],
) -> Result<(), ScriptError> {
    let scene_ids: HashSet<&str> = scenes.iter().map(|scene| scene.id.as_str()).collect();
    let section_ids: HashSet<&str> = sections.iter().map(|section| section.id.as_str()).collect();

    let mut scenes_without_sections: Vec<&str> =
        scene_ids.difference(&section_ids).copied().collect();
    let mut sections_without_scenes: Vec<&str> =
        section_ids.difference(&scene_ids).copied().collect();
    scenes_without_sections.sort_unstable();
    sections_without_scenes.sort_unstable();

    if !scenes_without_sections.is_empty() || !sections_without_scenes.is_empty() {
        let mut parts = Vec::new();
        if !scenes_without_sections.is_empty() {
            parts.push(format!(
                "scene(s) without OmniVoice section: {}",
                scenes_without_sections.join(", ")
            ));
        }
        if !sections_without_scenes.is_empty() {
            parts.push(format!(
                "OmniVoice section(s) without scene: {}",
                sections_without_scenes.join(", ")
            ));
        }
        return Err(ScriptError::SceneSectionMismatch {
            message: parts.join("; "),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = include_str!("../examples/demo.vprep");

    fn script_with(scenes_yaml: &str, omnivoice: &str) -> String {
        format!("{SCENES_MARKER}\n\n{scenes_yaml}\n\n{OMNIVOICE_MARKER}\n{omnivoice}")
    }

    fn valid_scenes() -> &'static str {
        r#"format_version: 1
scenes:
  - id: S01
    visuals:
      - id: V01
        media: video
        queries: ["person walking"]
        count: 1"#
    }

    fn valid_omni() -> &'static str {
        "\n# Demo\n\n## S01 - 0:00-0:20\n\n[WARM] Hello.\n"
    }

    #[test]
    fn parses_canonical_demo() {
        let parsed = parse_script(DEMO).expect("canonical demo must parse");

        assert_eq!(parsed.format_version, 1);
        assert_eq!(parsed.scenes.len(), 3);
        assert_eq!(parsed.omnivoice.title, "Why Silence Is Powerful");
        assert_eq!(parsed.omnivoice.sections.len(), 3);
        assert_eq!(parsed.omnivoice.sections[0].id, "S01");
        assert_eq!(parsed.omnivoice.sections[2].end_seconds, 65);
        assert_eq!(parsed.input_sha256.len(), 64);
    }

    #[test]
    fn preserves_omnivoice_block_exactly_after_marker_line() {
        let input = script_with(valid_scenes(), valid_omni());
        let expected = valid_omni();
        let parsed = parse_script(&input).unwrap();

        assert_eq!(parsed.omnivoice.raw_markdown, expected);
    }

    #[test]
    fn rejects_missing_marker() {
        let input = format!("{SCENES_MARKER}\n{}", valid_scenes());
        let error = parse_script(&input).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_MARKER_COUNT");
    }

    #[test]
    fn rejects_duplicate_scene_id() {
        let scenes = r#"format_version: 1
scenes:
  - id: S01
    visuals:
      - id: V01
        media: video
        queries: ["a"]
        count: 1
  - id: S01
    visuals:
      - id: V01
        media: image
        queries: ["b"]
        count: 1"#;
        let error = parse_script(&script_with(scenes, valid_omni())).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_DUPLICATE_SCENE_ID");
    }

    #[test]
    fn rejects_duplicate_section_id() {
        let omni = "\n# Demo\n\n## S01 - 0:00-0:20\nText\n## S01 - 0:20-0:40\nText\n";
        let error = parse_script(&script_with(valid_scenes(), omni)).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_DUPLICATE_SECTION_ID");
    }

    #[test]
    fn rejects_invalid_media() {
        let scenes = valid_scenes().replace("media: video", "media: gif");
        let error = parse_script(&script_with(&scenes, valid_omni())).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_INVALID_MEDIA");
    }

    #[test]
    fn rejects_empty_queries() {
        let scenes = valid_scenes().replace("queries: [\"person walking\"]", "queries: []");
        let error = parse_script(&script_with(&scenes, valid_omni())).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_EMPTY_QUERY");
    }

    #[test]
    fn rejects_zero_count() {
        let scenes = valid_scenes().replace("count: 1", "count: 0");
        let error = parse_script(&script_with(&scenes, valid_omni())).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_INVALID_COUNT");
    }

    #[test]
    fn rejects_scene_section_mismatch_in_both_directions() {
        let scenes = valid_scenes().replace("S01", "S02");
        let error = parse_script(&script_with(&scenes, valid_omni())).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_SCENE_SECTION_MISMATCH");
        let message = error.to_string();
        assert!(message.contains("S02"));
        assert!(message.contains("S01"));
    }

    #[test]
    fn rejects_timeline_overlap() {
        let scenes = r#"format_version: 1
scenes:
  - id: S01
    visuals:
      - id: V01
        media: video
        queries: ["a"]
        count: 1
  - id: S02
    visuals:
      - id: V01
        media: image
        queries: ["b"]
        count: 1"#;
        let omni = "\n# Demo\n\n## S01 - 0:00-0:20\nA\n\n## S02 - 0:15-0:40\nB\n";
        let error = parse_script(&script_with(scenes, omni)).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_TIMELINE_OVERLAP");
    }

    #[test]
    fn accepts_gaps_and_hour_timestamps() {
        let scenes = r#"format_version: 1
scenes:
  - id: S01
    visuals:
      - id: V01
        media: video
        queries: ["a"]
        count: 1
  - id: S02
    visuals:
      - id: V01
        media: either
        queries: ["b"]
        count: 2"#;
        let omni = "\n# Demo\n\n## S01 — 0:00-0:20\nA\n\n## S02 – 1:00:00-1:00:30\nB\n";
        let parsed = parse_script(&script_with(scenes, omni)).unwrap();
        assert_eq!(parsed.omnivoice.sections[1].start_seconds, 3600);
        assert_eq!(parsed.omnivoice.sections[1].end_seconds, 3630);
    }

    #[test]
    fn rejects_unknown_yaml_fields_to_keep_runtime_settings_out_of_script() {
        let scenes = format!("{}\nomnivoice_url: https://example.test", valid_scenes());
        let error = parse_script(&script_with(&scenes, valid_omni())).unwrap_err();
        assert_eq!(error.code(), "SCRIPT_INVALID_SCENES_YAML");
    }

    #[test]
    fn hash_changes_when_input_changes() {
        let a = script_with(valid_scenes(), valid_omni());
        let b = format!("{a}\n");
        let parsed_a = parse_script(&a).unwrap();
        let parsed_b = parse_script(&b).unwrap();
        assert_ne!(parsed_a.input_sha256, parsed_b.input_sha256);
    }
}
