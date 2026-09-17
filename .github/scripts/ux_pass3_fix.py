from pathlib import Path

path = Path('.github/scripts/ux_pass3.py')
text = path.read_text()
old = '    fn smart_workspace_action_prefers_remote_reconciliation_when_available() {'
new = '    fn workspace_action_prefers_remote_reconciliation_when_audio_is_unknown() {'
if old not in text:
    raise SystemExit('expected stale desktop test anchor was not found')
path.write_text(text.replace(old, new, 1))
