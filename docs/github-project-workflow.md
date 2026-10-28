# GitHub Project 운영 가이드
업데이트: 2026-10-28

기본 태스크 보드는 [backup-mic Project](https://github.com/users/channprj/projects/7/views/1)입니다. 보드의 Status를 실제 진행 상태의 원본으로 쓰고, 저장소 `channprj/backup-mic`의 Issue를 보드 항목으로 관리합니다.

사용자가 코딩 에이전트나 도구로 지시한 작업은 실행 전에 이 보드에 Issue로 등록합니다. 새 Issue의 기본 assignee는 `channprj`이며, 사용자가 다른 담당자를 지정한 경우에만 바꿉니다.

## 보드 구성

| 필드 | 값 |
| --- | --- |
| `Status` | `Backlog`, `To do`, `In progress`, `In review`, `Done`, `Closed` |
| `Priority` | `P0`(지금 바로), `P1`(가까운 시일 안), `P2`(여유가 있을 때) |
| `Size` | `XS`, `S`, `M`, `L`, `XL` |

보드에는 GitHub 기본 자동화가 켜져 있습니다. 저장소 Issue는 보드에 자동으로 추가되며, 추가된 항목은 `To do`, 닫힌 항목은 `Done`이 됩니다. Status를 `Done`으로 바꾸면 Issue도 닫힙니다. 자동화 결과를 그대로 믿지 말고, 등록과 Status 변경 뒤에는 항상 다시 조회합니다.

필드나 옵션을 임의로 추가하지 않습니다. 작업 전에 실제 구성을 다시 확인합니다.

```sh
gh project field-list 7 --owner channprj --format json
```

## 태스크 제목 작성

부모·leaf Issue의 제목은 **접두사 + 핵심 대상 + 명사형 작업**으로 씁니다. 보드는 연결된 Issue 제목을 그대로 보여 주며, 에이전트·CLI·웹 Issue form 모두 같은 기준을 따릅니다.

- 형식은 `type: 핵심 대상 및 작업`, 범위를 구분해야 하면 `type(scope): 핵심 대상 및 작업`입니다. `docs`, `feat`, `fix`, `chore` 등 작업에 맞는 타입을 고르고 `[Task]`는 붙이지 않습니다.
- `유지한다`, `기록한다`, `추가하기` 대신 `유지`, `기록`, `추가`, `수정`, `개선`, `제거`, `정리`처럼 작업 명사로 끝냅니다.
- 조사와 서술어는 줄이되 대상과 결과는 분명히 남깁니다. 같은 결과에 속하는 작업은 `및`이나 `·`로 잇고, 서로 독립적인 결과는 제목에 나열하기 전에 leaf로 나눕니다.
- 배경·구현 방법·완료 조건·상태는 본문과 보드 필드에 적습니다. 제목 끝에는 마침표를 붙이지 않습니다.

| 피할 표현 | 쓸 제목 |
| --- | --- |
| `fix(backup): 중단된 작업이 재시도되지 않는 문제를 수정한다` | `fix(backup): 중단된 작업 재시도 오류 수정` |
| `feat(transcription): 전사 결과를 SRT로도 저장하기` | `feat(transcription): 전사 결과 SRT 저장 추가` |
| `docs: README에 설치 방법을 추가함` | `docs: README 설치 방법 추가` |

## leaf Issue 기준

실행을 배정하는 Issue는 다음 조건을 모두 만족해야 합니다.

- 독립적으로 확인할 수 있는 결과가 하나다.
- 수정할 파일 또는 디렉터리 범위가 명확하다.
- 완료 조건이 체크리스트로 적혀 있다.
- 실행할 검증 방법과 기대 결과가 적혀 있다.
- 부모 Issue와 선행 태스크를 링크하거나 `없음`이라고 적었다.

여러 결과를 포함하는 목표는 부모 Issue로 만들고 leaf Issue로 나눕니다. 서브 에이전트에는 leaf Issue만 맡깁니다. GitHub가 지원하는 부모·자식(sub-issue)과 차단(blocked by) 관계를 설정하고, 본문에도 부모와 선행 Issue 링크를 남깁니다. 계획이 바뀌면 영향을 받는 leaf의 범위, 관계, 완료 조건을 함께 고칩니다.

웹에서 Issue를 만들 때는 [leaf 태스크 양식](../.github/ISSUE_TEMPLATE/leaf-task.yml)을 사용합니다.

## 에이전트 세션 기록

Claude Code, Codex 같은 AI 에이전트로 처리하는 Issue는 착수 전에 본문 `에이전트 세션` 절에 아래 표를 채웁니다. 어떤 에이전트가 어떤 모델과 추론 강도로 어느 세션에서 작업했는지 Issue 하나에서 확인하기 위한 기록입니다. 사람이 먼저 만든 Issue는 `미배정`으로 두었다가 에이전트를 배정할 때 채웁니다.

| 담당 역할 | 에이전트 종류 | 모델 | 추론 강도 | 세션 ID | 재개 방법 | 작업 경로·브랜치 | 시작 일시 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 주 에이전트 | Claude Code `<버전>` | `<모델 ID>` | `xhigh` | `<실제 세션 ID>` | `claude --resume <실제 세션 ID>` | 저장소 루트 / `main` | `YYYY-MM-DD HH:mm` |

선택한 설정: `<모델 등급 또는 모델>` / `<추론 강도>` — `<근거 한 줄>`

각 값은 현재 도구가 알려 주는 정보로 확인합니다.

| 값 | Claude Code | Codex |
| --- | --- | --- |
| 세션 ID | 셸 환경의 `CLAUDE_CODE_SESSION_ID` | 셸 환경의 `CODEX_THREAD_ID` |
| 모델 | 세션이 알려 주는 모델 ID 또는 `--model` 값 | 세션이 알려 주는 모델 ID 또는 `-m` 값 |
| 추론 강도 | `CLAUDE_EFFORT` 또는 `--effort` 값 | `-c model_reasoning_effort=` 값 또는 설정 파일 |
| 버전 | `claude --version` | `codex --version` |

- 다른 대화의 로그를 골라 현재 세션으로 추측하거나, 예전 세션의 모델을 기억으로 적지 않습니다.
- 값을 확인할 수 없으면 그 칸에 `확인 불가`와 이유를 적고, 확인되는 즉시 보완합니다.
- 서브 에이전트의 모델과 추론 강도는 위임할 때 지정한 값을 적습니다. 지정하지 않았다면 서브 에이전트가 보고한 값을 적습니다.
- 시작 일시는 한국 표준시 `YYYY-MM-DD HH:mm`으로 적습니다. 작업 경로는 저장소 루트나 worktree 이름처럼 개인 홈 경로를 드러내지 않는 형태로 적습니다.
- 새 세션에서 이어 가거나 도구·모델·추론 강도가 바뀌면 이전 행을 유지한 채 새 행을 추가하고, 착수·재개 댓글에 같은 정보와 맡은 범위를 남깁니다.
- 대화·로그 원문, 인증 토큰이나 자격증명은 기록하지 않습니다. 저장한 뒤 `gh issue view --json body,comments,projectItems`로 다시 조회합니다.

### 모델과 추론 강도 선택

작업 난이도와 위험에 맞춰 고르고, 고른 값과 근거를 `선택한 설정` 줄에 적습니다. 사용자가 지정한 값이 있으면 그 값을 따릅니다.

| 작업 성격 | 예 | 기준 |
| --- | --- | --- |
| 기계적인 변경과 조회 | 문구 변경, 오탈자, 좁은 범위의 읽기 전용 조회, 테스트 실행 | 작은 모델, 낮은 강도 |
| 범위가 분명한 실행 | 한 모듈의 기능·버그 수정, 절차가 정해진 문서 | 중간 모델, 중간 강도 |
| 설계와 핵심 위험 판단 | 구조 설계, 복잡한 원인 규명, 데이터 손실·동시성·권한 판단, 최종 검증 | 가장 강한 모델, 높은 강도 |

## 등록 순서

`KEYWORD`, `ISSUE_URL`, `ISSUE_NUMBER`는 실제 값으로 바꿉니다.

1. 같은 목적의 열린·닫힌 Issue와 기존 보드 항목을 검색합니다.

   ```sh
   gh issue list --repo channprj/backup-mic --state all --search "KEYWORD" --json number,title,state,url,projectItems
   gh project item-list 7 --owner channprj --limit 100 --format json
   ```

2. 중복이 없으면 [제목 기준](#태스크-제목-작성)에 맞는 제목과 leaf 본문으로 Issue를 만듭니다. 에이전트가 만드는 Issue에는 [세션 정보](#에이전트-세션-기록)도 넣습니다. 부모나 선행 Issue가 있으면 본문에 링크합니다.

   ```sh
   gh issue create --repo channprj/backup-mic --title "fix(backup): 중단된 작업 재시도 오류 수정" --body-file ISSUE_BODY.md --assignee channprj
   ```

3. 보드 등록을 확인합니다. 자동 추가가 늦거나 빠졌다면 직접 추가합니다. `item-add`는 이미 등록된 항목이면 같은 항목을 돌려줍니다.

   ```sh
   gh issue view ISSUE_NUMBER --repo channprj/backup-mic --json title,url,projectItems
   gh project item-add 7 --owner channprj --url ISSUE_URL --format json
   ```

4. `Priority`와 `Size`를 예상치로 채웁니다. 확신이 서지 않으면 비워 둡니다.

   ```sh
   gh project item-edit 7 --owner channprj --url ISSUE_URL --field "Priority" --value "P1"
   gh project item-edit 7 --owner channprj --url ISSUE_URL --field "Size" --value "M"
   ```

5. 착수할 때 Status를 바꾸고, 에이전트 종류·모델·추론 강도·세션 ID·담당 범위를 담은 착수 댓글을 남깁니다.

   ```sh
   gh project item-edit 7 --owner channprj --url ISSUE_URL --field "Status" --value "In progress"
   gh issue comment ISSUE_NUMBER --repo channprj/backup-mic --body "착수 — 에이전트 종류: TOOL / 모델: MODEL / 추론 강도: EFFORT / 세션 ID: SESSION_ID / 범위: TASK_SCOPE"
   ```

6. 다시 조회해 보드에 반영됐는지 확인합니다. 결과가 바로 보이지 않으면 짧은 간격으로 횟수를 정해 재조회하고, 확인되지 않은 상태에서 새 Issue를 또 만들지 않습니다.

   ```sh
   gh project item-list 7 --owner channprj --limit 100 --format json
   ```

웹 Issue form의 `projects` 설정은 생성자에게 보드 쓰기 권한이 있을 때만 동작합니다. 자세한 제한은 GitHub의 [Issue form 문법](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms)을 참고합니다.

## 저장소가 직접 확인하는 것

보드와 저장소가 조용히 어긋나지 않도록 저장소가 두 가지를 검사합니다.

- **커밋마다 Issue**: `.githooks/commit-msg`가 메시지에서 `Refs|Closes|Fixes #번호`를 찾습니다. 없으면 커밋을 거부하고 무엇을 넣어야 하는지 알려 줍니다. `pnpm install`의 `prepare` 단계나 `pnpm run setup:hooks`로 설치되며, 기존 사용자 훅이나 다른 `core.hooksPath`가 있으면 덮어쓰지 않고 멈춥니다.
- **면제 대상**: 버전 올림으로 만드는 `chore(release)` 커밋과 git이 만드는 머지·되돌리기 커밋입니다. 사람이 범위를 정한 작업이 아니기 때문입니다.

```sh
pnpm run tasks:check                          # origin/main..HEAD
node scripts/task-refs.mjs range v0.260815.0..HEAD
```

이 명령은 참조가 빠진 커밋을 먼저 보여 주고, 모두 참조가 있으면 그 번호가 실제 Issue이고 보드에 등록되어 있는지 확인합니다. `gh`가 없거나 로그인되어 있지 않거나, 로그인한 계정이 비공개 보드를 볼 수 없으면 보드 확인만 건너뜁니다. 여러 GitHub 계정을 쓴다면 `gh auth switch --user channprj`로 바꾼 뒤 다시 실행합니다. 검사 자체의 테스트는 `pnpm test`에 포함됩니다.

## 상태 사용법

| 작업 단계 | Status | 사용 기준 |
| --- | --- | --- |
| 계획 | `Backlog` | 아직 leaf로 정리되지 않았거나 우선순위가 정해지지 않음 |
| 실행 대기 | `To do` | leaf 기준과 선행 조건을 충족해 바로 착수할 수 있음 |
| 진행 | `In progress` | 담당자가 구현 또는 검증 중 |
| 검토 | `In review` | 결과 보고 후 검토·통합 중 |
| 완료 | `Done` | 검토와 필요한 검증, main 푸시까지 완료 |
| 종료 | `Closed` | 취소, 중복, 대체, 닫기로 한 보류 등으로 완료하지 않고 종료 |

보드에는 `Blocked`가 없습니다. 막힌 작업은 현재 Status를 유지하고, Issue 본문이나 댓글에 원인, 해제 조건, 다음 행동을 적습니다. 선행 Issue가 원인이면 링크와 관계도 함께 고칩니다.

Status는 최종 보고 때 몰아서 바꾸지 않고, 다음 사건이 일어날 때마다 바꾼 뒤 다시 조회합니다.

| 사건 | Status | Issue에 남길 내용 |
| --- | --- | --- |
| leaf 조건과 선행 조건 충족 | `To do` | 실행 가능한 범위와 선행 조건 확인 결과 |
| 범위·우선순위 미정 또는 선행 작업 대기 | `Backlog` | 대기 이유, 선행 Issue 링크, 다음 판단 조건 |
| 담당 배정·작업 착수 | `In progress` | 담당 에이전트, 작업 범위, 착수 기준 |
| 결과 보고·검토 착수 | `In review` | 변경 범위, 검증 명령과 결과, 커밋·PR 링크 |
| 검토 수정 재착수 | `In progress` | 검토 지적, 수정 범위, 재검증 방법 |
| 작업 차단 | 현재 Status 유지 | 원인, 선행 Issue 링크, 해제 조건, 다음 행동 |
| 차단 해소·작업 재개 | 작업 단계에 맞는 Status | 해소된 조건, 현재 담당자, 재개 범위와 다음 검증 |
| 검토·검증 완료 | `Done` | 완료 조건별 검증 결과와 커밋·PR 링크 |
| 취소·중복·대체·닫는 보류 | `Closed` | 종료 사유와 대체 Issue 링크(있는 경우) |

댓글은 의미 있는 전환과 판단 근거를 남기는 용도입니다. 단순 대기, 같은 상태의 반복 조회, 반복 도구 실행은 댓글로 남기지 않습니다.

### 보류 항목

`pending` 라벨은 사용자가 의도적으로 뒤로 미룬 항목에 답니다. 선행 작업이나 외부 입력을 기다리는 차단과 다르며, 할 수는 있지만 지금은 하지 않기로 정한 상태입니다.

- 보류 항목은 닫지 않고 `Backlog`에 열어 둡니다. 사용자가 닫기로 정한 보류만 `Closed`로 닫습니다.
- 사용자가 다시 진행하라고 하기 전에는 착수, leaf 분해, Status 변경, 라벨 제거를 하지 않습니다. 다른 작업과 범위가 겹치면 그 사실만 해당 Issue에 댓글로 남깁니다.
- 새로 보류할 때는 `pending` 라벨을 달고 Status를 `Backlog`로 둡니다. 본문에는 기다리는 동안의 동작, 담당, 다시 검토할 조건을 적습니다.
- 다시 열 때는 사용자의 지시를 댓글에 인용해 남기고 라벨을 뗀 뒤, [leaf 기준](#leaf-issue-기준)을 채우고 `To do`로 옮깁니다.

```sh
gh issue edit ISSUE_NUMBER --repo channprj/backup-mic --add-label "pending"
gh issue list --repo channprj/backup-mic --label "pending" --state open --json number,title
```

### Issue 닫기

사유 없이 Issue를 닫지 않습니다. `Done`이든 `Closed`든 닫는 댓글에 사유를 남기고, 댓글·close reason·보드 Status를 한 번에 맞춥니다.

| 보드 Status | close reason | 댓글 첫머리 | 댓글에 남길 내용 |
| --- | --- | --- | --- |
| `Done` | `completed` | `## 완료` | 완료 조건별 결과와 근거, 커밋 등 최종 증거, 남은 위험과 후속 Issue 링크 |
| `Closed` | `not planned` | `## 종료 — <사유>` | 완료하지 않는 이유, 판단 근거, 남은 일을 옮긴 Issue 링크 |
| `Closed` | `duplicate` | `## 종료 — 중복` | 원본 Issue 링크와 중복으로 판단한 근거 |

```sh
gh issue close ISSUE_NUMBER --repo channprj/backup-mic --reason completed --comment "$(cat close.md)"
gh issue close ISSUE_NUMBER --repo channprj/backup-mic --reason "not planned" --comment "$(cat close.md)"
gh issue close ISSUE_NUMBER --repo channprj/backup-mic --duplicate-of ORIGINAL_NUMBER --comment "$(cat close.md)"
```

닫힌 항목은 자동화가 `Done`으로 옮기므로, `not planned`·`duplicate`로 닫았다면 Status를 `Closed`로 다시 바꿉니다. 그런 다음 Issue의 `state`·`stateReason`·`projectItems`를 조회해 셋이 맞는지 확인합니다. 전제가 무너진 Issue는 제목과 범위를 바꿔 다른 작업에 재사용하지 않고, 원래 제목대로 `Closed`로 닫은 뒤 남은 일은 새 Issue로 나눕니다.

`Refs #N` 커밋을 main에 올리고 검증까지 마친 세션은 그 세션 안에서 Issue를 닫습니다. 닫지 못하고 끝내야 한다면 남은 완료 조건을 댓글로 남깁니다.

## 완료 보고와 재개

서브 에이전트에 leaf를 맡겼다면 결과 보고에 다음 내용을 받습니다.

- Issue 번호와 링크
- 에이전트 종류, 모델, 추론 강도와 실제 세션 ID(확인 불가이면 사유)
- 변경한 파일
- 검증 명령과 결과
- 남은 위험과 차단 사항

결과 보고를 받으면 바로 `In review`로 옮기고 검토·통합과 필요한 검증을 진행합니다. 같은 checkout을 고치는 구현은 파일 범위가 달라도 차례로 진행하고, Git 조작은 주 에이전트 한 명이 맡습니다. main 푸시와 검증을 마친 경우에만 `Done`으로 바꿉니다.

새 세션이나 중단 후 재개할 때는 먼저 보드의 Status·관계와 Issue 본문·댓글·세션 기록을 읽고, 실제 Git 커밋·작업 트리·테스트 결과와 대조합니다. 어긋난 곳이 있으면 Git과 검증 결과를 기준으로 Issue와 보드를 바로잡은 뒤 진행하며, 이미 끝난 leaf를 다시 실행하지 않습니다.

## Projects 권한이 없을 때

조회나 변경이 Projects 인증 범위 부족으로 실패하면 다음 인증만 요청합니다.

```sh
gh auth refresh -s project
```

그동안 할 수 있는 로컬 준비와 차단 원인을 Issue 댓글이나 작업 기록에 남깁니다. 인증 뒤에는 새 Issue를 만들지 않고 기존 Issue를 재사용해 보드 등록부터 다시 확인합니다. Projects API의 조회에는 `read:project`, 변경에는 `project` 범위가 필요합니다. 자세한 내용은 GitHub의 [Projects API 인증 안내](https://docs.github.com/en/issues/planning-and-tracking-with-projects/automating-your-project/using-the-api-to-manage-projects)를 참고합니다.
