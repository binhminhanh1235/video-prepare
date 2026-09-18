use regex::Regex;

use crate::script::{self, PreparedScript, ScriptError, OMNIVOICE_MARKER, SCENES_MARKER};

/// Parse a Video Prepare script with a conservative formatting fallback for pasted input.
///
/// Valid scripts are parsed byte-for-byte without modification. If strict parsing fails,
/// we normalize only presentation-level issues that commonly appear when copying YAML from
/// chat/Markdown, then retry strict validation. Semantic validation remains owned by
/// `script::parse_script` and is never weakened here.
pub fn parse_script(input: &str) -> Result<PreparedScript, ScriptError> {
    match script::parse_script(input) {
        Ok(prepared) => Ok(prepared),
        Err(original_error) => {
            let candidate = format_script_candidate(input);
            if candidate == input {
                return Err(original_error);
            }
            script::parse_script(&candidate)
        }
    }
}

/// Return a normalized script suitable for previewing or persisting.
///
/// The formatter intentionally repairs only safe, presentation-level problems. The result
/// must still pass the strict script parser, so invalid media values, IDs, counts, timeline
/// overlaps, scene/section mismatches, and other semantic errors are still rejected.
pub fn format_script(input: &str) -> Result<String, ScriptError> {
    let candidate = format_script_candidate(input);
    script::parse_script(&candidate)?;
    Ok(candidate)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInspectionResult {
    pub raw_markdown: String,
    pub script: Option<crate::script::OmniVoiceScript>,
    pub status_message: String,
    pub is_valid: bool,
    pub section_narrations: Vec<(String, String)>,
}

pub fn inspect_audio_input(input: &str) -> AudioInspectionResult {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return AudioInspectionResult {
            raw_markdown: String::new(),
            script: None,
            status_message: "No script entered".to_owned(),
            is_valid: false,
            section_narrations: Vec::new(),
        };
    }

    let candidate = format_script_candidate(input);
    let has_marker = candidate.contains(OMNIVOICE_MARKER);
    let raw_payload = if let Some(pos) = candidate.find(OMNIVOICE_MARKER) {
        let after = &candidate[pos + OMNIVOICE_MARKER.len()..];
        let after = after
            .strip_prefix("\r\n")
            .or_else(|| after.strip_prefix('\n'))
            .unwrap_or(after);
        after.to_owned()
    } else {
        candidate.clone()
    };

    let narrations = script::extract_section_narrations(&raw_payload);

    if raw_payload.trim().is_empty() {
        return AudioInspectionResult {
            raw_markdown: raw_payload,
            script: None,
            status_message: "OMNIVOICE script is empty".to_owned(),
            is_valid: false,
            section_narrations: Vec::new(),
        };
    }

    match script::parse_omnivoice(&raw_payload) {
        Ok(omni) => {
            let (is_valid, status_message) = if has_marker {
                (true, "Valid format".to_owned())
            } else {
                (
                    false,
                    "Audio format is valid, but missing '--- SCENES ---' and '--- OMNIVOICE ---' markers".to_owned(),
                )
            };
            AudioInspectionResult {
                raw_markdown: raw_payload,
                script: Some(omni),
                status_message,
                is_valid,
                section_narrations: narrations,
            }
        }
        Err(err) => AudioInspectionResult {
            raw_markdown: raw_payload,
            script: None,
            status_message: format!("Invalid format: {} · {}", err.code(), err),
            is_valid: false,
            section_narrations: narrations,
        },
    }
}

fn format_script_candidate(input: &str) -> String {
    let normalized_newlines = input.replace("\r\n", "\n").replace('\r', "\n");
    let without_bom = normalized_newlines.trim_start_matches('\u{feff}');
    let without_fence = strip_outer_markdown_fence(without_bom);
    let repaired_indentation = repair_scenes_yaml_indentation(&without_fence);
    let repaired_scenes = repair_flattened_tagged_scenes(&repaired_indentation);
    repair_omnivoice_markdown(&repaired_scenes)
}

fn strip_outer_markdown_fence(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let Some(first) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return input.to_owned();
    };
    let Some(last) = lines.iter().rposition(|line| !line.trim().is_empty()) else {
        return input.to_owned();
    };

    if first < last && lines[first].trim_start().starts_with("```") && lines[last].trim() == "```" {
        let mut stripped = lines[first + 1..last].join("\n");
        if input.ends_with('\n') {
            stripped.push('\n');
        }
        stripped
    } else {
        input.to_owned()
    }
}

fn repair_scenes_yaml_indentation(input: &str) -> String {
    let had_trailing_newline = input.ends_with('\n');
    let mut output = Vec::new();
    let mut in_scenes = false;
    let mut queries_indent: Option<usize> = None;

    for raw_line in input.lines() {
        let mut line = normalize_leading_tabs(raw_line);
        let trimmed = line.trim_start();

        if trimmed == SCENES_MARKER {
            in_scenes = true;
            queries_indent = None;
            output.push(line);
            continue;
        }
        if trimmed == OMNIVOICE_MARKER {
            in_scenes = false;
            queries_indent = None;
            output.push(line);
            continue;
        }

        if in_scenes {
            let indent = line.len() - trimmed.len();

            if trimmed.starts_with("queries:") {
                queries_indent = Some(indent);
            } else if trimmed.starts_with("count:") {
                if let Some(expected_indent) = queries_indent {
                    if indent > expected_indent {
                        line = format!("{}{}", " ".repeat(expected_indent), trimmed);
                    }
                }
                queries_indent = None;
            } else if let Some(expected_indent) = queries_indent {
                if indent < expected_indent
                    || (trimmed.starts_with("- id:") && indent < expected_indent)
                {
                    queries_indent = None;
                }
            }
        }

        output.push(line);
    }

    let mut repaired = output.join("\n");
    if had_trailing_newline {
        repaired.push('\n');
    }
    repaired
}

#[derive(Debug)]
struct RecoveredScene {
    id: String,
    visuals: Vec<RecoveredVisual>,
}

#[derive(Debug)]
struct RecoveredVisual {
    id: String,
    media: Option<String>,
    queries: Vec<String>,
    count: Option<String>,
}

fn repair_flattened_tagged_scenes(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let Some(scenes_marker) = lines.iter().position(|line| line.trim() == SCENES_MARKER) else {
        return input.to_owned();
    };
    let Some(omnivoice_marker) = lines
        .iter()
        .position(|line| line.trim() == OMNIVOICE_MARKER)
    else {
        return input.to_owned();
    };
    if scenes_marker >= omnivoice_marker {
        return input.to_owned();
    }

    let payload = &lines[scenes_marker + 1..omnivoice_marker];
    if payload
        .iter()
        .any(|line| line.trim_start().starts_with("- id:"))
    {
        return input.to_owned();
    }

    let scene_id_re = Regex::new(r"^S\d{2,}$").expect("scene id regex is valid");
    let visual_id_re = Regex::new(r"^V\d{2,}$").expect("visual id regex is valid");
    let mut format_version: Option<String> = None;
    let mut saw_scenes_key = false;
    let mut recovered_shape = false;
    let mut scenes = Vec::new();
    let mut current_scene: Option<RecoveredScene> = None;
    let mut current_visual: Option<RecoveredVisual> = None;
    let mut in_queries = false;

    for raw_line in payload {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("format_version:") {
            if format_version.is_some() {
                return input.to_owned();
            }
            let value = value.trim();
            if value.is_empty() {
                return input.to_owned();
            }
            format_version = Some(value.to_owned());
            continue;
        }

        if trimmed == "scenes:" {
            if saw_scenes_key {
                return input.to_owned();
            }
            saw_scenes_key = true;
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("id:") {
            let id = value.trim();
            if scene_id_re.is_match(id) {
                if let Some(visual) = current_visual.take() {
                    let Some(scene) = current_scene.as_mut() else {
                        return input.to_owned();
                    };
                    scene.visuals.push(visual);
                }
                if let Some(scene) = current_scene.take() {
                    scenes.push(scene);
                }
                current_scene = Some(RecoveredScene {
                    id: id.to_owned(),
                    visuals: Vec::new(),
                });
                in_queries = false;
                recovered_shape = true;
                continue;
            }

            if visual_id_re.is_match(id) {
                let Some(scene) = current_scene.as_mut() else {
                    return input.to_owned();
                };
                if let Some(visual) = current_visual.take() {
                    scene.visuals.push(visual);
                }
                current_visual = Some(RecoveredVisual {
                    id: id.to_owned(),
                    media: None,
                    queries: Vec::new(),
                    count: None,
                });
                in_queries = false;
                recovered_shape = true;
                continue;
            }

            return input.to_owned();
        }

        if trimmed == "visuals:" {
            if current_scene.is_none() || current_visual.is_some() {
                return input.to_owned();
            }
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("media:") {
            let Some(visual) = current_visual.as_mut() else {
                return input.to_owned();
            };
            if visual.media.is_some() {
                return input.to_owned();
            }
            let value = value.trim();
            if value.is_empty() {
                return input.to_owned();
            }
            visual.media = Some(value.to_owned());
            in_queries = false;
            continue;
        }

        if trimmed == "queries:" {
            if current_visual.is_none() || in_queries {
                return input.to_owned();
            }
            in_queries = true;
            continue;
        }

        if let Some(value) = trimmed.strip_prefix("count:") {
            let Some(visual) = current_visual.as_mut() else {
                return input.to_owned();
            };
            if visual.count.is_some() {
                return input.to_owned();
            }
            let value = value.trim();
            if value.is_empty() {
                return input.to_owned();
            }
            visual.count = Some(value.to_owned());
            in_queries = false;
            continue;
        }

        if in_queries {
            let query = trimmed.strip_prefix("- ").unwrap_or(trimmed).trim();
            if query.is_empty() || serde_yaml::from_str::<String>(query).is_err() {
                return input.to_owned();
            }
            let Some(visual) = current_visual.as_mut() else {
                return input.to_owned();
            };
            visual.queries.push(query.to_owned());
            continue;
        }

        return input.to_owned();
    }

    if let Some(visual) = current_visual.take() {
        let Some(scene) = current_scene.as_mut() else {
            return input.to_owned();
        };
        scene.visuals.push(visual);
    }
    if let Some(scene) = current_scene.take() {
        scenes.push(scene);
    }

    if !recovered_shape || !saw_scenes_key || scenes.is_empty() {
        return input.to_owned();
    }
    let Some(format_version) = format_version else {
        return input.to_owned();
    };

    if scenes.iter().any(|scene| {
        scene.visuals.is_empty()
            || scene.visuals.iter().any(|visual| {
                visual.media.is_none() || visual.queries.is_empty() || visual.count.is_none()
            })
    }) {
        return input.to_owned();
    }

    let mut canonical = format!("format_version: {format_version}\nscenes:\n");
    for scene in scenes {
        canonical.push_str(&format!("  - id: {}\n    visuals:\n", scene.id));
        for visual in scene.visuals {
            canonical.push_str(&format!(
                "      - id: {}\n        media: {}\n        queries:\n",
                visual.id,
                visual.media.expect("validated media")
            ));
            for query in visual.queries {
                canonical.push_str(&format!("          - {query}\n"));
            }
            canonical.push_str(&format!(
                "        count: {}\n",
                visual.count.expect("validated count")
            ));
        }
    }

    let mut repaired = lines[..=scenes_marker].join("\n");
    repaired.push('\n');
    repaired.push_str(canonical.trim_end_matches('\n'));
    repaired.push('\n');
    repaired.push_str(&lines[omnivoice_marker..].join("\n"));
    if input.ends_with('\n') {
        repaired.push('\n');
    }
    repaired
}

fn repair_omnivoice_markdown(input: &str) -> String {
    let had_trailing_newline = input.ends_with('\n');
    let mut lines: Vec<String> = input.lines().map(str::to_owned).collect();
    let Some(marker) = lines
        .iter()
        .position(|line| line.trim() == OMNIVOICE_MARKER)
    else {
        return input.to_owned();
    };

    let section_re = Regex::new(
        r"(?i)^S\d+\s*[—–-]\s*\d{1,2}:\d{2}(?::\d{2})?\s*[—–-]\s*\d{1,2}:\d{2}(?::\d{2})?\s*$",
    )
    .expect("plain OmniVoice section regex is valid");

    let mut has_title = lines[marker + 1..].iter().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with("# ") && !trimmed.starts_with("## ")
    });
    let mut changed = false;

    for line in lines.iter_mut().skip(marker + 1) {
        let trimmed = line.trim().to_owned();
        if trimmed.is_empty() {
            continue;
        }

        if section_re.is_match(&trimmed) {
            *line = format!("## {trimmed}");
            changed = true;
            continue;
        }

        if !has_title && !trimmed.starts_with('#') {
            *line = format!("# {trimmed}");
            has_title = true;
            changed = true;
        }
    }

    if !changed {
        return input.to_owned();
    }

    let mut repaired = lines.join("\n");
    if had_trailing_newline {
        repaired.push('\n');
    }
    repaired
}

fn normalize_leading_tabs(line: &str) -> String {
    let mut prefix = String::new();
    let mut consumed = 0usize;

    for (index, ch) in line.char_indices() {
        match ch {
            ' ' => prefix.push(' '),
            '\t' => prefix.push_str("  "),
            _ => {
                consumed = index;
                break;
            }
        }
        consumed = index + ch.len_utf8();
    }

    if consumed == 0 || !line[..consumed].contains('\t') {
        return line.to_owned();
    }

    prefix.push_str(&line[consumed..]);
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    fn malformed_script() -> &'static str {
        r#"--- SCENES ---

format_version: 1
scenes:

- id: S01
  visuals:
  - id: V01
    media: video
    queries:
    - "thoughtful man sitting alone by window morning light cinematic"
    - "quiet man looking through window peaceful room cinematic"
      count: 2

--- OMNIVOICE ---

# Why Silence Is Powerful

## S01 - 0:00-0:20

[WARM] Most people believe silence means weakness.
"#
    }

    #[test]
    fn repairs_count_accidentally_nested_under_last_query() {
        let formatted = format_script(malformed_script()).expect("formatter should repair YAML");

        assert!(formatted.contains("\n    count: 2\n"));
        assert!(!formatted.contains("\n      count: 2\n"));

        let prepared = parse_script(malformed_script()).expect("formatted script should parse");
        assert_eq!(prepared.scenes[0].visuals[0].count, 2);
        assert_eq!(prepared.scenes[0].visuals[0].queries.len(), 2);
    }

    #[test]
    fn accepts_outer_markdown_code_fence() {
        let fenced = format!("```yaml\n{}\n```\n", malformed_script());
        let prepared = parse_script(&fenced).expect("outer Markdown fence should be removed");

        assert_eq!(prepared.omnivoice.title, "Why Silence Is Powerful");
    }

    fn flattened_tagged_script() -> &'static str {
        r#"--- SCENES ---
format_version: 1
scenes:

id: S01
visuals:

id: V01
media: video
queries:

"exhausted college student closing laptop late night desk lamp"

"tired man rubbing eyes in front of computer dark room"
count: 2

id: S02
visuals:

id: V01
media: image
queries:

"young adult sitting on edge of bed staring at floor morning"

"frustrated person looking at notebook leaning back in chair"
count: 1

--- OMNIVOICE ---

The Real Reason We Learn

S01 - 0:00-0:20
[WARM] Nobody burns out from learning.

S02 - 0:20-0:40
And when the only reason you sit down to study is external pressure.
"#
    }

    #[test]
    fn repairs_flattened_tagged_llm_output() {
        let prepared =
            parse_script(flattened_tagged_script()).expect("tagged LLM output should parse");

        assert_eq!(prepared.scenes.len(), 2);
        assert_eq!(prepared.scenes[0].id, "S01");
        assert_eq!(prepared.scenes[0].visuals[0].queries.len(), 2);
        assert_eq!(prepared.scenes[0].visuals[0].count, 2);
        assert_eq!(
            prepared.scenes[1].visuals[0].media,
            crate::script::MediaKind::Image
        );
        assert_eq!(prepared.omnivoice.title, "The Real Reason We Learn");
        assert_eq!(prepared.omnivoice.sections.len(), 2);
        assert!(prepared
            .omnivoice
            .raw_markdown
            .contains("# The Real Reason We Learn"));
        assert!(prepared
            .omnivoice
            .raw_markdown
            .contains("## S01 - 0:00-0:20"));
    }

    #[test]
    fn formats_flattened_tagged_llm_output_to_canonical_shape() {
        let formatted =
            format_script(flattened_tagged_script()).expect("tagged LLM output should format");

        assert!(formatted.contains("\n  - id: S01\n    visuals:\n"));
        assert!(formatted.contains(
            "\n          - \"exhausted college student closing laptop late night desk lamp\"\n"
        ));
        assert!(formatted.contains("\n# The Real Reason We Learn\n"));
        assert!(formatted.contains("\n## S02 - 0:20-0:40\n"));
    }

    #[test]
    fn does_not_silently_drop_unknown_fields_in_flattened_input() {
        let invalid =
            flattened_tagged_script().replace("media: video", "media: video\nmood: cinematic");
        let error = parse_script(&invalid).expect_err("unknown fields must still fail");

        assert_eq!(error.code(), "SCRIPT_INVALID_SCENES_YAML");
    }

    #[test]
    fn semantic_validation_is_not_weakened() {
        let invalid = malformed_script().replace("media: video", "media: gif");
        let error = parse_script(&invalid).expect_err("invalid media must still fail");

        assert_eq!(error.code(), "SCRIPT_INVALID_MEDIA");
    }

    #[test]
    fn valid_input_keeps_original_hash() {
        let valid = malformed_script().replace("      count: 2", "    count: 2");
        let strict = script::parse_script(&valid).unwrap();
        let tolerant = parse_script(&valid).unwrap();

        assert_eq!(tolerant.input_sha256, strict.input_sha256);
    }

    #[test]
    fn inspect_audio_valid_and_invalid_inputs() {
        let empty = inspect_audio_input("   ");
        assert!(!empty.is_valid);
        assert_eq!(empty.status_message, "No script entered");

        let demo = include_str!("../examples/demo.vprep");
        let valid = inspect_audio_input(demo);
        assert!(valid.is_valid);
        assert_eq!(valid.status_message, "Valid format");
        assert_eq!(valid.script.as_ref().unwrap().title, "Why Silence Is Powerful");
        assert_eq!(valid.section_narrations.len(), 3);

        let invalid_time = demo.replace("## S01 - 0:00-0:20", "## S01 - 0:30-0:20");
        let invalid = inspect_audio_input(&invalid_time);
        assert!(!invalid.is_valid);
        assert!(invalid.status_message.contains("SCRIPT_INVALID_SECTION_RANGE"));
    }
}
