# Video Prepare Implementation Plan

Status: INITIAL PLAN

## 1. Delivery strategy

Video Prepare duoc trien khai theo contract-first.

Thu tu:

```text
P0 Contracts and project foundation
P1 Script parser and local project store
P2 Visual flow with Pexels
P3 Audio flow with OmniVoice
P4 Desktop UI and runtime settings
P5 Retry/resume hardening and manual takeover
```

Khong mo rong sang workflow engine tong quat trong MVP.

## 2. P0 - Contracts and repository foundation

Goal:

- chot input script format v1;
- chot runtime settings boundary;
- chot status/retry/resume semantics;
- tao example script;
- tao Rust workspace skeleton.

Artifacts:

```text
README.md
docs/architecture.md
docs/script-format-v1.md
docs/runtime-settings.md
docs/state-and-resume.md
docs/implementation-plan.md
examples/demo.vprep
```

Acceptance:

- docs khong mau thuan ve `Scene ID == OmniVoice Section ID`;
- script co dung hai phan SCENES va OMNIVOICE;
- runtime secret khong nam trong script;
- OmniVoice URL runtime UI duoc mo ta ro;
- v1 visual provider la Pexels nhung script khong hard-code provider.

## 3. P1 - Script parser and local project store

Goal:

```text
script.vprep
-> validate
-> PreparedScript
-> create local project
-> persist project/status
-> reopen project
```

Tasks:

1. Rust workspace + app crate.
2. Define serde models cho SCENES.
3. Exact two-section splitter.
4. YAML validation.
5. Minimal OmniVoice Markdown section scanner.
6. Cross-validation Scene IDs == Section IDs.
7. Timeline validation.
8. SHA-256 input fingerprint.
9. Project folder creation.
10. Atomic JSON persistence.
11. Reopen/load project.

Acceptance tests:

- valid demo parses;
- missing marker rejected;
- duplicate scene rejected;
- invalid media rejected;
- empty query rejected;
- missing matching section rejected;
- duplicate section rejected;
- overlap rejected;
- original OmniVoice raw Markdown preserved byte-for-byte except defined outer split behavior;
- project can reopen after process restart simulation.

## 4. P2 - Visual flow: Pexels

Goal:

```text
Scene visual requests
-> Pexels search
-> query fallback
-> download assets
-> persist provenance/status
```

Tasks:

1. `StockProvider` internal trait.
2. `PexelsProvider`.
3. Runtime API key injection.
4. Test connection action.
5. Image search.
6. Video search.
7. `either` resolution.
8. Query fallback.
9. Download to scene folder.
10. Provider asset ID de-dup.
11. PARTIAL state.
12. Retry missing assets only.
13. Rate/network error classification.

Acceptance:

- Pexels key khong xuat hien trong logs/project state;
- completed asset khong bi download lai khi resume;
- partial 1/2 retry chi tim phan con thieu;
- visual failure khong chan audio flow architecture.

## 5. P3 - Audio flow: OmniVoice

Prerequisite:

OmniVoice server advertise:

```text
POST /api/v1/projects/import
features.project_import = true
```

Goal:

```text
OMNIVOICE raw Markdown
-> import project
-> generate
-> persist job_id
-> wait/reconnect
-> discover/import artifacts
```

Tasks:

1. `OmniVoiceClient`.
2. Runtime base URL.
3. Optional bearer token.
4. URL normalization.
5. `/health` connection test.
6. `/api/v1/capabilities` discovery.
7. Capability gate for project import.
8. POST project import.
9. Persist remote project ID/source hash.
10. POST generate.
11. Persist job ID immediately.
12. Wait/poll job.
13. Reconnect after app restart.
14. Artifact discovery.
15. Targeted retry design for failed sections.
16. New URL behavior after Kaggle/Colab restart.

Acceptance:

- user changes OmniVoice URL without restarting Video Prepare;
- project/script not mutated when URL changes;
- request after Apply uses new URL;
- timeout does not fabricate FAILED remote state;
- existing job can be queried after local restart if same server reachable;
- server replacement creates new attempt while preserving old attempt history.

## 6. P4 - Desktop UI and runtime settings

Recommended MVP UI stack:

```text
eframe / egui
```

Screens:

### Project list

- create from script;
- open existing project;
- overall status.

### Project dashboard

- flow toggles;
- Run;
- Resume;
- Retry Failed;
- scene table;
- errors/incomplete items.

### Scene detail

- narration metadata;
- visual requests/assets;
- visual status;
- audio status;
- attempts/errors;
- per-flow retry.

### Settings

- Data Root;
- Visual Flow toggle;
- Audio Flow toggle;
- Pexels key + Test;
- OmniVoice URL + Test;
- OmniVoice token;
- voice/variant/language/quality;
- read section titles;
- download concurrency.

Acceptance:

- no manual config-file editing required;
- URL/key can be changed while app is running;
- error/incomplete work visible without reading logs;
- flow OFF does not delete old outputs.

## 7. P5 - Hardening

Tasks:

1. Startup reconciliation.
2. Stale RUNNING -> INTERRUPTED.
3. UNKNOWN_REMOTE handling.
4. Local artifact existence/checksum validation.
5. Manual visual file takeover.
6. Better typed error catalog.
7. Structured NDJSON logs.
8. Portable Data Root tests.
9. Windows/macOS path tests.
10. Documentation and release checklist.

## 8. Suggested Rust module layout

```text
src/
  main.rs
  app.rs
  domain/
    mod.rs
    script.rs
    project.rs
    state.rs
    settings.rs
  script/
    mod.rs
    parser.rs
    validation.rs
  storage/
    mod.rs
    project_store.rs
    atomic_write.rs
  providers/
    mod.rs
    stock.rs
    pexels.rs
    omnivoice.rs
  flows/
    mod.rs
    visual.rs
    audio.rs
  ui/
    mod.rs
    project_list.rs
    dashboard.rs
    scene_detail.rs
    settings.rs
  error.rs
```

Khong bat buoc giu layout nay neu implementation thuc te tim duoc cach gon hon, nhung domain/provider/UI boundary phai ro.

## 9. MVP dependency direction

```text
UI
 |
Application/Flows
 |
Domain
 |
Provider interfaces + Storage interfaces
 |
HTTP/File implementations
```

Domain khong nen phu thuoc egui.

Script parser khong nen phu thuoc Pexels/OmniVoice HTTP client.

## 10. Definition of Done cho moi phase

Moi phase chi duoc coi la DONE khi:

- code/build hoac docs theo scope da commit;
- tests lien quan da chay;
- khong fabricate PASS;
- README/docs duoc update neu contract thay doi;
- known limitations duoc ghi ro;
- next READY task duoc xac dinh.

## 11. Exact next task sau docs bootstrap

Sau khi PR docs duoc review/merge, task dau tien nen la:

```text
P1.01 - Initialize Rust workspace and implement Script Format v1 parser/validator
```

DoD:

- app crate build duoc;
- parse `examples/demo.vprep`;
- preserve OmniVoice raw block;
- typed validation errors;
- unit tests cho happy path va cac invalid cases quan trong;
- chua goi Pexels;
- chua goi OmniVoice;
- chua can desktop UI ngoai placeholder neu can.

Ly do: parser la contract boundary cua toan bo app. Neu parser va validation chua on dinh, code provider/UI som se tao coupling va rework khong can thiet.
