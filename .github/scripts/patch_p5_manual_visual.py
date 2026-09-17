from pathlib import Path

manual = Path("src/manual_visual.rs")
text = manual.read_text()
old = """    let request = &project.prepared_script.scenes[scene_index].visuals[request_index];
    ensure_media_compatible(request.media, kind)?;

    reconcile_local_project(project)?;
"""
new = """    let (request_media, request_count) = {
        let request = &project.prepared_script.scenes[scene_index].visuals[request_index];
        (request.media, request.count)
    };
    ensure_media_compatible(request_media, kind)?;

    reconcile_local_project(project)?;
"""
if old in text:
    text = text.replace(old, new, 1)
elif new not in text:
    raise SystemExit("manual visual borrow anchor not found")
text = text.replace("request_status.assets.len() >= request.count as usize", "request_status.assets.len() >= request_count as usize")
text = text.replace("next_missing_slot(&request_status.assets, request.count)", "next_missing_slot(&request_status.assets, request_count)")
manual.write_text(text)

inspection = Path("src/inspection.rs")
text = inspection.read_text()
text = text.replace(
    "    load_audio_status, load_visual_status, AudioFlowStatus, StoredProject, TaskState,\n    VisualFlowStatus,\n",
    "    load_audio_status, load_visual_status, AudioFlowStatus, StoredProject, TaskState,\n",
)
inspection.write_text(text)
