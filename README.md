# DJI Mic Backup

DJI Mic Mini 2S 케이스를 USB-C로 연결하면 두 송신기의 녹음을 로컬 폴더에 자동 백업하는 macOS 메뉴 막대 앱입니다. 원본 WAV를 먼저 복사하고 SHA-256으로 검증한 뒤, 기본 설정에서는 AAC-LC M4A를 만들어 다시 검사합니다. 검증된 원본을 정리할 때도 영구 삭제하지 않고 macOS 휴지통으로 이동합니다.

## 기본 동작

- TX01과 TX02를 볼륨 이름이나 마운트 경로가 아닌 페어링된 장치 UUID와 물리 속성으로 식별합니다.
- 자동 백업과 M4A 변환은 기본으로 켜져 있고, `백업 후 휴지통으로 이동`은 기본으로 꺼져 있습니다.
- 연결된 녹음이 2초 동안 변하지 않은 일반 WAV 파일인지 확인한 뒤 처리합니다.
- 장치가 계속 연결되어 있어도 15초마다 파일 메타데이터를 확인하므로 새 녹음을 감지합니다. 변경이 없으면 백업 작업이나 로그를 만들지 않습니다.
- M4A는 AAC-LC 192 kbps 프로필로 변환하며, `/usr/bin/afinfo -x`로 컨테이너, 코덱, 채널, 샘플레이트, 프레임과 재생 시간을 확인합니다.
- M4A 변환을 끄면 원본과 SHA-256이 같은 WAV를 최종 산출물로 보관합니다.
- 같은 녹음을 다시 연결하면 ledger와 파일을 재검증하며 중복 복사본을 만들지 않습니다.
- 네트워크 통신, 클라우드 업로드, 분석 도구, 전사 기능은 없습니다.

기본 백업 위치는 `/Users/channprj/Documents/DJI-Mic-Mini-2S`입니다. 날짜별 작업 기록은 다음 위치에 UTF-8 텍스트로 계속 추가됩니다.

```text
<백업 위치>/logs/YYYY/MM/YYMMDD-backup-mic.log
```

로그에는 절대 원본 경로, 볼륨 UUID, 전체 해시, 제안 ID나 오디오 데이터가 기록되지 않습니다. SQLite ledger가 권위 있는 기록이며, 일일 로그를 쓸 수 없으면 원본 휴지통 이동은 잠깁니다.

## 빌드와 로컬 설치

요구 사항은 macOS 13 이상과 Apple Silicon Mac입니다. 이 로컬 패키지는 Developer ID 서명이나 공증을 하지 않는 개인용 ad-hoc 빌드입니다.

```bash
pnpm install --frozen-lockfile
./scripts/check.sh
./scripts/package-local.sh
./scripts/install-local.sh "src-tauri/target/release/bundle/macos/DJI Mic Backup.app"
open -a "/Users/channprj/Applications/DJI Mic Backup.app"
```

패키징 스크립트는 앱과 DMG의 번들 식별자, 최소 macOS 버전, arm64 아키텍처, deep strict 코드 서명, DMG와 SHA-256을 검사합니다.

설치 스크립트는 새 앱을 `/Users/channprj/Applications` 안의 권한이 제한된 임시 디렉터리에 먼저 복사해 검증합니다. 실행 중인 앱에 종료를 요청한 뒤 기존의 정확한 앱 번들을 임시 롤백 위치로 옮기고 새 앱을 원자적으로 설치합니다. 설치 후 검증이나 실행 파일 해시 비교가 실패하면 이전 앱을 복원합니다. 성공하면 이전 앱은 영구 삭제하지 않고 macOS 휴지통으로 이동합니다.

## 처음 사용과 설정

1. 앱을 처음 열면 나타나는 macOS 폴더 선택 창에서 기본 백업 폴더를 확인하거나 다른 로컬 폴더를 선택합니다.
2. DJI Mic 케이스에 두 송신기를 넣고 USB-C로 Mac에 연결합니다.
3. 메뉴 막대의 DJI Mic Backup 아이콘을 누릅니다.
4. 표시된 두 저장 장치를 TX01과 TX02에 지정하고 `이 송신기로 연결`을 누릅니다.
5. 첫 백업이 끝날 때까지 케이스를 분리하지 않습니다.
6. `백업 검증 완료`와 녹음 수·용량·완료 시간이 표시되는지 확인합니다.

`설정…` 창에서 백업 위치, 자동 백업, M4A 변환, 백업 후 휴지통 이동, 로그인할 때 시작을 바꿀 수 있습니다. 설정은 Rust가 SQLite에 저장한 뒤에만 UI에 확정됩니다. `로그 열기`는 현재 날짜의 로그 폴더를 엽니다.

## 원본을 휴지통으로 이동

수동과 자동 정리 모두 같은 재검증 절차를 사용합니다. 자동 정리는 기본으로 꺼져 있으며, 처음 켤 때 복구 가능 위치와 독립적인 송신기 처리 방식을 설명하는 확인 창에서 동의해야 합니다.

수동으로 정리하려면 다음과 같이 진행합니다.

1. 완전히 검증된 송신기의 `휴지통으로 이동`을 누릅니다.
2. 세션 수, 파일 수와 용량을 확인합니다.
3. 확인 창에서 다시 `휴지통으로 이동`을 누릅니다.
4. 앱이 장치 정체성, 마운트/스캔 세대, 현재 원본 전체, source SHA-256, 최종 artifact SHA-256과 오디오 형상, ledger와 일일 로그를 다시 확인합니다.

`TX_MIC001_20260809_021747`처럼 인식된 세션 폴더에는 검증된 WAV만 있어야 합니다. 조건을 만족하면 개별 WAV가 아니라 폴더 전체를 한 항목으로 외장 볼륨의 macOS 휴지통에 이동하므로 폴더 구조까지 복구할 수 있습니다. 숨김 파일, 알 수 없는 파일, 하위 폴더, 심볼릭 링크가 하나라도 있으면 세션 전체를 그대로 두고 거부합니다. 세션 밖의 검증된 루트 WAV는 파일별로 휴지통에 이동합니다.

자동 정리를 켜면 TX01과 TX02가 서로 독립적으로 처리됩니다. 한 송신기의 전체 snapshot이 끝났더라도 다른 송신기의 실패 상태를 덮어쓰지 않습니다. 앱은 Finder, AppleScript, 셸, `rm` 또는 `.Trashes` 직접 조작 없이 Foundation의 macOS Trash API만 사용하며 휴지통을 비우지 않습니다.

기존 버전이 WAV를 영구 정리한 뒤 남긴 빈 세션 폴더는 이름이 정확하고, 현재도 완전히 비어 있고, 같은 폴더 아래의 녹음이 정리됐다는 ledger 증거가 있으며, 페어링된 장치가 일치할 때만 휴지통으로 이동합니다. 비슷하게 생긴 빈 폴더만으로는 정리 권한이 생기지 않습니다.

휴지통 제안은 5분 후 만료됩니다. 장치 분리, 목적지 변경, 새 스캔, 백업 시작 또는 파일 변화가 감지되면 즉시 무효화됩니다.

## 장애와 복구

- 백업 중 장치가 분리되거나 파일이 바뀌면 원본은 유지되고 다음 15초 검사나 다음 연결에서 다시 시도합니다.
- 앱이 소유한 `.part-<uuid>`, staging WAV와 recovery marker만 제한적으로 복구·정리합니다. 기존 대상 파일은 덮어쓰지 않습니다.
- 앱이 강제 종료되면 진행 중 작업은 중단으로 기록되고 다음 실행에서 안전하게 재검증합니다.
- ledger가 손상되면 격리한 뒤 새 기록을 만들며, 원본 휴지통 이동은 artifact를 다시 색인할 때까지 잠깁니다.
- `저장 공간이 부족합니다`가 표시되면 staging WAV와 M4A가 동시에 존재해도 최종적으로 10 GiB가 남도록 공간을 확보합니다.
- 상태가 갱신되지 않으면 `지금 백업`을 눌러 전체 스캔과 목적지 검증을 즉시 요청합니다.

## 독립 백업 확인

검증 스크립트는 live source WAV의 SHA-256을 ledger의 source 증거와 비교하고, 최종 artifact의 SHA-256을 별도의 artifact 증거와 비교합니다. WAV 모드에서만 source와 artifact 해시가 같아야 합니다. M4A 모드는 손실 압축이므로 두 해시를 비교하지 않고 `/usr/bin/afinfo -x`의 오디오 형상을 ledger와 대조합니다.

```bash
./scripts/verify-backup.sh \
  /Users/channprj/Documents/DJI-Mic-Mini-2S \
  "TX01=/Volumes/DJI-MIC-1" \
  "TX02=/Volumes/DJI-MIC-2"
```

일반 출력은 경로와 해시를 숨기고 파일 수만 보여 줍니다. 로컬 진단에 꼭 필요할 때만 `--diagnostic`을 추가합니다. 별도 ledger를 검사하려면 `--ledger /absolute/path/to/ledger.sqlite3`를 사용할 수 있습니다. 이 스크립트는 source, destination, ledger를 읽기만 합니다.

## 개발과 안전 수용 테스트

```bash
pnpm install --frozen-lockfile
./scripts/check.sh
./scripts/accept-deletion-fixture.sh
pnpm tauri dev
```

`check.sh`는 Rust 포맷/테스트/Clippy, frontend 테스트/typecheck/build와 함께 영구 원본 삭제, 셸 기반 오디오 실행, 임의 settings IPC, `.Trashes` 직접 조작을 검사합니다.

`accept-deletion-fixture.sh`는 새 64 MiB MS-DOS FAT32 이미지를 정확히 `/Volumes/DJI-DELTEST`로 마운트하고 sentinel을 기록한 뒤에만 실행됩니다. 테스트는 `TX_MIC…` 세션이 source 위치에서 사라지고 폐기용 이미지 안의 복구 가능 Trash fixture에 폴더 전체로 존재하며 destination이 유지되는지 확인합니다. 이미 같은 이름의 마운트가 있으면 실행을 거부하고, 연결된 `DJI-MIC-1/2`는 대상으로 허용하지 않습니다.

브라우저에서 네이티브 IPC 없이 UI 상태만 확인하려면 `pnpm dev` 실행 후 `preview.html?state=copying`, `complete`, `error`, `pairing`을 엽니다.

안전 핵심은 `src-tauri/crates/backup-core`의 Tauri 비의존 Rust 코드에 있습니다. React는 엄격한 Zod DTO와 열네 개의 고정된 명령만 사용하며 파일 시스템, 셸, 네트워크 또는 임의 경로 열기 권한을 갖지 않습니다.

## 범위 밖

오디오 재생, 전사, 태깅, 편집, 클라우드 동기화, 휴지통 자동 비우기, 자동 꺼내기, App Store 배포와 공증은 이 버전의 범위에 포함되지 않습니다.
