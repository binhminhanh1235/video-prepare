from pathlib import Path
import re

path = Path("src/providers/omnivoice.rs")
text = path.read_text()
if "bearer_token_for_request" not in text:
    pattern = r"(    pub fn base_url\(&self\) -> &str \{\n        &self\.normalized_base_url\n    \}\n)"
    match = re.search(pattern, text)
    if not match:
        raise SystemExit("base_url accessor structure not found")
    addition = """

    pub(crate) fn http_client(&self) -> &Client {
        &self.client
    }

    pub(crate) fn bearer_token_for_request(&self) -> Option<&str> {
        self.bearer_token.as_deref()
    }
"""
    text = text[: match.end()] + addition + text[match.end() :]
    path.write_text(text)
