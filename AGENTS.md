# AGENTS.md

이 저장소에서 작업하는 사람과 코딩 에이전트(Claude Code, Codex 등)가 함께 따르는 규칙입니다.

## GitHub Project

- 실행할 수 있는 모든 작업의 진행 상태는 [backup-mic Project](https://github.com/users/channprj/projects/7/views/1)를 원본으로 삼는다. Issue 생성, 보드 등록, 상태 변경, 닫기 절차는 [GitHub Project 운영 가이드](docs/github-project-workflow.md)를 따른다.
- 사용자가 코딩 에이전트나 도구로 작업을 지시하면, 지시를 받은 에이전트는 구현·문서·운영 작업을 시작하기 전에 같은 목적의 열린·닫힌 Issue를 검색하고, 없으면 Issue(부모 또는 leaf)를 만들어 보드에 등록한다. 어떤 에이전트나 도구를 쓰든 같다.
- Issue 없이 시작해도 되는 경우는 leaf로 나누어 등록하기 전의 준비, Projects 인증이 막힌 동안의 로컬 준비, 사용자가 명시적으로 예외를 지정한 경우뿐이다. 이때도 작업을 마치기 전에 Issue를 만들어 커밋이 가리킬 대상을 남긴다.
- Issue 제목은 `type: 핵심 대상 및 작업` 또는 `type(scope): 핵심 대상 및 작업` 형식의 간결한 한국어 명사형으로 쓴다. `유지한다`, `추가하기` 대신 `유지`, `추가`, `수정`, `개선`처럼 끝낸다. 예: `fix(backup): 중단된 작업 재시도 오류 수정`. 자세한 기준은 [태스크 제목 작성](docs/github-project-workflow.md#태스크-제목-작성)을 따른다.
- Issue의 기본 assignee는 GitHub 사용자 `channprj`다. 사용자가 다른 담당자를 지정한 경우에만 바꾸며, 서브 에이전트 담당은 assignee가 아니라 배정 댓글에 기록한다.
- 보드에 등록한 Issue에는 `Priority`와 `Size`를 예상치로 채운다. 확신이 서지 않으면 비워 둔다.
- 큰 목표는 부모 Issue로 묶고, 결과 하나와 명확한 파일 범위·완료 조건·검증 방법·선행 태스크를 가진 leaf Issue에만 실행을 배정한다. 서브 에이전트에는 leaf Issue만 맡긴다.
- AI 에이전트로 작업하는 Issue에는 착수 전에 본문 `에이전트 세션` 절에 담당 역할, 에이전트 종류, 모델, 추론 강도, 실제 세션 ID, 재개 방법, 작업 경로·브랜치, 시작 일시(KST)를 기록한다. 새 세션·도구 전환·모델이나 추론 강도 변경 시 이전 행을 보존하고 새 행과 착수·재개 댓글을 추가한다. 값을 확인할 수 없으면 `확인 불가`와 이유를 적고 추측하지 않는다. 대화·로그 원문과 자격증명은 올리지 않는다. 형식은 [에이전트 세션 기록](docs/github-project-workflow.md#에이전트-세션-기록)을 따른다.
- Status와 Issue 댓글은 배정·착수, 결과 보고·검토, 수정 재착수, 차단·재개, 검증 완료가 일어날 때 갱신하고 다시 조회한다. 보드에 없는 Status를 만들지 않으며, 단순 대기나 반복 도구 실행은 댓글로 남기지 않는다.
- 사유 없이 Issue를 닫지 않는다. `Done`은 `completed`, `Closed`는 `not planned`·`duplicate`로 닫으면서 완료 근거나 종료 사유를 댓글로 남기고 보드 Status를 함께 맞춘다. 양식은 [Issue 닫기](docs/github-project-workflow.md#issue-닫기)를 따른다.
- `pending` 라벨이 붙은 Issue는 사용자가 의도적으로 뒤로 미룬 항목이다. `Backlog`에 열어 두고, 사용자가 다시 진행하라고 하기 전에는 착수·분해·Status 변경·라벨 제거를 하지 않는다. 다른 작업과 범위가 겹쳐도 그 Issue의 일을 끌어와 구현하지 않는다.
- 새 세션은 보드·Issue와 실제 Git·검증 상태를 대조해 재개한다. 모든 leaf의 완료 조건을 확인하기 전에는 부모 Issue를 완료하지 않는다.

## 커밋

- 커밋 메시지에는 그 작업이 속한 Issue를 `Refs #번호`로 남긴다. 검증을 마치기 전에는 `Closes #번호`로 완료를 미리 선언하지 않는다.
- `pnpm install`의 `prepare` 단계 또는 `pnpm run setup:hooks`로 설치되는 `commit-msg` 훅이 참조를 확인한다. `chore(release)` 커밋과 머지·되돌리기 커밋은 면제한다.
- `pnpm run tasks:check`는 범위 안의 커밋이 모두 Issue를 참조하는지, 그 Issue가 실제로 있고 보드에 등록되어 있는지 확인한다. 기본 범위는 `origin/main..HEAD`다. `gh`를 쓸 수 없거나 로그인한 계정이 보드를 볼 수 없으면 보드 확인은 건너뛰고 작업을 막지 않는다.
