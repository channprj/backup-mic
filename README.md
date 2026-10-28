# Backup Mic

녹음기를 연결하면 녹음 파일을 로컬 폴더에 자동 백업하는 macOS 메뉴 막대 앱입니다.
**macOS 13 이상 · Apple Silicon**을 지원합니다.

## 주요 기능

- DJI Mic Mini 2S 기본 규칙과 사용자 정의 녹음기 규칙
- SHA-256 검증, 중복 백업 방지, 중단된 작업 재시도
- WAV 백업 후 AAC-LC 128 kbps M4A 변환
- 검증된 원본을 macOS 휴지통으로 이동 — 자동 이동은 기본으로 꺼져 있습니다.
- Whisper(whisper.cpp) 로컬 전사 — 기본으로 꺼져 있으며 녹음 시각이 붙은 TXT와 SRT·VTT·JSON으로 저장하고, 사전과 Ollama 번역을 지원합니다.
- 클라우드 업로드 없이 로컬에서 처리

## 시작하기

개발 환경: Node.js 26, pnpm 10.33.4, Rust 1.97 이상, Xcode Command Line Tools, CMake(whisper.cpp 빌드용).

```bash
pnpm install --frozen-lockfile
pnpm tauri dev
```

앱에서 백업 폴더를 선택하고 녹음기를 연결한 뒤, 메뉴 막대에서 `백업 검증 완료`를 확인합니다.
기본 백업 위치는 `~/Documents/Backup Mic`이며, 다른 녹음기는 설정의 `녹음기 규칙`에서 추가할 수 있습니다.

자동 백업과 M4A 변환은 기본으로 켜져 있습니다. 변환을 끄면 WAV로 보관하며 원본 휴지통 이동은 비활성화됩니다.
오류가 발생하면 앱의 `로그 열기`로 확인할 수 있습니다.

## 전사

설정 창의 `전사` 탭에서 `백업 후 자동 전사`를 켜면, 백업이 끝날 때마다 새로 검증된 녹음을 이 Mac에서 whisper.cpp로 전사합니다.
녹음은 외부로 보내지 않으며 Apple Silicon GPU(Metal)를 사용합니다.

- **모델**: Tiny부터 Large v3까지 15개 ggml 모델 중에서 고르며 기본값은 Medium입니다. 모델은 고정된 Hugging Face 리비전에서 내려받고 크기와 SHA-256이 맞을 때만 설치합니다. 중단된 다운로드는 이어서 받습니다.
- **모델 경로**: 모델 폴더(기본 `~/Library/Application Support/com.channprj.BackupMic/whisper-models`)를 바꾸거나, 이미 가진 ggml `.bin` 파일을 직접 선택할 수 있습니다.
- **저장 위치**: 기본 `~/Documents/Backup Mic Transcripts`이며 백업 폴더와 같은 하위 폴더 구조로 저장합니다. 이미 있는 전사 파일은 덮어쓰지 않으므로, 다시 전사하려면 해당 파일을 지우고 `지금 전사`를 누릅니다.
- **타임스탬프**: TXT는 WakeNote 내보내기처럼 줄마다 `2025-10-28 14:30:25 [TX01] - 텍스트` 형식으로 녹음 시각과 출처(DJI 송신기 또는 녹음기 규칙 이름)를 붙입니다. 시각은 원본 파일명의 시작 시각(`TX01_MIC001_20251028_143022.wav` 등)에 구간 위치를 더해 계산하고, 파일명에 시각이 없으면 원본 수정 시각에서 녹음 길이를 빼서 구합니다. SRT·VTT는 재생기를 위해 파일 기준 상대 시간을 유지하고, JSON에는 두 시간을 모두 넣습니다.
- **사전**: WakeNote 사전과 같은 `정식 표기 = 별칭, 별칭` 형식입니다. 정식 표기는 Whisper에 힌트로 주고, 별칭은 저장하기 전에 정식 표기로 바꿉니다. 영문 별칭은 대소문자를 무시하고 단어 경계에서만, 그 밖의 별칭은 정확히 일치할 때만 바꿉니다. 단어를 칩으로 하나씩 편집하거나 `텍스트로 편집`으로 한꺼번에 고칠 수 있습니다.
- **번역**: Ollama 주소(기본 `http://localhost:11434`)와 모델 이름을 넣으면 전사가 끝난 뒤 고른 언어로 번역해 원본 옆에 `이름.언어.확장자`(예: `….en.txt`)로 저장합니다. 모델을 비워 두면 번역하지 않고, 번역이 실패해도 원본 전사는 남습니다. 다른 컴퓨터의 주소를 쓰면 전사 텍스트가 그 컴퓨터로 전송됩니다.
- **Whisper 옵션**: 언어(자동 감지 포함), 빔 서치·그리디, 온도와 고급 옵션(스레드, GPU, 임계값, 세그먼트 길이 등)을 설정할 수 있습니다.

전사를 켜기 전에 백업한 녹음은 `이전에 백업한 녹음도 전사`를 켰을 때만 오래된 순서로 전사합니다.
백업이 진행 중이면 끝날 때까지 기다렸다가 이어서 전사합니다.

## 앱 빌드와 설치

```bash
./scripts/package-local.sh
./scripts/install-local.sh "src-tauri/target/release/bundle/macos/Backup Mic.app"
open -a "$HOME/Applications/Backup Mic.app"
```

현재 `VERSION`으로 빌드하며, 앱은 `~/Applications`에 설치됩니다. 로컬 빌드는 ad-hoc 서명을 사용하며 Apple 공증은 포함하지 않습니다.

## 개발 검사

Python 3.9 이상, ripgrep, Gitleaks 8.30 이상과 `cargo-audit`가 필요합니다.
보안 검사에는 전체 Git 이력과 네트워크 접근이 필요합니다.

```bash
brew install ripgrep gitleaks cargo-audit cmake
./scripts/check.sh
```

Rust·프런트엔드 테스트, 타입 검사, 빌드, 비밀정보·의존성 감사를 실행합니다.
보안 검사만 실행하려면 `pnpm security:check`를 사용합니다.

## 작업 관리

작업 진행 상태는 [backup-mic Project](https://github.com/users/channprj/projects/7/views/1)에서 Issue로 관리합니다.
커밋 메시지에는 작업이 속한 Issue를 `Refs #번호`로 남기며, `pnpm install`이 설치하는 `commit-msg` 훅이 이를 확인합니다.
훅만 다시 설치하려면 `pnpm run setup:hooks`, 푸시 전 범위를 확인하려면 `pnpm run tasks:check`를 사용합니다.
자세한 규칙은 [AGENTS.md](AGENTS.md)와 [GitHub Project 운영 가이드](docs/github-project-workflow.md)를 참고합니다.

## 라이선스

[MIT](LICENSE) · Copyright (c) 2026 channprj
