# Video Prepare Script Format v1

Status: DRAFT CONTRACT

Muc tieu cua format nay la de mot file script duy nhat co the dieu khien hai flow doc lap:

1. Visual flow: lay image/video theo scene, v1 dung Pexels.
2. Audio flow: gui phan narration native sang OmniVoice Studio.

Script chi mo ta noi dung can san xuat. Script KHONG chua API key, OmniVoice URL, Data Root, flow ON/OFF, voice runtime, quality preset, download concurrency hoac cac setting van hanh khac. Tat ca cac setting do thuoc Runtime Settings UI.

## 1. Cau truc tong the

File script bat buoc co dung hai phan theo thu tu sau:

```text
--- SCENES ---

<YAML scene specification>

--- OMNIVOICE ---

<native OmniVoice Markdown narration>
```

Hai marker phai dung nguyen van va nam tren dong rieng:

```text
--- SCENES ---
--- OMNIVOICE ---
```

Khong duoc dat marker trong code block, narration, query hoac comment.

Parser v1 thuc hien:

```text
input file
   |
   +-- split at exact marker: --- OMNIVOICE ---
   |
   +-- SCENES payload -> YAML parser
   |
   +-- OMNIVOICE payload -> preserved as native Markdown
```

Phan OMNIVOICE phai duoc giu nguyen noi dung. Video Prepare khong tu y rewrite, split chunk, thay directive hoac chuan hoa narration text truoc khi gui sang OmniVoice.

## 2. Vi du day du

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
          - "lonely man looking through window"
        count: 2

      - id: V02
        media: image
        queries:
          - "dark quiet room cinematic"
        count: 1

  - id: S02
    visuals:
      - id: V01
        media: video
        queries:
          - "calm man ignoring angry conversation"
          - "person calmly walking away from argument"
        count: 2

  - id: S03
    visuals:
      - id: V01
        media: either
        queries:
          - "empty street at night cinematic"
          - "lonely city street at night"
        count: 1

--- OMNIVOICE ---

# Why Silence Is Powerful

## S01 - 0:00-0:20

### Silence is not weakness

[WARM] Most people believe silence means weakness.

But silence can also mean control.

## S02 - 0:20-0:42

### Stop reacting

[EMPHASIZE] You do not have to react to every provocation.

Sometimes the strongest response is no response at all.

## S03 - 0:42-1:05

[SOFT] Silence gives you enough distance to see clearly.
```

## 3. Quy tac can cot: Scene ID la Section ID

Video Prepare v1 dung mot namespace ID duy nhat cho scene va narration section.

Quy tac:

```text
Scene S01 == OmniVoice Section S01
Scene S02 == OmniVoice Section S02
Scene S03 == OmniVoice Section S03
```

Ly do:

- visual status va audio status cung gan vao mot scene;
- retry scene khong can mapping trung gian;
- imported OmniVoice project co section ID trung voi project local;
- log, error, provenance va resume state de doc;
- tranh mapping kieu `scene-17 -> audio-section-12`.

Validation bat buoc:

- moi scene ID trong SCENES phai ton tai trong OMNIVOICE;
- moi section ID trong OMNIVOICE phai co scene tuong ung trong SCENES, tru khi sau nay schema version moi khai bao ro audio-only section;
- scene ID phai unique;
- OmniVoice section ID phai unique;
- canonical ID v1 co dang `S` + 2 chu so tro len, vi du `S01`, `S02`, `S10`.

## 4. SCENES section

### 4.1 Root schema

```yaml
format_version: 1
scenes: []
```

`format_version` bat buoc va hien tai chi chap nhan `1`.

`scenes` bat buoc, la array co it nhat mot scene.

### 4.2 Scene

```yaml
- id: S01
  visuals: []
```

Fields:

| Field | Required | Type | Meaning |
| --- | --- | --- | --- |
| `id` | yes | string | Stable scene ID, phai match OmniVoice section ID |
| `visuals` | yes | array | Danh sach visual request cua scene |

Scene khong chua narration. Narration chi nam trong phan OMNIVOICE.

Scene khong chua `duration`, `start`, `end` trong v1. Planned timing da co trong OmniVoice section header va chi co mot source of truth.

### 4.3 Visual request

```yaml
- id: V01
  media: video
  queries:
    - "thoughtful man sitting alone by window cinematic"
    - "lonely man looking through window"
  count: 2
```

Fields:

| Field | Required | Type | Meaning |
| --- | --- | --- | --- |
| `id` | yes | string | Stable visual request ID trong scene |
| `media` | yes | enum | `image`, `video`, hoac `either` |
| `queries` | yes | string[] | Search queries thu theo thu tu fallback |
| `count` | yes | integer | So asset can tim, toi thieu 1 |

`visual.id` chi can unique trong scene. Vi du `S01/V01` va `S02/V01` la hop le.

### 4.4 Media behavior

`media: image`

- chi chap nhan image candidate.

`media: video`

- chi chap nhan video candidate.

`media: either`

- chap nhan image hoac video;
- dung khi noi dung quan trong hon loai asset;
- provider adapter co the uu tien theo runtime policy sau nay.

V1 khong co `mixed`. Mot scene tu dong tro thanh mixed neu co nhieu visual request voi media khac nhau.

### 4.5 Query fallback

Queries duoc thu theo thu tu khai bao.

Vi du:

```yaml
queries:
  - "calm man walking away from argument"
  - "person leaving conflict calmly"
  - "person walking away cinematic"
count: 2
```

Semantics:

```text
query 1
  |
  +-- du count -> completed
  |
  +-- chua du -> query 2
                 |
                 +-- du count -> completed
                 |
                 +-- chua du -> query 3
```

Neu thu het query ma co asset nhung chua du `count`, task co the la `PARTIAL` thay vi mat ket qua da tai.

Retry chi tim phan con thieu. Khong mac dinh xoa va tai lai asset da completed.

### 4.6 Provider khong nam trong script

Khong viet:

```yaml
provider: pexels
api_key: ...
```

V1 runtime se dung Pexels, nhung script mo ta nhu cau noi dung, khong mo ta credential hay provider deployment.

Sau nay cung mot script co the duoc chay voi:

```text
Pexels -> Pixabay -> Unsplash
```

ma khong can sua script.

## 5. OMNIVOICE section

Phan sau `--- OMNIVOICE ---` la native OmniVoice Project Studio Markdown.

Canonical Video Prepare v1 khuyen nghi syntax:

```markdown
# Project title

## S01 - 0:00-0:20

### Optional section title

[WARM] Narration text.

More narration.

## S02 - 0:20-0:45

[SOFT] More narration.
```

Video Prepare KHONG so huu grammar narration. OmniVoice la authority cho narration format.

### 5.1 Project title

```markdown
# Why Silence Is Powerful
```

Dung lam project title.

V1 yeu cau mot H1 title de script de doc va project import co ten ro rang.

### 5.2 Section header

Canonical syntax cua Video Prepare:

```markdown
## S01 - 0:00-0:20
```

Trong do:

- `S01` la stable section ID;
- `0:00` la planned start;
- `0:20` la planned end.

Video Prepare se dung timing nay cho validation va metadata. Timing khong duoc duplicate trong SCENES.

Validation bo sung cua Video Prepare:

- start phai nho hon end;
- section phai theo thu tu thoi gian tang dan;
- v1 khong chap nhan overlap giua hai section lien tiep;
- gap co the chap nhan;
- ID phai unique.

### 5.3 Section title

```markdown
### Silence is not weakness
```

Day la OmniVoice metadata theo behavior hien tai. Co doc title hay khong la runtime audio setting cua OmniVoice, khong phai script config cua Video Prepare.

### 5.4 Style directives

Vi du:

```markdown
[WARM] ...
[SOFT] ...
[EMPHASIZE] ...
[DEFAULT] ...
[WHISPER] ...
[LOW_PITCH] ...
[HIGH_PITCH] ...
```

Video Prepare khong tu dien giai style. Noi dung duoc chuyen sang OmniVoice de canonical parser cua OmniVoice xu ly.

### 5.5 Khong tron visual directive vao OmniVoice

Khong duoc viet:

```markdown
[PEXELS]
[VIDEO]
[IMAGE]
[QUERY]
```

Visual metadata chi nam trong SCENES.

Ly do: OmniVoice co directive parser rieng. Tron visual metadata vao narration co the vo tinh thay doi beat/chunk boundaries va lam audio behavior kho du doan.

## 6. Runtime settings khong nam trong script

Nhung field sau day tuyet doi khong thuoc Script Format v1:

```text
Data Root
Visual Flow ON/OFF
Audio Flow ON/OFF
Pexels API Key
OmniVoice URL
OmniVoice API Token
Voice name
Voice variant
Language runtime override
Quality preset
Read section titles
Download concurrency
Retry policy
```

Tat ca phai duoc quan ly qua Runtime Settings UI.

Ly do quan trong voi OmniVoice URL:

- OmniVoice co the chay tren Kaggle hoac Colab;
- trycloudflare/Gradio public URL co the thay doi sau moi restart;
- user chi can paste URL moi trong UI;
- project va script khong duoc thay doi;
- request moi se dung client/runtime config moi.

## 7. Validation pipeline

Khi import script, Video Prepare phai validate theo thu tu:

```text
1. file readable UTF-8
2. exact SCENES marker exists once
3. exact OMNIVOICE marker exists once
4. marker order is correct
5. SCENES YAML parses
6. format_version == 1
7. scene IDs unique
8. visual IDs unique within scene
9. visual media valid
10. queries non-empty
11. count >= 1
12. parse OmniVoice section headers for cross-validation
13. OmniVoice IDs unique
14. timing valid and non-overlapping
15. Scene IDs == OmniVoice Section IDs
16. OmniVoice body non-empty
```

Neu mot validation fail, khong chay visual flow va khong import audio project.

## 8. Error examples

### Missing OmniVoice section

SCENES:

```yaml
- id: S03
```

nhung OMNIVOICE chi co S01 va S02.

Error:

```text
SCRIPT_SCENE_SECTION_MISMATCH
Scene S03 has no matching OmniVoice section.
```

### Missing scene

OMNIVOICE co:

```markdown
## S04 - 1:00-1:20
```

nhung SCENES khong co S04.

Error:

```text
SCRIPT_SCENE_SECTION_MISMATCH
OmniVoice section S04 has no matching scene.
```

### Invalid media

```yaml
media: gif
```

Error:

```text
SCRIPT_INVALID_MEDIA
S01/V01 media must be image, video, or either.
```

### Empty query

```yaml
queries: []
```

Error:

```text
SCRIPT_EMPTY_QUERY
S01/V01 requires at least one non-empty query.
```

### Overlapping narration timing

```markdown
## S01 - 0:00-0:20
## S02 - 0:15-0:40
```

Error:

```text
SCRIPT_TIMELINE_OVERLAP
S02 starts before S01 ends.
```

## 9. Canonical in-memory model

Parser co the chuyen file thanh model toi thieu:

```text
PreparedScript
  format_version
  scenes[]
    id
    visuals[]
      id
      media
      queries[]
      count
  omnivoice
    raw_markdown
    title
    sections[]
      id
      start
      end
```

`omnivoice.raw_markdown` la phan can gui sang API import cua OmniVoice.

Khong can parse Beat/Chunk trong Video Prepare. Do la noi bo cua OmniVoice.

## 10. Project snapshot

Khi tao project local, input goc nen duoc snapshot immutable:

```text
projects/<project-id>/
  input/
    script.vprep
```

Runtime state va output nam ngoai snapshot:

```text
projects/<project-id>/
  input/script.vprep
  project.json
  status.json
  scenes/
  audio/
  logs/
```

Khong ghi API key/token vao snapshot, project.json, status.json hoac logs.

## 11. Change detection

V1 nen tinh SHA-256 cua input file.

```text
input_sha256
```

Neu user mo lai project va source script da thay doi, app phai bao ro script changed thay vi am tham coi output cu la valid.

Future version co the tinh fingerprint tung scene de invalidate co chon loc. V1 co the bat dau voi whole-file fingerprint de giu implementation don gian va deterministic.

## 12. Compatibility promise cua v1

Trong `format_version: 1`:

- hai top-level marker giu nguyen;
- `Scene ID == OmniVoice Section ID`;
- visual media enum giu `image|video|either`;
- runtime settings khong duoc chen vao script;
- OmniVoice block duoc giu native, khong bi Video Prepare rewrite;
- them provider moi khong duoc bat buoc thay doi script.

Neu can breaking change, tao `format_version: 2` thay vi am tham thay doi semantics cua v1.
