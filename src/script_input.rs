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

fn format_script_candidate(input: &str) -> String {
    let normalized_newlines = input.replace("\r\n", "\n").replace('\r', "\n");
    let without_bom = normalized_newlines.trim_start_matches('\u{feff}');
    let without_fence = strip_outer_markdown_fence(without_bom);
    repair_scenes_yaml_indentation(&without_fence)
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
}
