from pathlib import Path

path = Path("src/audio.rs")
text = path.read_text()
replacements = {
    "fn save_audio_status(project_root: &Path, status: &AudioFlowStatus) -> Result<(), AudioError> {":
        "pub(crate) fn save_audio_status(project_root: &Path, status: &AudioFlowStatus) -> Result<(), AudioError> {",
    "fn persist_audio_and_coarse(\n": "pub(crate) fn persist_audio_and_coarse(\n",
}
for old, new in replacements.items():
    if new in text:
        continue
    if old not in text:
        raise SystemExit(f"audio visibility anchor not found: {old!r}")
    text = text.replace(old, new, 1)
path.write_text(text)
