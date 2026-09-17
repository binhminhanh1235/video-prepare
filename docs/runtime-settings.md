# Runtime Settings UI

Status: DRAFT CONTRACT

Muc tieu: user cau hinh toan bo runtime behavior tu UI. Khong yeu cau user sua `.env`, `config.toml`, `settings.yaml` hoac file config bang tay.

## 1. Nguyen tac

Runtime settings la cach app chay, khong phai noi dung project.

Vi vay cac setting sau KHONG duoc chen vao input script:

```text
Data Root
Visual Flow ON/OFF
Audio Flow ON/OFF
Pexels API Key
OmniVoice URL
OmniVoice API Token
Voice name
Voice variant
Language
Quality preset
Read section titles
Download concurrency
```

## 1.1 Persistence qua restart

Khi user bam `Apply settings`:

- non-secret settings duoc ghi vao `preferences.json` trong app config directory;
- file co schema version + revision de UI co the xac nhan lan save/load that;
- lan khoi dong tiep theo app load file nay truoc khi render UI va khoi tao draft tu persisted snapshot;
- format plain `SafePreferences` cu van duoc load de backward-compatible;
- tren macOS va Windows, `Pexels API Key` va `OmniVoice API Token` duoc luu trong OS credential store, khong nam trong `preferences.json`;
- neu secure credential store loi, non-secret settings van duoc save va UI phai hien warning ro rang.

UI phai hien thi path `Loaded from` / `Will save to`, revision da load va secure-secret status de tranh cam giac save thanh cong nhung restart lai mat du lieu.

`OmniVoice URL` chap nhan ca service root va public REST URL ket thuc bang `/api/v1`; app normalize `/api/v1` ve service root de user co the paste truc tiep URL OmniVoice Studio in ra luc startup.

## 2. Settings groups

### Project

`Data Root`

- user chon thu muc bang system folder browser (Finder tren macOS, File Explorer/folder dialog tren Windows, system chooser tren Linux), khong can go path thu cong;
- field Data Root tren UI la read-only va duoc cap nhat tu folder browser;
- app tao `projects/` ben trong Data Root;
- project phai portable trong Data Root;
- khong luu absolute path vao project neu co the dung relative path.

### Flows

`Visual Flow`

- ON/OFF runtime;
- OFF thi visual tasks duoc danh dau skipped by runtime policy, khong xoa asset cu.

`Audio Flow`

- ON/OFF runtime;
- OFF thi khong import/generate OmniVoice trong run moi;
- audio artifact cu giu nguyen.

### Visual

`Pexels API Key`

- set/paste runtime;
- co nut `Test`;
- khong log key;
- khong ghi vao project.json/status.json/script.

`Download concurrency`

- runtime tuning;
- default bao thu, vi du 4;
- co the dieu chinh trong UI.

### Audio

`OmniVoice URL`

Vi du:

```text
https://neo-station-pavilion-bay.trycloudflare.com
```

Hoac Gradio/public host tu Kaggle/Colab.

URL nay co the thay doi moi lan OmniVoice restart. User phai co the paste URL moi va apply ngay khi app dang chay.

`OmniVoice API Token`

- optional neu server khong bat bearer auth;
- runtime secret;
- khong ghi vao project/log.

`Voice name`

- runtime generation choice;
- khong thuoc script content.

`Voice variant`

- vi du `AUTO`, `DEFAULT`, `WARM` tuy contract OmniVoice.

`Language`

- runtime generation setting.

`Quality preset`

- `SAFE`, `BALANCED`, `FAST` theo capability hien co cua OmniVoice;
- UI nen uu tien select/dropdown thay vi free text neu server advertise danh sach.

`Read section titles`

- runtime generation/import option neu OmniVoice support;
- khong thay doi source script.

## 3. OmniVoice URL runtime replacement

Requirement bat buoc:

```text
old URL
  |
user paste new URL
  |
Test
  |
Apply
  |
new OmniVoiceClient instance/config
  |
all future requests use new URL
```

Khong can restart Video Prepare.

Khong mutate existing project script.

Khong reset status da completed.

## 4. URL normalization

Input:

```text
https://abc.trycloudflare.com/
```

Normalize thanh:

```text
https://abc.trycloudflare.com
```

Reject:

- empty URL;
- unsupported scheme;
- URL co credentials embedded neu policy v1 khong support;
- malformed URL.

Khuyen nghi chi chap nhan `http`/`https`, trong do public Kaggle/Colab nen dung `https`.

## 5. Test Connection

Nut `Test` cua OmniVoice nen chay sequence:

```text
GET /health
GET /api/v1/capabilities
```

UI success nen hien thi it nhat:

```text
Connected
service: omnivoice-studio
runtime: kaggle/colab/local
model_loaded: true/false
project_import: supported/unsupported
```

Neu `project_import` chua co, Audio Flow phai bao ro prerequisite thay vi fail mo ho.

## 6. Capability-aware UI

Video Prepare khong nen doan feature dua tren version string neu `/api/v1/capabilities` co the cho biet truc tiep.

Vi du:

```text
features.project_import
features.async_generation
features.audio_artifacts
endpoints.project_import
endpoints.generate_project
```

Neu endpoint import khong available:

```text
Audio Flow unavailable: connected OmniVoice does not advertise project_import.
```

Khong silently fallback sang behavior khac ma user khong biet.

## 7. Apply semantics

Khi user bam `Apply`:

1. validate field local;
2. voi service co nut Test, khong bat buoc auto-call Test neu user khong muon, nhung app phai cho thay connection state;
3. atomically replace runtime settings snapshot;
4. request moi dung snapshot moi;
5. request dang chay khong bi mutate giua chung;
6. khong ghi secret vao project state.

Moi run/attempt nen capture non-secret provenance can thiet, vi du:

```text
provider = pexels
omnivoice_project_id = ...
omnivoice_job_id = ...
```

Khong capture API token/key.

## 8. Persistence cua settings

Safe preferences duoc persist tu dong khi user bam `Apply settings` va duoc load lai khi Video Prepare khoi dong lan sau:

```text
Data Root
flow toggles
OmniVoice URL
voice name
voice variant
language
quality preset
read section titles
download concurrency
```

Preferences file nam trong application config directory cua OS:

```text
macOS   ~/Library/Application Support/video-prepare/preferences.json
Windows %APPDATA%/video-prepare/preferences.json
Linux   $XDG_CONFIG_HOME/video-prepare/preferences.json
        hoac ~/.config/video-prepare/preferences.json
```

Co the override config directory bang `VIDEO_PREPARE_CONFIG_DIR`, huu ich cho test hoac portable deployment.

Secrets van memory-only va KHONG ghi vao preferences file:

```text
Pexels API Key
OmniVoice API Token
```

Neu can remember secret trong future, dung OS credential store thay vi plaintext app config.

## 9. UI sketch

```text
Settings

Project
  Data Root        [ D:/VideoPrepare           ] [Browse]

Flows
  Visual Flow      [ON]
  Audio Flow       [ON]

Visual
  Pexels API Key   [***********************] [Test]
  Concurrency      [4]

Audio
  OmniVoice URL    [https://abc.trycloudflare.com] [Test]
  API Token        [***********************]
  Voice            [Narrator v]
  Variant          [AUTO v]
  Language         [en v]
  Quality          [BALANCED v]
  Read titles      [OFF]

                         [Cancel] [Apply]
```

## 10. Error UX

Khong chi hien `Connection failed`.

Can phan loai it nhat:

```text
INVALID_URL
DNS_ERROR
CONNECT_TIMEOUT
TLS_ERROR
UNAUTHORIZED
FORBIDDEN
SERVICE_MISMATCH
HEALTH_FAILED
CAPABILITY_MISSING
SERVER_ERROR
```

Example:

```text
OmniVoice connected, but project import is not supported by this server.
Required capability: project_import
```

## 11. Runtime setting change va resume

Vi du:

```text
Audio attempt 1
URL A
-> Kaggle restart
-> connection lost
```

User doi thanh URL B.

App khong duoc auto-assume job cu completed/failed tren server moi.

State nen la:

```text
previous remote attempt: INTERRUPTED or UNKNOWN_REMOTE
new attempt: PENDING
```

Neu project import tren server moi la idempotent theo `project_id + source_hash`, app co the import lai an toan va generate/resume theo contract cua server moi.

## 12. Security

- mask secrets trong UI sau khi nhap;
- khong log request Authorization header;
- khong log Pexels key;
- khong ghi secret trong crash report;
- khong chen secret vao URL query string;
- neu can persist future, dung OS credential store.
