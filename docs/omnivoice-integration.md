# OmniVoice Integration Contract

Status: TARGET CONTRACT

## 1. Goal

Video Prepare treats OmniVoice Studio as the authority for narration parsing, project generation, section/chunk recovery, voice selection and audio generation.

Video Prepare must not reimplement OmniVoice Beat/Chunk/TTS behavior.

## 2. Input boundary

Video Prepare sends only the raw content after:

```text
--- OMNIVOICE ---
```

It must not send SCENES/Pexels metadata to OmniVoice.

## 3. Required OmniVoice capability

Target capability:

```text
POST /api/v1/projects/import
```

The connected OmniVoice server should advertise:

```json
{
  "features": {
    "project_import": true
  },
  "endpoints": {
    "project_import": "/api/v1/projects/import"
  }
}
```

Video Prepare should gate Audio Flow on capability discovery instead of guessing from server version.

## 4. Import flow

```text
raw native OmniVoice Markdown
  |
POST /api/v1/projects/import
  |
project_id + source_hash
```

Target request:

```json
{
  "project_id": "why-silence-is-powerful",
  "script": "# Why Silence Is Powerful\n\n## S01 - 0:00-0:20\n\n[WARM] ...",
  "speak_section_titles": false
}
```

Target import semantics:

- first import creates the project;
- same project ID + same canonical source is idempotent;
- same project ID + different source returns conflict;
- import does not run GPU generation;
- import does not overwrite an existing different project silently.

## 5. Generate flow

After import:

```text
POST /api/v1/projects/{project_id}/generate
```

Runtime request may include:

```text
voice_name
voice_variant
language
quality_preset
resume
```

These values come from Video Prepare Runtime Settings UI, not from the script format.

## 6. Durable remote identity

Immediately persist:

```text
omnivoice_base_url
omnivoice_project_id
source_hash
job_id after generation submit
```

Do not persist API token.

## 7. Async job flow

Expected flow:

```text
POST generate
  |
202 + job_id
  |
GET /api/v1/jobs/{job_id}/wait
or
GET /api/v1/jobs/{job_id}
  |
terminal state
```

Video Prepare must not rely on one long-running HTTP request for the whole audio render.

## 8. Artifact discovery

After completed generation, use OmniVoice artifact APIs advertised by capabilities.

Current integration target:

```text
GET /api/v1/artifacts?project_id=<id>
```

Do not invent artifact download behavior if the connected server does not advertise/provide it. Capability response and current OmniVoice implementation are authority.

## 9. Runtime URL changes

Kaggle/Colab can expose a new trycloudflare or Gradio public URL after restart.

Video Prepare requirement:

```text
Settings -> OmniVoice URL -> Test -> Apply
```

No Video Prepare restart required.

A new URL creates a new remote connection context. Existing remote job IDs from the old URL must not be assumed to exist on the new server.

## 10. Connection test

Recommended:

```text
GET /health
GET /api/v1/capabilities
```

Success UI should expose:

```text
connected
runtime
model_loaded
project_import support
```

## 11. Authentication

If bearer auth is enabled:

```text
Authorization: Bearer <token>
```

Token is a runtime secret.

Never write it to:

```text
script.vprep
project.json
status.json
run.ndjson
```

## 12. Failure and resume

Timeout while waiting for a job is not proof of generation failure.

Use:

```text
UNKNOWN_REMOTE
```

when remote terminal state cannot be verified.

When the same server becomes reachable, reconcile by job ID before retrying.

When a new OmniVoice server URL replaces the old runtime, create a new attempt and preserve the old attempt history.
