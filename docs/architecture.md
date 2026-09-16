# Video Prepare Architecture

Status: DRAFT CONTRACT

## 1. Muc tieu san pham

Video Prepare la mot Rust desktop app gon nhe de bien mot script da duoc chuan hoa thanh bo asset san xuat gom visual va audio.

App khong tao script bang LLM va khong thay the OmniCreator. No tap trung vao mot duong ong nho, ro rang va co the resume:

```text
script.vprep
   |
   +-- Visual Flow -> Pexels -> image/video assets
   |
   +-- Audio Flow -> OmniVoice Studio -> generated audio
   |
   +-- persisted project status -> retry/resume
```

## 2. Nguyen tac kien truc

1. Rust single executable cho desktop runtime.
2. Local-first project state.
3. Hai flow visual va audio doc lap.
4. Runtime settings qua UI, khong sua config file bang tay.
5. Secrets khong nam trong project files.
6. Persist state sau moi buoc quan trong.
7. Resume dua tren persisted state, khong dua tren memory cua process.
8. Manual takeover co the them som cho asset visual.
9. OmniVoice la authority cho narration parsing va audio generation.
10. Video Prepare khong reimplement beat/chunk/TTS logic cua OmniVoice.

## 3. Boundary

```text
Desktop UI
   |
Application Service
   |
   +-- Script Parser
   +-- Project Store
   +-- Visual Coordinator
   +-- Audio Coordinator
   +-- Runtime Settings
   |
   +-- Pexels Client
   +-- OmniVoice Client
   |
Local Project Folder
```

## 4. Script pipeline

Input script co hai phan:

```text
SCENES
+
OMNIVOICE
```

SCENES duoc parse boi Video Prepare.

OMNIVOICE duoc preserve raw va gui sang OmniVoice Project Import API.

Chi parse section header toi thieu o local de validation `Scene ID == Section ID` va timeline. Khong parse Beat/Chunk.

Chi tiet: `docs/script-format-v1.md`.

## 5. Visual flow

V1 dung Pexels.

```text
Scene
  |
  +-- Visual Request V01
  |      |
  |      +-- query 1
  |      +-- query 2 fallback
  |      +-- query 3 fallback
  |
  +-- Visual Request V02
```

Moi request tracking doc lap:

```text
PENDING
RUNNING
PARTIAL
COMPLETED
FAILED
SKIPPED
INTERRUPTED
```

PARTIAL nghia la da tim/tai duoc mot phan asset nhung chua dat `count`.

Retry phai giu lai asset valid da co va chi tim phan con thieu.

## 6. Audio flow

Target contract:

```text
native OmniVoice Markdown
   |
POST /api/v1/projects/import
   |
project_id
   |
POST /api/v1/projects/{project_id}/generate
   |
job_id
   |
GET /api/v1/jobs/{job_id}/wait
   |
GET /api/v1/artifacts?project_id=...
```

Video Prepare khong giu mot HTTP request dai trong suot generation.

App persist `project_id` va `job_id` ngay khi nhan duoc response de co the reconnect sau restart.

Neu OmniVoice URL thay doi vi Kaggle/Colab restart, user update URL trong Runtime Settings UI. Project local va script khong thay doi.

## 7. Flow independence

Visual flow va Audio flow khong chan lan nhau.

Vi du:

```text
Visual: COMPLETED
Audio: FAILED
```

Project tong the la PARTIAL, khong phai FAILED toan bo.

User co the:

```text
Retry Audio
```

ma khong download lai visual.

Tuong tu neu Pexels fail nhung OmniVoice completed, audio phai duoc giu nguyen.

## 8. Runtime Settings

Tat ca setting execution duoc quan ly bang UI.

Core groups:

```text
Project
  Data Root

Flows
  Visual Flow ON/OFF
  Audio Flow ON/OFF

Visual
  Pexels API Key
  download concurrency

Audio
  OmniVoice URL
  OmniVoice API Token
  voice name
  voice variant
  language
  quality preset
  read section titles
```

Chi tiet: `docs/runtime-settings.md`.

## 9. Project storage

De xuat:

```text
<DataRoot>/projects/<project-id>/
  input/
    script.vprep
  project.json
  status.json
  scenes/
    S01/
      visual-status.json
      images/
      videos/
    S02/
  audio/
    omnivoice.json
    artifacts/
  logs/
    run.ndjson
```

`input/script.vprep` la immutable snapshot cua input tai luc create project.

`project.json` la metadata/project identity.

`status.json` la canonical runtime status local.

## 10. Atomic persistence

Moi file state quan trong phai ghi theo pattern:

```text
write temp
-> flush
-> rename replace
```

Khong ghi truc tiep vao `status.json` theo kieu co the de lai JSON bi cat khi process crash.

## 11. Crash recovery

Khi app mo lai project:

- `COMPLETED` giu nguyen neu artifact con ton tai va metadata hop le;
- stale `RUNNING` khong duoc coi la success;
- stale local work chuyen thanh `INTERRUPTED` hoac reconciled state;
- OmniVoice job co `job_id` thi query server truoc khi retry;
- unknown remote state phai duoc ghi la UNKNOWN/INTERRUPTED, khong fabricate FAILED hoac COMPLETED.

## 12. Provider abstraction

V1 khong can dynamic plugin loader.

Interface toi thieu:

```text
StockProvider
  search(request) -> candidates
  download(asset, destination) -> downloaded asset
```

Implementation dau tien:

```text
PexelsProvider
```

Sau nay co the compile them:

```text
PixabayProvider
UnsplashProvider
```

Script khong hard-code provider de khong phai doi format khi them provider moi.

## 13. UI toi thieu

Dashboard:

```text
Video Prepare                         Settings

Project: Why Silence Is Powerful

Visual Flow  ON     17/18   PARTIAL
Audio Flow   ON      3/3    COMPLETED

[ Run ] [ Resume ] [ Retry Failed ]

Scene | Visual      | Audio       | Error
S01   | COMPLETED   | COMPLETED   |
S02   | PARTIAL 1/2 | COMPLETED   | Not enough Pexels results
S03   | COMPLETED   | COMPLETED   |
```

Scene detail cho phep retry rieng visual/audio.

## 14. Non-goals v1

- script generation bang LLM;
- storyboard AI;
- dynamic plugin loading;
- final video render;
- DaVinci automation;
- distributed queue;
- cloud backend;
- user accounts;
- MCP server cua Video Prepare;
- reimplement OmniVoice TTS engine;
- general-purpose workflow DAG.

## 15. Rust stack de xuat

MVP:

```text
eframe / egui    Desktop UI
tokio            async runtime
reqwest          HTTP
serde            data model
serde_yaml       SCENES parser
serde_json       project/status persistence
thiserror        typed errors
tracing          structured logs
uuid             local attempt IDs
sha2             input/artifact fingerprint
```

Khong can SQLite trong MVP neu project state van nho va moi project la mot portable folder.

## 16. Source of truth

- Script syntax: `docs/script-format-v1.md`
- Runtime settings: `docs/runtime-settings.md`
- Status/resume: `docs/state-and-resume.md`
- Delivery plan: `docs/implementation-plan.md`

Neu code va docs conflict trong giai doan contract-first, task implementation phai resolve conflict ro rang thay vi am tham thay doi contract.
