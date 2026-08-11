# Backup Mic 수동 녹음기 재탐색 및 감지 상태 복구 설계

## 배경

Backup Mic은 규칙에 맞는 외장 녹음기를 감지하면 공개 snapshot과 해당 source를 곧바로 `detecting`으로 바꾼다. 실제 백업 예약은 그 뒤에 자동 백업 설정, 초기 설정 완료, destination 유효성, 일치 source 존재 여부를 확인해 결정한다. 자동 백업이 꺼져 있으면 예약하지 않지만 `detecting` 상태를 정리하지 않는다.

React popover는 `detecting`을 실제 작업 중인 phase로 취급한다. 따라서 진행 카드가 계속 `녹음기를 확인하는 중`과 `확인 중`을 표시하고 기존 `지금 백업` 버튼도 비활성화한다. 현재 설치본에서 `DJI-MIC-1`과 `DJI-MIC-2`는 정상 source로 감지됐고 두 볼륨에 숨김·휴지통 파일을 제외한 WAV 20개가 있었지만, 자동 백업이 꺼진 상태에서 마지막 `device.detected` 이후 `scan.started`가 한 번도 기록되지 않았다.

이 변경은 거짓 작업 상태를 제거하고, 사용자가 버튼 또는 `Command-R`로 연결된 녹음기를 다시 안정적으로 스캔한 뒤 새 파일을 즉시 백업할 수 있게 한다.

## 목표

- 자동 백업이 꺼진 상태에서 감지된 녹음기가 영구적으로 `detecting`에 머무르지 않게 한다.
- 연결됐지만 작업하지 않는 source를 명확한 수동 백업 대기 상태로 표시한다.
- popover에 `다시 확인 및 백업` 버튼을 추가한다.
- popover가 활성화된 동안 `Command-R`이 버튼과 같은 동작을 수행하게 한다.
- 수동 동작은 기존의 안정 스캔, no-clobber 복사, SHA-256 검증, M4A 변환, source 재검증과 선택된 휴지통 정책을 그대로 사용한다.
- 클릭과 단축키의 중복 실행을 기존 operation guard와 UI pending 상태로 차단한다.
- 자동 재탐색과 수동 백업의 안전 조건, rule authority, destination authority와 삭제 경계를 약화하지 않는다.

## 범위 밖

- 별도의 파일 선택 목록이나 백업 전 검토 화면을 추가하지 않는다.
- 자동 백업 설정을 사용자의 선택 없이 켜지 않는다.
- 녹음기 규칙, glob 의미, 15초 fingerprint 재탐색 주기나 archive layout을 변경하지 않는다.
- 새로운 임의 경로 IPC 또는 webview filesystem 권한을 추가하지 않는다.
- settings window나 앱이 포커스되지 않은 상태의 전역 단축키를 추가하지 않는다.
- 설치 검증을 위해 실제 연결 녹음기의 파일을 자동으로 복사하거나 휴지통으로 옮기지 않는다.

## 상태 모델

장치 감지는 `matched source 존재`와 `백업 작업 실행 중`을 분리해 표현한다.

### 자동 백업이 시작되는 감지

다음 조건을 모두 만족하면 현재처럼 source와 전체 snapshot을 `detecting`으로 전환하고 백업을 예약한다.

- 자동 백업이 켜져 있다.
- 초기 설정이 `ready`다.
- 유효한 destination이 있다.
- 하나 이상의 규칙 source가 연결돼 있다.

예약된 작업은 operation guard로 직렬화된 뒤 `scanning`으로 전환한다. `detecting`은 실제로 백업 시작을 기다리는 짧은 상태에만 사용한다.

### 수동 백업을 기다리는 감지

규칙 source는 일치하지만 자동 백업이 꺼져 있으면 source와 전체 snapshot을 `idle`로 유지한다. source의 `mounted`는 `true`이고 channel row는 `연결됨`을 표시한다. settled card는 연결 source가 없을 때의 `연결 대기 중` 대신 다음 내용을 표시한다.

- 제목: `녹음기 연결됨`
- 설명: `다시 확인 및 백업을 눌러 새 녹음을 확인하세요`

이 상태는 실제 operation이 아니므로 수동 버튼과 `Command-R`을 사용할 수 있다. 자동 백업 설정을 다시 켜는 기존 경로는 현재 requirements를 재평가하고 백업을 시작한다.

### 작업 종료와 오류

수동 또는 자동 작업이 시작되면 `scanning` 이후의 기존 상태 전이를 그대로 따른다. 새 파일이 없으면 `nothing_new`, 검증된 파일이 있으면 `completed_deletion_pending`, source별 실패가 있으면 `partial_failure` 또는 `error`로 끝난다. 작업 시작 자체가 거부되면 기존 privacy-safe 지원 코드와 재시도 안내를 표시하며 거짓 `detecting` 상태를 만들지 않는다.

## 수동 재탐색 UI

popover header의 연결 개수 옆에 `RefreshCw` 아이콘 버튼을 추가한다.

- 접근 가능한 이름: `녹음기 다시 확인 및 백업`
- 보조 설명 또는 title: `녹음기 다시 확인 및 백업 (⌘R)`
- setup이 `ready`이고 실제 작업과 다른 UI action이 없을 때 활성화한다.
- 실행 중에는 spinner 또는 회전 상태를 표시하고 재입력을 막는다.

기존 footer의 `지금 백업`은 같은 동작을 제공하므로 유지한다. header 버튼은 상태 카드가 길어져도 항상 찾기 쉬운 재탐색 진입점이고, footer 버튼은 기존 사용 흐름과 하위 호환성을 보존한다.

`BackupPopover`는 버튼과 keyboard handler가 공유하는 하나의 `refreshAndBackup` 함수를 사용한다. 편집 가능한 input 계열이 아닌 곳에서 `Command-R`을 누르면 webview 기본 reload를 항상 `preventDefault`한다. 다음 조건을 모두 만족할 때만 백업 요청을 추가로 실행한다.

- macOS Command 키와 `R`이 함께 눌렸다.
- setup이 `ready`다.
- 실제 백업·복사·검증·휴지통 작업이 진행 중이지 않다.
- 다른 popover action이 pending이 아니다.
- event target이 input, textarea, select 또는 contenteditable이 아니다.

실행 조건을 만족하면 버튼과 동일한 단일 요청을 실행한다. 편집 가능한 target에서는 키 입력을 가로채지 않는다. setup 미완료, 실제 작업 중 또는 다른 action pending 상태에서는 app reload만 막고 새 백업을 중첩하지 않는다.

## 명령과 데이터 흐름

새 임의 command를 만들지 않는다. 버튼과 `Command-R`은 기존 `backup_now` Tauri command를 호출한다.

```text
button click 또는 Command-R
  -> React single-flight pending guard
  -> backup_now
  -> AppState backup readiness + process-wide operation guard
  -> 현재 matched source authority snapshot
  -> 각 source의 stable two-pass rule scan
  -> copy/verify/convert/revalidate/optional Trash pipeline
  -> snapshot event
  -> UI 완료 또는 오류 상태
```

`backup_now`는 현재 `matched_sources`의 canonical mount root를 Rust 내부에서 읽으므로 webview가 source 경로나 UUID를 보내지 않는다. Disk Arbitration이 감지한 현재 mount authority와 저장된 rule을 다시 검증한 뒤 파일을 탐색한다. 연결 source가 없거나 setup/destination이 준비되지 않았으면 현재의 좁은 오류로 거부한다.

## 자동 재탐색과의 관계

15초 `RescanScheduler`는 자동 백업이 켜져 있을 때 연결 중 새로 생긴 파일의 fingerprint 변화를 감지해 백업을 예약하는 기존 역할을 유지한다. 자동 백업이 꺼져 있으면 scheduler는 deadline을 연기하되 UI를 `detecting`으로 만들지 않는다.

수동 재탐색은 scheduler fingerprint 변화 여부를 기다리지 않는다. 사용자의 명시적 요청이므로 현재 matched source 전체를 기존 `backup_now` 경로에서 다시 안정 스캔한다. 작업 중 요청은 process-wide operation guard와 UI single-flight guard가 거부하거나 합치며 병렬 복사를 만들지 않는다.

## 휴지통과 안전 경계

수동 재탐색은 백업 종류만 수동일 뿐 저장된 전역 설정을 우회하지 않는다. 자동 휴지통 이동이 켜져 있으면 source별 전체 복사, M4A 변환, 최종 artifact 검증, source snapshot 재검증, durable ledger와 audit 경계가 모두 완료된 뒤 기존 정책대로 recoverable macOS Trash 이동을 수행한다.

설치와 런타임 검증 과정에서는 `Command-R`이나 새 버튼을 실제 연결 녹음기에 자동으로 실행하지 않는다. 현재 source를 변경하지 않은 채 다음을 확인한다.

- 재설치 후 자동 백업 OFF 상태가 `detecting`에 머무르지 않는다.
- 연결된 두 녹음기가 `연결됨`으로 표시된다.
- 새 버튼이 활성화되고 단축키 안내가 노출된다.
- 실제 파일 처리 계약은 isolated fixture와 기존 full pipeline 테스트로 검증한다.

## 오류 처리

- 연결 source가 없으면 기존 `device_removed` 계열 안내를 inline action error로 표시한다.
- destination 또는 setup이 준비되지 않았으면 기존 `invalid_request` 경계를 유지한다.
- 이미 작업 중이면 UI가 재입력을 차단하고 backend `Busy`가 최종 방어선이 된다.
- scan, copy, hash, conversion, ledger 또는 Trash 오류는 기존 source별 독립 실패와 원본 보존 규칙을 유지한다.
- keyboard handler는 실패를 삼키지 않고 버튼과 같은 action error 상태에 연결한다.

## 테스트와 검증

### Rust 상태와 스케줄링

- 자동 백업 OFF에서 일치 장치를 mount하면 source는 mounted지만 phase는 `idle`이고 백업을 예약하지 않는지 검증한다.
- 자동 백업 ON이고 setup/destination이 준비된 mount는 `detecting`과 백업 예약을 만드는지 검증한다.
- 자동 백업 OFF의 15초 scheduler defer가 snapshot을 작업 중으로 바꾸지 않는지 검증한다.
- 수동 `backup_now`가 자동 백업 설정과 무관하게 matched source를 안정 스캔하는 기존 계약을 회귀 검증한다.

### React와 IPC

- 자동 백업 OFF의 mounted idle snapshot이 `녹음기 연결됨`과 수동 안내를 표시하는지 검증한다.
- header 새로고침 버튼과 footer `지금 백업`이 각각 `backupNow`를 정확히 한 번 호출하는지 검증한다.
- `Command-R`이 기본 reload를 막고 `backupNow`를 정확히 한 번 호출하는지 검증한다.
- input 계열 target, setup 미완료, 실제 active phase와 pending action에서는 단축키가 새 요청을 만들지 않는지 검증한다.
- 버튼과 단축키가 동시에 입력돼도 single-flight guard가 중복 요청을 만들지 않는지 검증한다.
- 고정 Tauri command allowlist가 임의 filesystem command 없이 유지되는지 검증한다.

### 전체 및 설치 검증

- 프런트엔드 전체 테스트, TypeScript build, Rust workspace test, format과 package 검증을 실행한다.
- ad-hoc deep/strict codesign과 DMG checksum을 검증한다.
- `/Users/channprj/Applications/Backup Mic.app`에 새 번들을 설치하고 실행한다.
- 설치 실행 파일과 build artifact SHA-256이 일치하는지 확인한다.
- 실제 연결 디스크와 설정을 변경하지 않고 자동 백업 OFF 시작 상태가 idle로 수렴하는지 로그와 표시 상태로 확인한다.
- worktree가 깨끗하고 `HEAD`, tracking upstream과 live remote가 `0 0`으로 일치하는지 확인한다.

## 완료 기준

- WAV가 있는 연결 녹음기가 자동 백업 OFF에서 더 이상 영구 `확인 중`으로 표시되지 않는다.
- 연결된 source는 수동 백업 가능한 settled 상태로 보인다.
- header 새로고침 버튼과 `Command-R`이 같은 안전한 scan-and-backup 요청을 실행한다.
- 중복 요청과 실제 작업 중 재진입이 차단된다.
- 기존 자동 백업, fingerprint 재탐색, 파일 검증과 휴지통 안전 계약이 유지된다.
- 검증된 커밋이 원격에 푸시되고 새 앱이 빌드·재설치·실행된다.
