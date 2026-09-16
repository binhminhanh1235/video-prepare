# State, Retry and Resume Contract

Status: DRAFT CONTRACT

## 1. Muc tieu

Video Prepare phai co the dong app, crash, mat mang, thay OmniVoice URL, mo lai project va tiep tuc ma khong lam lai phan da hoan tat.

Persisted state la source of truth cho local progress.

## 2. Task states

Canonical states v1:

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

### PENDING

Chua bat dau hoac da duoc dua ve trang thai san sang retry.

### RUNNING

Dang thuc thi trong process hien tai.

RUNNING khong duoc tu dong coi la success sau restart.

### PARTIAL

Da co mot phan output hop le nhung chua dat yeu cau day du.

Vi du visual request can 3 asset nhung moi tai duoc 2.

### COMPLETED

Task da co output day du va output can thiet van ton tai/hop le.

### FAILED

Da co ket qua terminal that bai ro rang cho attempt hien tai.

### SKIPPED

Khong chay do runtime policy, vi du Visual Flow OFF.

SKIPPED khong phai error.

### INTERRUPTED

Task dang RUNNING nhung process/local runtime bi ngat truoc khi co terminal result.

### UNKNOWN_REMOTE

Da submit remote work nhung app khong the xac dinh terminal state cua remote job, vi du OmniVoice URL cu khong con truy cap duoc sau Kaggle restart.

Khong fabricate FAILED hoac COMPLETED trong truong hop nay.

## 3. Project aggregate status

Project co the derive:

```text
READY
RUNNING
PARTIAL
COMPLETED
FAILED
```

Goi y:

- tat ca flow enabled completed -> COMPLETED;
- co task running -> RUNNING;
- co output completed nhung con failed/partial/interrupted -> PARTIAL;
- chua run -> READY;
- FAILED chi dung khi khong co progress usable va terminal failures chan flow, tranh lam mat thong tin partial success.

## 4. Per-scene state

Vi du:

```json
{
  "id": "S02",
  "visual": {
    "status": "PARTIAL",
    "requests": [
      {
        "id": "V01",
        "status": "PARTIAL",
        "required_count": 2,
        "completed_count": 1,
        "attempts": 2
      }
    ]
  },
  "audio": {
    "status": "COMPLETED",
    "omnivoice_section_id": "S02"
  }
}
```

## 5. Attempt model

Moi network/provider execution nen co attempt record.

Toi thieu:

```text
attempt_id
flow
scene_id
visual_id optional
started_at
finished_at optional
status
error optional
remote_job_id optional
provider_asset_ids optional
```

Secret khong duoc ghi vao attempt.

## 6. Visual resume

Moi downloaded asset can provenance toi thieu:

```text
provider
provider_asset_id
query
media_type
local_relative_path
checksum optional but recommended
```

Khi retry PARTIAL visual:

```text
required_count = 3
existing valid = 2
missing = 1
```

Chi tim/tai them 1 asset.

Khong mac dinh xoa 2 asset cu.

Can de-duplicate theo provider asset ID va local checksum neu co.

## 7. Audio resume

Sau `POST /projects/import`, persist ngay:

```text
omnivoice_project_id
source_hash if returned
server base URL used for import
```

Sau `POST /projects/{id}/generate`, persist ngay:

```text
job_id
submitted_at
```

Sau do polling/wait co the bi mat ket noi ma khong mat identity cua remote work.

## 8. OmniVoice URL changed

Scenario:

```text
URL A
POST generate -> job_id=123
Kaggle restarts
URL A dead
user sets URL B
```

Khong duoc gia dinh job 123 ton tai tren URL B.

Attempt cu:

```text
UNKNOWN_REMOTE
```

Sau do app co the:

1. Test URL B.
2. Import project idempotently vao server moi.
3. Submit generation moi.
4. Tao attempt moi.

Lich su attempt cu duoc giu de audit.

## 9. Startup reconciliation

Khi mo project:

```text
load status.json
verify local artifacts
reconcile stale RUNNING
reconcile remote audio attempts when possible
recompute aggregate status
```

Rule:

- local RUNNING tu process truoc -> INTERRUPTED;
- remote RUNNING co reachable server + job_id -> query job;
- remote server unreachable -> UNKNOWN_REMOTE;
- completed local file missing -> khong giu COMPLETED mot cach mu quang, mark needs retry/reconciliation.

## 10. Retry commands

UI v1 nen support:

```text
Resume Project
Retry Failed
Retry Scene
Retry Visual
Retry Audio
Retry Missing Visual Assets
```

`Retry Failed` chi retry task co status:

```text
FAILED
INTERRUPTED
```

`UNKNOWN_REMOTE` nen can reconciliation hoac explicit retry policy de tranh duplicate remote generation khong can thiet.

## 11. Atomic state write

`status.json` va `project.json` phai duoc ghi atomic:

```text
write status.json.tmp
flush
rename -> status.json
```

Khong de crash tao JSON half-written.

## 12. Logs

`logs/run.ndjson` nen append structured event:

```json
{"event":"visual.search.started","scene":"S01","visual":"V01","attempt":"..."}
{"event":"visual.asset.saved","scene":"S01","visual":"V01","provider":"pexels","asset_id":"..."}
{"event":"audio.job.submitted","project_id":"...","job_id":"..."}
```

Khong log:

```text
API keys
authorization tokens
full Authorization headers
```

## 13. Manual takeover

Visual request co the duoc hoan thanh bang local file do user chon.

Provenance:

```text
source = manual
local_relative_path = ...
```

Manual complete la output hop le, khong phai workaround bi an.

## 14. Invariants

- completed task khong bi chay lai neu khong co ly do invalidate;
- failed attempt khong duoc doi thanh success neu khong co output proof;
- network timeout khong duoc mac dinh coi la remote failure;
- unknown remote state duoc giu la unknown;
- flow nay fail khong xoa success cua flow kia;
- retry khong xoa artifact valid neu khong co explicit replace action.
