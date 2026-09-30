# Backup Mic

녹음기를 연결하면 녹음 파일을 로컬 폴더에 자동 백업하는 macOS 메뉴 막대 앱입니다.
**macOS 13 이상 · Apple Silicon**을 지원합니다.

## 주요 기능

- DJI Mic Mini 2S 기본 규칙과 사용자 정의 녹음기 규칙
- SHA-256 검증, 중복 백업 방지, 중단된 작업 재시도
- WAV 백업 후 AAC-LC 128 kbps M4A 변환
- 검증된 원본을 macOS 휴지통으로 이동 — 자동 이동은 기본으로 꺼져 있습니다.
- 클라우드 업로드 없이 로컬에서 처리

## 시작하기

개발 환경: Node.js 26, pnpm 10.33.4, Rust 1.97 이상, Xcode Command Line Tools.

```bash
pnpm install --frozen-lockfile
pnpm tauri dev
```

앱에서 백업 폴더를 선택하고 녹음기를 연결한 뒤, 메뉴 막대에서 `백업 검증 완료`를 확인합니다.
기본 백업 위치는 `~/Documents/Backup Mic`이며, 다른 녹음기는 설정의 `녹음기 규칙`에서 추가할 수 있습니다.

자동 백업과 M4A 변환은 기본으로 켜져 있습니다. 변환을 끄면 WAV로 보관하며 원본 휴지통 이동은 비활성화됩니다.
오류가 발생하면 앱의 `로그 열기`로 확인할 수 있습니다.

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
brew install ripgrep gitleaks cargo-audit
./scripts/check.sh
```

Rust·프런트엔드 테스트, 타입 검사, 빌드, 비밀정보·의존성 감사를 실행합니다.
보안 검사만 실행하려면 `pnpm security:check`를 사용합니다.

## 라이선스

[MIT](LICENSE) · Copyright (c) 2026 channprj
