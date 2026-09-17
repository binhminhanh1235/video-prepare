from pathlib import Path

path = Path('.github/scripts/ux_pass3.py')
text = path.read_text()

old = '    fn smart_workspace_action_prefers_remote_reconciliation_when_available() {'
new = '    fn workspace_action_prefers_remote_reconciliation_when_audio_is_unknown() {'
if old not in text:
    raise SystemExit('expected stale desktop test anchor was not found')
text = text.replace(old, new, 1)

# Avoid Python triple-string escaping turning the Rust single-quote char literal into invalid `'''`.
bad_single_quote_match = "| '\\'' |"
if bad_single_quote_match not in text:
    raise SystemExit('expected single-quote matcher anchor was not found')
text = text.replace(bad_single_quote_match, '|', 1)

# Quick-connect must not discard unrelated unsaved Settings edits after saving only the verified endpoint.
old_draft_reset = '                            self.draft = RuntimeSettingsDraft::from_snapshot(&snapshot);'
new_draft_update = '                            self.draft.omnivoice_url = snapshot.safe.omnivoice_url.clone();'
if old_draft_reset not in text:
    raise SystemExit('expected quick-connect draft reset anchor was not found')
text = text.replace(old_draft_reset, new_draft_update, 1)

path.write_text(text)
