# Backup Mic 경로 규칙, 실시간 미리보기, 안전 취소 및 UI 밀도 설계

## 배경

현재 녹음기 규칙은 파일명 prefix와 suffix를 지원하지만 날짜 폴더 구조는 `YYYY/MM/`로 고정되어 있다. 규칙 편집기의 미리보기 역시 `directory/YYYY/MM/YYMMDD-ZOOM0001.m4a`를 고정 예시로 표시해 DJI 파일명 축약, 사용자가 입력한 archive 이름, M4A 변환 설정과 실제 저장 경로를 충분히 반영하지 않는다.

또한 사용자가 수동 재탐색을 시작하면 stable scan, 복사, 검증과 변환이 끝날 때까지 UI에서 취소할 수 없다. 실제 연결 녹음기로 설치본을 검증하는 과정에서도 장시간 스캔이 시작될 수 있으므로, 원본 안전성과 자동 휴지통 정책을 유지하면서 현재 작업을 명시적으로 중단할 수 있어야 한다.

설정 화면은 작은 보조 텍스트 옆에 일부 버튼과 control text만 크게 보여 위계가 불안정하다. 기존 macOS 메뉴 막대 유틸리티의 색, SF 계열 글꼴, 반투명 surface와 signal-path 성격은 유지하되 버튼 높이, 글자 크기와 간격을 한 리듬으로 정리한다.

## 목표

- 녹음기별 날짜 폴더 구조로 정확히 세 가지 preset을 제공한다.
  - `YYYY/MM/DD/`
  - `YYYY/MM/`
  - `YYMMDD/`
- 기존 규칙과 DJI Mic Mini 2S 사전 설정은 `YYYY/MM/`로 동작을 보존한다.
- 새 녹음에는 선택된 폴더 구조를 적용하고, 기존 ledger artifact 경로는 그대로 재사용한다.
- 파일명 미리보기가 archive, 날짜 구조, 녹음기 profile, prefix, suffix와 출력 형식을 즉시 반영한다.
- 스캔과 백업 중 사용자가 안전하게 취소할 수 있게 한다.
- 취소 요청 뒤 원본 파일을 자동 휴지통으로 이동하지 않는다.
- 버튼, 입력 control, label과 간격을 기존 macOS UI 안에서 일관되게 정리한다.

## 범위 밖

- 정규식 기반의 임의 경로 템플릿 언어를 추가하지 않는다.
- 세 가지 이외의 사용자 정의 날짜 token 조합을 지원하지 않는다.
- source 파일명 자체를 임의의 정규식으로 변환하지 않는다.
- 기존 archive를 새 구조로 이동하거나 재배치하지 않는다.
- companion evidence 파일의 `source-extras` 구조를 변경하지 않는다.
- 자동 백업 또는 자동 휴지통 설정을 사용자 동의 없이 변경하지 않는다.
- 앱 전체의 색상, 서체 또는 정보 구조를 새 디자인 시스템으로 교체하지 않는다.

## 날짜 폴더 모델

Rust core와 frontend contract에 `date_folder_layout`을 추가한다. 저장 값은 API와 SQLite에서 동일하게 사용한다.

| 저장 값 | UI 표시 | 2026년 8월 12일 결과 예시 |
| --- | --- | --- |
| `year_month_day` | `YYYY/MM/DD/` | `DJI Mic Mini 2S/2026/08/12/260812-T01_MIC001_20260812_120000.m4a` |
| `year_month` | `YYYY/MM/` | `DJI Mic Mini 2S/2026/08/260812-T01_MIC001_20260812_120000.m4a` |
| `compact_date` | `YYMMDD/` | `DJI Mic Mini 2S/260812/260812-T01_MIC001_20260812_120000.m4a` |

Rust enum은 `DateFolderLayout::{YearMonthDay, YearMonth, CompactDate}`로 둔다. 폴더 구성은 recording destination을 계산하는 core 함수 한 곳에서 처리해 실제 백업, 테스트와 향후 caller가 같은 규칙을 사용하게 한다.

SQLite migration은 기존 row와 새 row에 `year_month` 기본값을 사용한다. 이 선택으로 배포 전 생성한 사용자 규칙, DJI 사전 설정과 현재 archive 경로가 변경 없이 유지된다. DJI preset을 복원하거나 새로 생성할 때도 `year_month`를 명시한다.

날짜 구조 변경은 변경 뒤 처음 처리하는 새 recording에만 적용한다. ledger에 이미 안전한 artifact 경로가 저장되어 있으면 현재 no-clobber와 evidence 재사용 경계를 유지하고 새 레이아웃으로 파일을 복제하거나 이동하지 않는다.

## 실제 저장 경로와 파일명

최종 recording 경로는 다음 순서로 구성한다.

```text
archive directory
  / selected date folders
  / YYMMDD-{prefix}{profiled source stem}{suffix}.{artifact extension}
```

- 날짜는 recording metadata에서 기존에 선택하던 effective date를 사용한다.
- 파일명 앞의 `YYMMDD-`는 세 폴더 preset 모두에서 유지한다.
- prefix와 suffix는 녹음기 규칙별 값이며 profile 처리된 source stem 앞뒤에 붙인다.
- DJI profile은 기존처럼 `TX01_`을 `T01_`, `TX02_`를 `T02_`로 줄인다.
- M4A 변환이 켜져 있으면 `.m4a`, 꺼져 있으면 원본 recording 확장자인 `.wav`를 쓴다.
- companion 파일은 기존 `archive/source-extras/...` 경로와 원본 추적 규칙을 유지한다.

## 실시간 파일명 미리보기

규칙 편집기의 미리보기를 입력 form 아래의 작은 signal-path card로 정리한다. 미리보기는 network나 실제 녹음기 파일을 읽지 않는 순수 frontend formatter로 즉시 계산한다. 실제 파일명을 노출하지 않으면서 profile에 맞는 대표 입력을 사용한다.

DJI preset 또는 DJI profile 규칙은 다음 source 예시를 사용한다.

```text
TX01_MIC001_20260812_120000.WAV
```

일반 규칙은 다음 source 예시를 사용한다.

```text
ZOOM0001.WAV
```

미리보기는 두 줄을 제공한다.

- `원본 예시`: profile별 대표 source filename
- `백업 결과`: 선택한 archive, 오늘 날짜, 날짜 폴더 구조, profile 축약, prefix, suffix와 현재 M4A 설정을 모두 적용한 최종 상대 경로

archive 이름, 날짜 폴더 select, prefix, suffix 또는 M4A 설정이 바뀌면 저장 버튼을 누르기 전에도 결과가 즉시 갱신된다. 긴 경로는 wrapping과 monospace 처리로 전체 의미를 유지한다. 별도의 placeholder 경로와 실제 결과를 혼합하지 않는다.

규칙 목록의 destination 설명도 고정 `YYYY/MM` 대신 선택된 레이아웃 label을 표시한다.

## 스캔 및 백업 취소

### 사용자 흐름

`scanning`, `copying`, `verifying`, `converting` 등 실제 작업 phase에서는 footer의 비활성 `지금 백업` 대신 경고색이 아닌 중립 secondary `백업 취소` 버튼을 표시한다. 사용자가 누르면 즉시 `취소 중…`으로 바뀌고 snapshot이 settled phase로 전환될 때까지 재입력을 막는다.

header의 `다시 확인 및 백업`은 작업 중 계속 비활성화한다. `Command-R`은 WebView reload를 막지만 중첩 backup을 시작하지 않는다. 취소 완료 뒤에는 연결된 녹음기가 mounted idle 상태로 돌아가 다시 수동 백업할 수 있다.

### 명령과 operation token

Tauri에 인자 없는 `cancel_backup` command를 추가한다. command는 새로운 파일 경로나 source 식별자를 받지 않고 process-wide `AppState`의 현재 cancellation token만 취소한다.

```text
백업 취소 클릭
  -> React single-flight cancel state
  -> cancel_backup
  -> AppState current CancellationToken.cancel()
  -> scanner/copy/verify/convert checkpoint가 Cancelled 반환
  -> orchestrator가 Cancelled를 일반 오류와 분리
  -> mounted source와 전체 snapshot을 idle + operation_cancelled로 정리
  -> UI가 settled 상태로 복귀
```

실제 operation이 없지만 오래된 snapshot만 active로 남은 경우 command가 즉시 cancellation settlement를 수행한다. 실제 operation이 있으면 blocking 작업이 token을 관찰한 뒤 같은 settlement 경로를 사용한다. 중복 취소는 idempotent하게 다룬다.

### stable scan cancellation

두 번의 directory scan 사이 안정화 대기에도 cancellation token을 전달한다. 다음 지점에서 취소를 확인한다.

- 첫 scan 시작 전과 각 directory entry 처리 중
- 첫 scan 종료 뒤 안정화 sleep 전과 후
- 두 번째 scan의 각 directory entry 처리 중
- scan 결과를 backup pipeline으로 전달하기 직전

기존 scheduler의 빠른 one-pass fingerprint scan은 operation token과 분리된 internal helper를 사용한다. 수동·자동 backup의 stable scan만 현재 operation token에 연결한다.

### 원본과 자동 휴지통 안전

orchestrator는 `run_backup`의 recording 처리와 automatic retirement 사이에서 cancellation을 다시 확인한다. 따라서 복사와 검증이 이미 끝났더라도 취소가 요청됐으면 원본 session을 휴지통으로 이동하지 않는다.

취소 시 이미 완성되고 검증된 destination artifact는 ledger evidence와 함께 남겨 재실행에서 안전하게 재사용한다. 임시 파일은 기존 rollback/cleanup 경로를 따른다. source 원본은 취소 처리 자체로 삭제하거나 이동하지 않는다.

취소는 `error`가 아니다. snapshot은 다음 상태로 정리한다.

- overall, mounted source와 transmitter phase: `idle`
- message code: `operation_cancelled`
- current/failure stage: 없음
- progress: 초기값
- error: 없음
- deletion ready와 retirement active: `false`
- activity: privacy-safe `backup_cancelled` 한 건

## UI 밀도와 시각 위계

기존 색상, SF Pro Text/Rounded/Mono 서체, rounded cards, 얇은 border와 메뉴 막대 utility 정체성은 유지한다. 변경은 compact control rhythm에 집중한다.

### 크기 체계

- settings title: 22px
- section heading: 13px
- body와 버튼/control text: 11px
- caption과 metadata: 10px
- 기본 control 높이: 32px
- icon-only control: 32px 정사각형
- 기본 간격: 4px, 8px, 12px, 16px 단계

공용 Button의 기본 글자 크기와 높이를 이 체계에 맞추고, 명시적 큰 CTA가 필요한 경우만 별도 size를 사용한다. 기존 10–12px 설정 text 옆에서 14px button label만 튀는 상태를 제거한다. 입력, select, 버튼의 baseline과 radius를 맞춰 같은 form row 안에서 한 제품처럼 보이게 한다.

### 경로 미리보기 signature

미리보기 card는 `원본 예시 → 백업 결과`라는 하나의 signal path를 만든다. accent 색은 arrow/icon과 결과의 핵심 경로에만 제한적으로 사용한다. 나머지 설정 surface는 현재의 차분한 contrast를 유지한다. 지나친 badge, gradient, 그림자나 decorative card를 새로 추가하지 않는다.

### 접근성과 반응형 동작

- 날짜 폴더 select는 visible label과 저장 값이 일대일로 대응한다.
- 취소, 새로고침과 저장 버튼은 구체적인 accessible name을 유지한다.
- 색만으로 phase나 action을 전달하지 않고 text와 icon을 함께 사용한다.
- 긴 경로는 viewport를 밀어내지 않고 wrapping한다.
- hover, focus-visible, disabled와 pending 상태를 현재 light/dark palette 모두에서 구분한다.

## 오류 처리

- 알 수 없는 `date_folder_layout`은 deserialization 또는 SQLite hydration에서 조용히 임의 처리하지 않고 명확한 invalid-data 오류로 거부한다.
- 기존 migration row는 `year_month`로 채워 예측 가능한 호환성을 보장한다.
- cancellation command 실패는 popover inline action error로 표시하되 현재 backend snapshot을 계속 구독한다.
- cancellation race에서 operation이 이미 완료됐다면 완료 snapshot을 유지하고 별도의 거짓 오류를 만들지 않는다.
- 취소 직후 source가 unmount된 경우 mounted row를 강제로 복원하지 않고 현재 authority에 맞는 idle/disconnected snapshot을 만든다.

## 테스트와 검증

### Core와 ledger

- 세 enum 저장 값의 serde와 display contract를 검증한다.
- 같은 recording date와 source stem에 대해 세 경로가 정확히 생성되는지 검증한다.
- prefix, suffix와 DJI profile이 세 레이아웃 모두에서 유지되는지 검증한다.
- migration 전 규칙이 `year_month`로 hydrate되는지 검증한다.
- 새 규칙 create/update/list와 DJI preset restore가 레이아웃을 보존하는지 검증한다.
- 기존 ledger artifact가 레이아웃 변경 뒤에도 같은 경로로 재사용되는지 회귀 검증한다.

### Cancellation

- stable scan이 sleep 전후와 entry 순회 중 cancellation을 관찰하는지 검증한다.
- `cancel_backup`이 active token을 취소하고 snapshot을 idle로 정리하는지 검증한다.
- `CoreError::Cancelled`가 error snapshot이 되지 않는지 검증한다.
- copying 완료 뒤 cancellation이 automatic retirement를 실행하지 않는지 검증한다.
- UI 취소 버튼이 한 번만 command를 호출하고 `취소 중…`으로 바뀌며 header refresh를 비활성화하는지 검증한다.

### 미리보기와 UI

- 세 select option과 `YYYY/MM/` 기본값을 검증한다.
- archive, layout, prefix, suffix와 M4A 변경이 즉시 정확한 결과 경로에 반영되는지 검증한다.
- DJI preview가 `TX01_`을 `T01_`로 변환하는지 검증한다.
- 규칙 목록이 선택한 layout label을 표시하는지 검증한다.
- 공용 버튼과 주요 settings control이 compact type/height class를 사용하는지 검증한다.

### 전체 및 설치

- frontend 전체 테스트와 production build를 실행한다.
- Rust workspace test와 format check를 실행한다.
- local package를 빌드하고 app signature, architecture와 executable hash를 확인한다.
- 설치본을 재설치하고 실행하되 연결된 실제 recorder에서 새 backup을 시작하지 않는다.
- source WAV 수와 설정 값이 검증 전후 동일한지 확인한다.
- 새 DMG를 `~/Downloads`로 안전하게 옮기고 `hdiutil verify`와 SHA-256을 기록한다.
- 각 의미 단위 커밋을 push하고 clean tree, tracking upstream과 live remote `0 0`을 증명한다.

## 완료 기준

- 사용자는 각 규칙에서 정확히 세 날짜 폴더 구조 중 하나를 선택할 수 있다.
- 기존 규칙과 DJI preset은 업그레이드 뒤에도 `YYYY/MM/`를 사용한다.
- 미리보기가 실제 profile과 현재 출력 설정을 반영해 입력 즉시 갱신된다.
- 작업 중 취소 버튼이 보이며 취소 뒤 error가 아닌 idle 상태로 돌아간다.
- 취소 요청 이후 원본이 자동 휴지통으로 이동하지 않는다.
- 버튼 text, control 높이와 간격이 설정 화면 전체에서 일관된다.
- 전체 테스트, package, 설치, 실행, DMG와 Git 원격 동기화 검증이 완료된다.
