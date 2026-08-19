# Backup Mic

이동식 녹음기를 연결하면 사용자가 입력한 glob 규칙에 맞는 녹음을 로컬 폴더에 자동 백업하는 macOS 메뉴 막대 앱입니다. DJI Mic Mini 2S 규칙은 처음부터 등록되어 있습니다. 원본 WAV를 먼저 복사하고 SHA-256으로 검증한 뒤, 기본 설정에서는 AAC-LC M4A를 만들어 다시 검사합니다. 검증된 원본을 정리할 때도 영구 삭제하지 않고 macOS 휴지통으로 이동합니다.

## 기본 동작

- 볼륨 이름, 필수 경로와 백업 파일 glob으로 녹음기를 찾고, DJI 프리셋은 장치 UUID와 물리 속성 검증을 추가로 적용합니다.
- 자동 백업과 M4A 변환은 기본으로 켜져 있고, `백업 후 휴지통으로 이동`은 기본으로 꺼져 있습니다.
- 규칙에 선택된 녹음과 같은 세션의 외부 M4A, AppleDouble(`._…`) 및 기타 일반 파일이 2초 동안 변하지 않았는지 확인한 뒤 처리합니다.
- 장치가 계속 연결되어 있어도 기본 15초마다 파일 메타데이터를 확인하므로 새 녹음을 감지합니다. 변경이 없으면 백업 작업이나 로그를 만들지 않습니다. 확인 주기는 설정에서 5초에서 1시간 사이로 바꿀 수 있고, 실행 중에 바꿔도 즉시 적용됩니다.
- 녹음기별 새 WAV와 추가 파일을 먼저 백업 폴더로 복사하고 SHA-256으로 검증합니다. 한 녹음기에서 하나라도 실패하면 그 녹음기의 M4A 변환은 시작하지 않지만 다른 녹음기는 독립적으로 완료할 수 있습니다.
- M4A는 백업 폴더의 WAV만 AAC-LC 128 kbps 프로필로 변환하며, `/usr/bin/afinfo -x`로 컨테이너, 코덱, 채널, 샘플레이트, 프레임과 재생 시간을 확인합니다.
- 새 WAV뿐 아니라 이전 버전이 백업 폴더에 남긴 모든 ledger-검증 WAV도 같은 변환 코호트에 포함합니다.
- M4A 변환을 끄면 원본과 SHA-256이 같은 WAV를 최종 산출물로 보관하고 외장 디스크 원본에는 수동·자동 휴지통 이동 권한을 부여하지 않습니다.
- 같은 녹음을 다시 연결하면 ledger와 파일을 재검증하며 중복 복사본을 만들지 않습니다.
- 네트워크 통신, 클라우드 업로드, 분석 도구, 전사 기능은 없습니다.

새 설치의 기본 백업 위치는 `/Users/channprj/Documents/Backup Mic`입니다. 기존 설치는 SQLite에 저장된 백업 위치를 그대로 사용합니다. 날짜별 작업 기록은 다음 위치에 UTF-8 텍스트로 계속 추가됩니다.

```text
<백업 위치>/logs/YYYY/MM/YYMMDD-backup-mic.log
```

로그에는 절대 원본 경로, 볼륨 UUID, 전체 해시, 제안 ID나 오디오 데이터가 기록되지 않습니다. SQLite ledger가 권위 있는 기록이며, 일일 로그를 쓸 수 없으면 원본 휴지통 이동은 잠깁니다.

백업 위치의 로그를 만들 수 없는 시작·설정·목적지 오류는 다음 사용자 로그 폴더에 같은 개인정보 보호 형식으로 기록됩니다.

```text
~/Library/Logs/com.channprj.BackupMic/YYYY/MM/YYMMDD-backup-mic.log
```

기존 `com.channprj.DJIMicBackup` 앱 데이터가 있고 새 ledger가 아직 없으면, 앱 시작 전에 SQLite 온라인 백업으로 커밋된 WAL 상태까지 새 앱 데이터 폴더로 복사하고 무결성 검사와 스키마 마이그레이션을 마친 뒤 원자적으로 설치합니다. 기존 ledger와 로그는 변경하거나 삭제하지 않습니다.

## 녹음기 규칙

설정의 `녹음기 규칙`에서 규칙을 직접 추가·편집·복제·보관할 수 있습니다. 정규식이나 셸 표현식은 실행하지 않으며, ASCII 대소문자를 구분하지 않는 `*`, `?`, `**` glob만 사용합니다.

- `규칙 이름`과 `보관 폴더 이름`: UI 표시와 선택한 백업 위치 아래의 규칙별 루트에 사용합니다. 보관 폴더 이름을 바꾸면 검증된 기존 산출물도 다음 백업 전에 새 루트로 안전하게 정리됩니다.
- `볼륨 이름 glob`: 연결된 볼륨 표시 이름을 고릅니다.
- `필수 경로 glob`: 볼륨이 해당 녹음기인지 확인할 때 모두 존재해야 하는 경로입니다.
- `백업 파일 glob`: 실제로 복사할 WAV와 companion 파일을 선택하며 하나 이상 필요합니다.
- `세션 폴더 glob`: 하나의 복구 가능한 단위로 휴지통에 보낼 폴더를 지정합니다.
- `파일명 프리픽스`·`파일명 서픽스`: 녹음기별 최종 파일명에 추가합니다.
- `날짜 폴더 구조`: 기본값은 `YYYY/MM/DD/`이며, 필요하면 `YYYY/MM/` 또는 `YYMMDD/`로 바꿀 수 있습니다.

예를 들어 백업 위치를 `/Volumes/990EVO+/labs/audio-records`로, 보관 폴더 이름을 `dji`로 설정하면 기본 경로는 `/Volumes/990EVO+/labs/audio-records/dji/YYYY/MM/DD/`입니다. Zoom H1n에 보관 폴더 `Zoom H1n`, 볼륨 `ZOOM_*`, 필수 경로 `RECORD/**`, 백업 파일 `RECORD/**/*.WAV`, 프리픽스 `zoom-`, 서픽스 `-field`를 설정하면 결과 파일은 `Zoom H1n/2026/08/10/260810-zoom-ZOOM0001-field.m4a`처럼 저장됩니다.

`DJI Mic Mini 2S`는 같은 규칙 엔진을 사용하는 편집 가능한 기본 규칙입니다. DJI의 기존 `TX01_`·`TX02_` 파일명만 `T01_`·`T02_`로 줄이며, 사용자가 지정한 프리픽스와 서픽스도 동일하게 적용합니다. `DJI 기본값 복원`은 규칙 ID와 기존 증거를 유지한 채 기본 필드만 복원합니다.

`DJI Mic Mini 2S`도 예외 없이 `<백업 위치>/<보관 폴더 이름>/YYYY/MM/DD/` 규칙을 사용하며, TX01과 TX02 파일은 송신기별 하위 폴더 없이 같은 날짜 폴더에 함께 저장됩니다. 이전 버전이 `<백업 위치>/YYYY/MM`, `<백업 위치>/<이전 보관 폴더>/YYYY/MM` 또는 더 오래된 날짜·송신기 폴더에 남긴 검증된 DJI 산출물은 다음 백업 전에 현재 보관 폴더와 날짜 구조 아래로 옮겨집니다. 새 위치의 크기와 SHA-256을 확인하고 ledger 경로를 갱신한 뒤에만 기존 파일을 Foundation의 macOS 휴지통으로 이동합니다. 같은 내용의 대상은 재사용하고, 다른 내용과 충돌하면 짧은 해시 서픽스를 붙입니다. 중단되거나 내용이 달라진 파일은 그대로 두고 다음 시작 또는 백업 때 안전하게 다시 시도합니다.

## 빌드와 로컬 설치

요구 사항은 macOS 13 이상과 Apple Silicon Mac입니다. 이 로컬 패키지는 Developer ID 서명이나 공증을 하지 않는 개인용 ad-hoc 빌드입니다.

```bash
pnpm install --frozen-lockfile
./scripts/check.sh
headatever init 0 --dry-run
# package.json, Cargo.toml, tauri.conf.json 등의 버전을 미리 맞춘 뒤
headatever init 0 --push
./scripts/package-local.sh "$(tr -d '\r\n' < VERSION)"
./scripts/install-local.sh "src-tauri/target/release/bundle/macos/Backup Mic.app" "$(tr -d '\r\n' < VERSION)"
open -a "/Users/channprj/Applications/Backup Mic.app"
```

`VERSION`은 직접 만들거나 수정하지 않고 Headatever가 생성합니다. Headatever 릴리스 커밋과 annotated tag를 먼저 일반 push한 뒤에만 패키징합니다. 패키징 스크립트는 앱과 DMG의 번들 식별자, `CFBundleShortVersionString`, 최소 macOS 버전, arm64 아키텍처, deep strict ad-hoc 코드 서명, DMG와 SHA-256을 검사합니다. 이전 빌드 산출물은 정확한 `target/release/bundle/macos`·`dmg` 디렉터리만 비운 뒤 새 앱과 DMG가 각각 하나인지 확인합니다.

설치 스크립트는 새 앱을 `/Users/channprj/Applications` 안의 권한이 제한된 임시 디렉터리에 먼저 복사해 검증합니다. 실행 중인 앱에 종료를 요청한 뒤 기존의 정확한 앱 번들을 임시 롤백 위치로 옮기고 새 앱을 원자적으로 설치합니다. 설치 후 검증이나 실행 파일 해시 비교가 실패하면 이전 앱을 복원합니다. 성공하면 이전 앱은 영구 삭제하지 않고 macOS 휴지통으로 이동합니다.

## 처음 사용과 설정

1. 앱을 처음 열면 나타나는 macOS 폴더 선택 창에서 기본 백업 폴더를 확인하거나 다른 로컬 폴더를 선택합니다.
2. DJI Mic Mini 2S 또는 설정에 규칙을 추가한 녹음기를 Mac에 연결합니다.
3. 메뉴 막대의 Backup Mic 아이콘을 누릅니다.
4. 첫 백업이 끝날 때까지 녹음기를 분리하지 않습니다.
5. `백업 검증 완료`와 녹음 수·용량·완료 시간이 표시되는지 확인합니다.

`설정…` 창에서 녹음기 규칙, 보관 폴더 이름, 날짜 폴더 구조, 파일명 프리픽스·서픽스, 백업 위치, 자동 백업, `백업 폴더에 남길 여유 공간`, `WAV 백업 후 M4A로 변환`, `연결된 녹음기 확인 주기`, 백업 후 휴지통 이동, 로그인할 때 시작을 바꿀 수 있습니다. 설정은 Rust가 SQLite에 저장하고 다시 읽은 뒤에만 UI에 확정됩니다. 백업 중 저장한 설정은 현재 실행을 바꾸지 않고 `다음 백업부터 적용됩니다`. 실패 카드의 안전한 오류 코드와 `로그 열기`로 원인 기록을 바로 확인할 수 있습니다.

## 원본을 휴지통으로 이동

수동과 자동 정리 모두 같은 재검증 절차를 사용합니다. 자동 정리는 기본으로 꺼져 있으며, 처음 켤 때 복구 가능 위치와 독립적인 송신기 처리 방식을 설명하는 확인 창에서 동의해야 합니다.

수동으로 정리하려면 다음과 같이 진행합니다.

1. 완전히 검증된 녹음기의 `휴지통으로 이동`을 누릅니다.
2. 세션 수, 파일 수와 용량을 확인합니다.
3. 확인 창에서 다시 `휴지통으로 이동`을 누릅니다.
4. 앱이 장치 정체성, 마운트/스캔 세대, 현재 원본 전체, source SHA-256, 최종 artifact SHA-256과 오디오 형상, 128kbps run barrier, ledger와 일일 로그를 다시 확인합니다.

규칙의 세션 glob으로 인식된 폴더에 있는 WAV, 외부 M4A, AppleDouble 및 기타 일반 파일은 각각 백업·검증 증거를 가져야 합니다. 조건을 만족하면 개별 파일이 아니라 폴더 전체를 한 항목으로 외장 볼륨의 macOS 휴지통에 이동하므로 빈 원본 폴더를 남기지 않고 폴더 구조까지 복구할 수 있습니다. 백업 후 새로 생기거나 바뀐 파일, 하위 폴더, 심볼릭 링크가 하나라도 있으면 세션 전체를 그대로 두고 거부합니다. 세션 밖의 검증된 파일은 파일별로 휴지통에 이동합니다.

자동 정리를 켜면 각 녹음기가 서로 독립적으로 처리됩니다. 한 녹음기의 전체 snapshot이 끝났더라도 다른 녹음기의 실패 상태를 덮어쓰지 않습니다. 앱은 Finder, AppleScript, 셸, `rm` 또는 `.Trashes` 직접 조작 없이 Foundation의 macOS Trash API만 사용하며 휴지통을 비우지 않습니다.

기존 버전이 WAV를 영구 정리한 뒤 남긴 빈 세션 폴더는 이름이 정확하고, 현재도 완전히 비어 있고, 같은 폴더 아래의 녹음이 정리됐다는 ledger 증거가 있으며, 페어링된 장치가 일치할 때만 휴지통으로 이동합니다. 비슷하게 생긴 빈 폴더만으로는 정리 권한이 생기지 않습니다.

휴지통 제안은 5분 후 만료됩니다. 장치 분리, 목적지 변경, 새 스캔, 백업 시작 또는 파일 변화가 감지되면 즉시 무효화됩니다.

## 장애와 복구

- 백업 중 장치가 분리되거나 파일이 바뀌면 원본은 유지되고 다음 15초 검사나 다음 연결에서 다시 시도합니다.
- 복사, ledger, 로그, Apple 변환, M4A 검증, 원본 재검증 또는 휴지통 이동이 실패하면 안전한 단계와 오류 코드가 기본 또는 fallback 로그에 남습니다.
- 한 녹음기의 전체 복사 배리어나 M4A 코호트가 완료되지 않으면 그 녹음기의 원본에는 휴지통 이동 권한이 생기지 않습니다.
- 앱이 소유한 `.part-<uuid>`, staging WAV와 recovery marker만 제한적으로 복구·정리합니다. 기존 대상 파일은 덮어쓰지 않습니다.
- 앱이 강제 종료되면 진행 중 작업은 중단으로 기록되고 다음 실행에서 안전하게 재검증합니다.
- ledger가 손상되면 격리한 뒤 새 기록을 만들며, 원본 휴지통 이동은 artifact를 다시 색인할 때까지 잠깁니다.
- `저장 공간이 부족합니다`가 표시되면 staging WAV와 M4A가 동시에 존재해도 설정한 여유 공간이 남도록 공간을 확보합니다. 기본값은 10 GiB이고 1 GiB에서 512 GiB 사이로 바꿀 수 있습니다. 범위를 벗어난 값은 조정하지 않고 거부합니다.
- 상태가 갱신되지 않으면 `지금 백업`을 눌러 전체 스캔과 목적지 검증을 즉시 요청합니다.

## 독립 백업 확인

검증 스크립트는 연결된 모든 in-scope WAV와 추가 파일의 SHA-256을 ledger source 증거와 비교합니다. 현재 원본이 휴지통으로 이동된 과거 녹음도 포함해 ledger의 모든 최종 artifact를 다시 해시하며, M4A는 `aac_lc_128k_v1` 코호트 배리어와 `/usr/bin/afinfo -x` 오디오 형상을 함께 대조합니다. 외부 M4A와 AppleDouble은 손실 변환 없이 raw copy의 크기와 SHA-256이 원본 증거와 같아야 합니다. WAV 모드에서만 source와 최종 artifact 해시가 같습니다. 스키마 v4부터는 변환 전 목적지 WAV의 경로·크기·SHA-256과 휴지통 이동 상태도 삭제하지 않고 보존하므로, 목적지 WAV가 사라진 뒤에도 선복사 사실을 독립적으로 확인할 수 있습니다.

```bash
./scripts/verify-backup.sh \
  "/Users/channprj/Documents/Backup Mic" \
  "DJI Mic Mini 2S=/Volumes/DJI-MIC-1" \
  "DJI Mic Mini 2S=/Volumes/DJI-MIC-2" \
  "Zoom H1n=/Volumes/ZOOM_H1N"
```

각 source 인자는 정확한 활성 규칙 이름과 현재 마운트 루트를 `규칙 이름=경로` 형태로 받습니다. 일반 출력은 경로와 해시를 숨기고 파일 수만 보여 줍니다. 규칙 아래의 누락된 source 증거, 보관 폴더 밖의 artifact, 잠기지 않은 보관 폴더, 변조된 M4A, 불완전한 배리어도 경로나 해시 없이 실패합니다. 로컬 진단에 꼭 필요할 때만 `--diagnostic`을 추가합니다. 별도 ledger를 검사하려면 `--ledger /absolute/path/to/ledger.sqlite3`를 사용할 수 있습니다. 이 스크립트는 source, destination, ledger 원본을 변경하지 않으며, 닫힌 WAL ledger가 read-only로 열리지 않을 때는 임시 복사본만 검사합니다.

## 개발과 안전 수용 테스트

```bash
pnpm install --frozen-lockfile
./scripts/check.sh
./scripts/accept-rule-fixtures.sh
./scripts/accept-deletion-fixture.sh
pnpm tauri dev
```

`check.sh`는 Rust 포맷/테스트/Clippy, frontend 테스트/typecheck/build와 함께 128kbps 명령, 전체 복사 배리어, 변환-off 원본 유지, fallback 로그, 동시 설정 저장, 독립 검증기, 영구 원본 삭제, 셸 기반 오디오 실행, 임의 settings IPC, `.Trashes` 직접 조작을 검사합니다.

`accept-rule-fixtures.sh`는 새 64 MiB MS-DOS FAT32 이미지 두 개를 정확히 `/Volumes/ZOOM_RULETEST`와 `/Volumes/SONY_RULETEST`로 마운트합니다. 각 이미지의 이름, 용량, 외부·쓰기 가능 속성, sentinel과 PCM WAV를 확인한 뒤 생산 `DeviceRegistry`, glob 규칙 매처, 안정 스캔, 복사·SHA-256 검증, Apple M4A 변환과 독립 배치 배리어를 실행합니다. Zoom과 Sony의 서로 다른 경로, 보관 폴더, 프리픽스·서픽스가 섞이지 않는지도 확인합니다. 같은 이름의 기존 마운트나 연결된 DJI 볼륨은 사용하지 않으며, 종료 시 기록해 둔 디스크 이미지 장치만 분리합니다. 이 검사는 실제 USB 녹음기나 실제 Disk Arbitration 연결 세션의 증거로 간주하지 않습니다.

`accept-deletion-fixture.sh`는 새 64 MiB MS-DOS FAT32 이미지를 정확히 `/Volumes/DJI-DELTEST`로 마운트하고 sentinel을 기록한 뒤에만 실행됩니다. 테스트는 두 PCM WAV, 외부 M4A, 명시적·FAT32 생성 AppleDouble을 모두 백업한 상태에서 한 `TX_MIC…` 세션이 source 위치에서 사라지고 복구 가능 Trash에 폴더 전체로 존재하는지 확인합니다. 생산 Apple 도구로 FAT32 위에서 128kbps M4A를 만들고 검사하며, raw 추가 파일과 destination이 유지되고 ledger 증거가 있는 빈 세션 폴더도 휴지통으로 이동하는지 증명합니다. 이미 같은 이름의 마운트가 있으면 실행을 거부하고, 연결된 `DJI-MIC-1/2`는 대상으로 허용하지 않습니다.

브라우저에서 네이티브 IPC 없이 UI 상태만 확인하려면 `pnpm dev` 실행 후 `preview.html?state=copying`, `complete`, `error`, `rules`를 엽니다.

안전 핵심은 `src-tauri/crates/backup-core`의 Tauri 비의존 Rust 코드에 있습니다. React는 엄격한 Zod DTO와 21개의 고정된 명령만 사용하며 파일 시스템, 셸, 네트워크 또는 임의 경로 열기 권한을 갖지 않습니다. 설정 명령도 임의의 키를 받지 않고 이름이 정해진 값만 받습니다.

## 범위 밖

오디오 재생, 전사, 태깅, 편집, 클라우드 동기화, 휴지통 자동 비우기, 자동 꺼내기, App Store 배포와 공증은 이 버전의 범위에 포함되지 않습니다.
