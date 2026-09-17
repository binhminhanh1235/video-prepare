from pathlib import Path

path = Path("src/project_catalog.rs")
text = path.read_text()
old = '''                    audio_flow: TaskState::Pending,
                },'''
new = '''                    audio_flow: TaskState::Pending,
                    visual_assets_ready: 0,
                    visual_assets_total: 0,
                    attention_count: 0,
                },'''
count = text.count(old)
if count != 2:
    raise SystemExit(f"expected 2 ProjectSummary test fixtures, found {count}")
text = text.replace(old, new)
path.write_text(text)
