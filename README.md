# Video Prepare

Video Prepare la mot Rust desktop app gon nhe de chuan bi asset cho video tu mot script da duoc tao san.

App tap trung vao ba viec:

1. Doc script theo `Video Prepare Script Format v1`.
2. Chay Visual Flow de tim/tai image va video theo scene, bat dau voi Pexels.
3. Chay Audio Flow bang cach gui native narration project sang OmniVoice Studio va tracking generation theo job.

Video Prepare khong thay the OmniCreator. No la mot production runner nho, deterministic, local-first va co the retry/resume.

## Core flow

```text
script.vprep
   |
   +-- SCENES --------> Visual Flow -------> Pexels -------> image/video assets
   |
   +-- OMNIVOICE -----> Audio Flow --------> OmniVoice ----> audio artifacts
   |
   +-- persisted project state ----------------------------> retry/resume
```

Hai flow Visual va Audio doc lap. Loi Pexels khong duoc xoa/chan audio da thanh cong, va loi OmniVoice khong duoc lam mat visual da tai.

## Input script

Input script co dung hai phan:

```text
--- SCENES ---

<YAML scene + visual queries>

--- OMNIVOICE ---

<native OmniVoice Markdown narration>
```

Vi du toi thieu:

```text
--- SCENES ---

format_version: 1
scenes:
  - id: S01
    visuals:
      - id: V01
        media: video
        queries:
          - "thoughtful man sitting alone by window cinematic"
        count: 2

--- OMNIVOICE ---

# Why Silence Is Powerful

## S01 - 0:00-0:20

[WARM] Most people believe silence means weakness.
```

Quy tac quan trong nhat:

```text
Scene S01 == OmniVoice Section S01
```

Script khong chua API key, OmniVoice URL, flow toggle, Data Root hoac runtime secret.

Doc day du: [docs/script-format-v1.md](docs/script-format-v1.md)

Canonical example: [examples/demo.vprep](examples/demo.vprep)

### LLM/paste compatibility

Canonical YAML + Markdown van la format duoc khuyen nghi, nhung input co hai tag chuan van duoc thu normalize an toan khi LLM lam mat presentation formatting.

Video Prepare co the recover cac truong hop pho bien nhu:

```text
--- SCENES ---
format_version: 1
scenes:
id: S01
visuals:
id: V01
media: video
queries:
"query one"
"query two"
count: 2

--- OMNIVOICE ---
Project title
S01 - 0:00-0:20
Narration...
```

Fallback chi sua presentation structure: khoi phuc YAML list/indent va them `#` / `##` cho title/section OmniVoice. Semantic validation khong duoc noi long: field la, ID sai, media sai, query rong, count sai, timeline overlap hoac scene/section mismatch van bi reject.

## Runtime settings UI

Tat ca execution settings duoc dua ra UI thay vi yeu cau sua config file:

```text
Project
  Data Root

Flows
  Visual Flow ON/OFF
  Audio Flow ON/OFF

Visual
  Pexels API Key
  Download concurrency

Audio
  OmniVoice URL
  OmniVoice API Token
  Voice
  Variant
  Language
  Quality preset
  Read section titles
```

OmniVoice URL la runtime setting first-class vi Kaggle/Colab co the tao trycloudflare hoac Gradio URL moi sau moi restart. User chi can paste URL moi, Test va Apply. Khong can restart Video Prepare va khong sua script/project.

Chi tiet: [docs/runtime-settings.md](docs/runtime-settings.md)

## OmniVoice integration

Target flow:

```text
OMNIVOICE raw Markdown
   |
POST /api/v1/projects/import
   |
project_id
   |
POST /api/v1/projects/{project_id}/generate
   |
job_id
   |
wait/reconcile
   |
artifacts
```

Video Prepare se capability-check `project_import` truoc khi chay Audio Flow.

Chi tiet: [docs/omnivoice-integration.md](docs/omnivoice-integration.md)

## Retry and resume

Canonical task states v1:

```text
PENDING
RUNNING
PARTIAL
COMPLETED
FAILED
SKIPPED
INTERRUPTED
UNKNOWN_REMOTE
```

Persisted state la source of truth. Timeout/mat mang khong duoc fabricate thanh remote failure.

Chi tiet: [docs/state-and-resume.md](docs/state-and-resume.md)

## Architecture

MVP de xuat:

```text
Rust
+ eframe/egui
+ tokio
+ reqwest
+ serde / serde_yaml / serde_json
+ thiserror
+ tracing
+ sha2
```

Khong can SQLite, dynamic plugin loader, MCP server, LLM, final rendering hoac general workflow DAG trong MVP.

Chi tiet: [docs/architecture.md](docs/architecture.md)

## Implementation plan

```text
P0 Contracts and project foundation
P1 Script parser and local project store
P2 Visual flow with Pexels
P3 Audio flow with OmniVoice
P4 Desktop UI and runtime settings
P5 Retry/resume hardening and manual takeover
```

Exact next implementation task sau docs bootstrap:

```text
P1.01 - Initialize Rust workspace and implement Script Format v1 parser/validator
```

Chi tiet: [docs/implementation-plan.md](docs/implementation-plan.md)

## Current status

Repository bootstrap dang o giai doan contract-first. Chua co production Rust implementation.

Docs trong branch bootstrap la source of truth cho phase implementation tiep theo. Neu implementation can thay doi contract, thay doi phai duoc ghi ro va update docs thay vi silently drift.
