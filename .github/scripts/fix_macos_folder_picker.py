from pathlib import Path

path = Path("src/desktop.rs")
text = path.read_text()
old = r'''r#"POSIX path of (choose folder with prompt \"Choose Video Prepare Data Root\")"#'''
new = '''r#"POSIX path of (choose folder with prompt "Choose Video Prepare Data Root")"#'''
if old not in text:
    raise SystemExit("macOS folder picker script literal not found")
path.write_text(text.replace(old, new, 1))
