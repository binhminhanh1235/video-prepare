from pathlib import Path

path = Path("src/desktop.rs")
text = path.read_text()
old = "request.completed_assets < request.target_count"
new = "request.completed_assets < request.target_count as usize"
if old in text:
    text = text.replace(old, new, 1)
elif new not in text:
    raise SystemExit("manual visual GUI comparison anchor not found")
path.write_text(text)
