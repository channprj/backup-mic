# Backup Mic 빠른 재개 백업, 보관 폴더 변경 및 증거 복구 설계

**Date:** 2026-08-12
**Status:** Approved
**Target:** macOS 13+, Backup Mic menu-bar app

## 배경과 확인된 원인

DJI-MIC-1에는 20개의 WAV가 있었지만 사용자에게는 백업이 시작되지 않는 것처럼 보였다. 실제 디렉터리 열거와 metadata 수집은 약 0.15초였으나 `scanning` phase 안에서 다음 작업이 직렬로 수행됐다.

- 녹음기마다 두 번 전체 스캔하고 그 사이 2초 대기
- 모든 source WAV의 SHA-256 계산
- ledger evidence를 찾은 뒤 같은 source WAV의 SHA-256 재계산
- 기존 verified M4A를 재사용할 때 source와 artifact를 다시 검증

그 결과 첫 DJI-MIC-1 실행은 run을 만들기 전에 실패했고 상세 failure code가 activity에 남지 않았다. 다음 실행은 사용자가 16초 뒤 취소했으며, 마지막 실행에서 취소 전에 검증된 2개를 재사용하고 나머지 18개, 4,349,997,168 bytes를 처리했다. 최종 실행은 20개를 모두 검증하고 외장 디스크 원본 세션을 recoverable Trash로 이동했다.

추가로 기존 archive layout migration이 ledger evidence 18개와 활성 목적지 파일을 불일치 상태로 남긴 것이 확인됐다. 그중 DJI-MIC-1 녹음 13개는 외장 디스크 Trash에 source WAV가 보존되어 있다. 자동 경로 이동은 사용자에게 승인된 "기존 파일은 그대로 두고 새 설정은 앞으로의 파일에만 적용" 정책과도 맞지 않는다.

## 목표

- 파일 발견은 파일 개수나 용량에 관계없이 metadata 중심으로 빠르게 끝낸다.
- source content는 복사 또는 기존 artifact 검증 시 한 번의 streaming pass로 확인한다.
- 취소된 실행에서 이미 검증된 artifact를 다음 실행이 그대로 재사용한다.
- 여러 녹음기의 안정성 검사는 녹음기 수만큼 대기 시간을 누적하지 않는다.
- 보관 폴더 이름과 날짜 구조를 바꾸면 기존 artifact는 이동하지 않고 새 파일에만 새 규칙을 적용한다.
- startup/backup 경로에서 verified artifact를 자동 이동하거나 Trash로 보내지 않는다.
- source별 준비 실패의 진단 코드를 activity, failure log와 snapshot에 보존한다.
- 현재 복구 가능한 13개 DJI-MIC-1 녹음을 원본 Trash 항목을 변경하지 않고 목적지에 복구한다.
- 설정 화면 버튼, input, select의 compact typography와 spacing을 일관되게 유지한다.

## 범위 밖

- SHA-256, no-clobber publish, audio property 검사 또는 source retirement preflight를 약화하지 않는다.
- 자동 백업과 자동 휴지통 사용자 설정을 변경하지 않는다.
- 기존 artifact를 새 archive name이나 날짜 layout으로 일괄 이동하지 않는다.
- 외장 디스크 Trash 원본을 복원하거나 영구 삭제하지 않는다.
- ledger evidence와 source가 모두 없는 과거 artifact를 추측으로 재생성하지 않는다.

## 빠른 발견과 준비

### 1. 공통 안정성 구간

모든 matched source를 먼저 한 번씩 metadata scan하고, 한 번의 2초 stability interval을 공유한 뒤 모든 source를 다시 scan한다. 각 source에서 path, size, mtime, file identity가 두 snapshot에 동일한 파일만 후보로 선택한다.

```text
scan A: source 1, source 2, ... source N
single 2-second interval
scan B: source 1, source 2, ... source N
stable intersection per source
```

이 구조는 두 개의 녹음기가 연결돼도 4초 이상 대기하지 않는다. cancellation은 각 directory entry, 공통 대기 전후와 결과 반환 전에 계속 검사한다.

### 2. Metadata-first ledger lookup

ledger는 `(source_id, relative_path, size, mtime)`로 기존 recording/additional-file evidence를 찾는 API를 제공한다. 이 조회는 content hash가 없더라도 후보를 돌려주지만, 후보를 신뢰 완료 상태로 간주하지 않는다.

- 기존 evidence가 있으면 recorded source SHA-256을 계획에 사용한다.
- 기존 artifact path는 현재 archive name과 달라도 safe relative path이고 canonical destination root 안의 regular file이면 재사용 후보가 된다.
- 실행 단계는 live source를 한 번 streaming hash하고 recorded source hash와 비교한다.
- artifact도 recorded size/hash 및 audio properties와 다시 비교한다.
- 검증 중 source metadata가 달라지면 `source_changed`로 실패한다.

신규 파일은 기본 destination path의 존재 여부만 metadata로 판정한다. 비어 있는 path이면 source를 미리 hash하지 않고 copy stream이 SHA-256을 동시에 계산한다. 충돌한 path에서만 기존 파일 식별과 hash suffix 결정을 위해 필요한 hash 작업을 수행한다. 계획에 아직 hash가 없는 경우 execution이 계산한 digest로 최종 no-clobber path를 선택하고 publish한다.

### 3. 단계 표시

`scanning`은 directory/metadata 발견과 공통 stability interval까지만 의미한다. discovery가 끝나면 snapshot을 준비/검증 상태로 전환한다. 기존 public phase를 불필요하게 확장하지 않고 `current_stage`와 message code로 다음을 구분한다.

- `scanning`: 백업 후보 경로와 metadata 확인
- `source_verification`: 기존 증거와 source content 확인
- `copy`: 새 source streaming copy
- `artifact_verification`: destination hash/audio 확인
- `source_revalidation`: Trash 직전 독립적인 전체 재검증

자동 Trash의 마지막 source/artifact 재검증은 삭제 안전 경계이므로 그대로 유지한다.

## 재개와 설정 변경

### 기존 artifact 재사용

verified artifact의 ledger relative path는 생성 당시의 불변 증거다. 현재 rule의 archive folder 또는 date layout과 일치하는지는 재사용 조건이 아니다. 다음 조건을 모두 만족할 때 exact ledger path를 재사용한다.

- safe relative path
- canonical destination root containment
- regular non-symlink file
- ledger byte count와 SHA-256 일치
- M4A이면 recorded audio properties 일치
- live source metadata와 SHA-256 일치

하나라도 실패하면 source 원본은 그대로 두고 명시적 failure를 반환한다. 기존 M4A가 없거나 변조됐다는 이유로 같은 ledger row를 조용히 덮어쓰지 않는다.

### 보관 폴더 이름 변경

`archive_directory_name` input은 evidence 존재 여부와 관계없이 편집 가능하다. 저장 시 safe single relative component validation을 그대로 적용한다.

- 기존 recording/additional artifact는 이동하지 않는다.
- 새 recording과 companion 파일만 저장 시점의 archive folder를 사용한다.
- DJI 기본값 복원은 사용자가 선택한 archive folder name을 유지한다.
- `archive_directory_locked` DB column과 DTO는 이전 schema 호환을 위해 유지할 수 있지만 archive rename을 거부하거나 UI를 disable하는 용도로 사용하지 않는다.

### 자동 migration 제거

startup과 backup 시작에서 `flatten_verified_recording_layout` 및 `migrate_legacy_rule_layout`을 실행하지 않는다. 기존 함수와 migration regression tests는 삭제하거나 read-only audit helper로 대체한다. 앱이 기존 artifact를 자동 복사·Trash·relocate하지 않는다는 회귀 테스트를 둔다.

## 실패 관측성

run 생성 전 source preparation이 실패해도 다음을 남긴다.

- source별 snapshot error와 public `message_code`
- `partial_failure` activity entry
- destination 또는 fallback daily failure log의 `operation=backup_run`, `stage=source_preparation`, diagnostic code

privacy boundary에 따라 volume path, UUID, source path, hash는 frontend와 activity text에 노출하지 않는다. 같은 실행에서 다른 source가 안전하게 진행할 수 있는 오류는 해당 source만 실패시키고, ledger/destination/cancellation 오류는 process-wide 처리한다.

## 기존 데이터 복구

복구는 production backup pipeline을 우회해 ledger를 임의 수정하지 않는다. 별도의 명시적 로컬 recovery tool은 다음 manifest를 입력받는다.

- exact source file path
- expected source size와 SHA-256
- ledger recording id
- destination root

각 항목은 다음 순서로 처리한다.

1. source가 recorder Trash 안의 regular non-symlink file인지 확인한다.
2. source size와 SHA-256을 ledger source evidence와 비교한다.
3. source WAV를 app-owned temporary file로 destination의 현재 rule path에 복사하고 flush/sync한다.
4. 복사본 hash를 source evidence와 비교한다.
5. 현재 AAC-LC 128 kbps profile로 M4A를 생성한다.
6. M4A audio properties, size와 SHA-256을 검증한다.
7. no-clobber 방식으로 publish한 뒤 ledger artifact evidence를 새 M4A에 atomically 갱신한다.
8. source Trash WAV와 기존 ledger evidence는 성공 전까지 변경하지 않는다.
9. 모든 항목 결과를 privacy-safe recovery report로 남긴다.

복구 대상은 현재 source가 실제 확인된 DJI-MIC-1 항목 13개로 제한한다. 나머지 5개는 source 또는 verified artifact를 찾을 수 없으므로 이번 복구에서 제외하고 진단 상태로 보고한다.

## UI 일관성

- 보관 폴더 input은 다른 rule text input과 동일한 11px control type, 32px height를 사용한다.
- rule toolbar와 editor action button은 공용 `size="sm"` rhythm을 사용한다.
- 버튼 안 spinner/icon은 text baseline과 동일한 gap을 사용한다.
- 경고/설명 문구는 10px caption, 본문과 버튼은 11px을 유지한다.
- archive rename 설명으로 "기존 백업은 그대로 유지되고 새 파일부터 적용"을 input 아래에 표시한다.
- light/dark theme의 기존 palette와 focus-visible contrast를 유지한다.

## 테스트 전략

### Rust core와 orchestration

- 두 source의 stable scan이 한 번의 2초 interval만 사용하는지 검증한다.
- metadata-only discovery가 source content를 열지 않는지 instrumented test로 검증한다.
- 기존 M4A를 archive/date-layout 변경 뒤 exact old path에서 재사용하는지 검증한다.
- 취소 전 commit된 2개와 미처리 18개가 다음 run에서 2개 재사용 + 18개 처리로 이어지는지 검증한다.
- 기존 artifact missing/hash mismatch는 덮어쓰지 않고 실패하는지 검증한다.
- startup과 backup이 기존 artifact를 이동하거나 Trash로 보내지 않는지 검증한다.
- run 전 preparation error가 source snapshot, activity와 failure reporter에 남는지 검증한다.
- whole-session retirement는 현재 snapshot 전체가 freshly verified된 경우에만 허용되는 기존 barrier tests를 유지한다.

### Rule과 frontend

- locked evidence가 있는 DJI/custom rule 모두 archive folder rename을 저장하는지 검증한다.
- rename 뒤 기존 evidence path가 그대로이고 새 destination만 새 archive를 사용하는지 검증한다.
- DJI preset restore가 사용자 archive folder를 보존하는지 검증한다.
- archive input이 enabled이고 안내 문구가 보이는지 검증한다.
- rule action button typography/spacing contract를 검증한다.
- scan, preparation failure와 retry copy를 사용자가 구분할 수 있는지 검증한다.

### 복구와 최종 검증

- disposable fixture로 recovery tool의 source mismatch, destination collision, conversion failure와 success를 검증한다.
- 실제 복구 전 13개 source의 size와 SHA-256을 ledger와 전수 대조한다.
- 실제 복구 후 13개 destination artifact의 ledger size/hash/audio evidence를 전수 대조한다.
- recorder Trash의 13개 source가 그대로 남아 있는지 확인한다.
- repository 전체 checks, production build, package, signature, DMG, install과 installed executable hash를 검증한다.
- 설치본을 실행하되 실제 recorder backup을 자동으로 시작하지 않는다.
- 각 의미 단위를 Conventional Commit으로 즉시 push하고 최종 local/tracking/live remote `0 0`을 증명한다.

## 완료 기준

- 두 recorder 발견 대기는 최대 한 번의 stability interval이고 discovery에서 전체 WAV를 hash하지 않는다.
- DJI-MIC-1 같은 대용량 세션에서 즉시 파일 개수/진행 단계가 표시되며 취소 후 재실행이 완료 증거를 재사용한다.
- archive folder rename과 날짜 layout 변경이 기존 artifact를 이동하지 않는다.
- 기존 artifact를 자동 이동하는 production 경로가 없다.
- source preparation failure 원인을 이후에도 확인할 수 있다.
- 복구 가능한 13개가 목적지와 ledger에서 검증되고 source Trash 원본은 보존된다.
- UI 글자 크기와 spacing이 compact control scale에 맞는다.
- 빌드, 설치, 실행과 Git 원격 동기화 검증이 완료된다.
