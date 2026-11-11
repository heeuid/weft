# weft

TUI 그리드(날실) 사이에 GUI(씨실)를 엮어 넣은 터미널. Rust (`eframe/egui` + `portable-pty` + `vte`).

- **두꺼운 로컬, 얇은 원격**: 렌더링·편집·하이라이트·검색 필터·정렬·히스토리·자동완성 순위는 전부 로컬에서 처리합니다.
  원격에서 꼭 필요한 일(파일 읽기/쓰기, 디렉터리 목록, 프로세스 표, grep)만 **ssh + POSIX 도구 + python 표준 라이브러리**로 합니다.
- **원격 디스크에 아무것도 저장하지 않음**: 셸 통합 rc는 `bash --rcfile <(… | base64 -d | gzip -dc)` 프로세스 치환으로,
  에이전트는 `python3 -c "exec(stdin…)"`에 stdin으로 소스를 흘려 넣어 메모리에서만 실행됩니다.

## 빌드 / 실행

```powershell
cd C:\Users\sunghee\claude-workspace\weft
cargo run --release
```

설정·로그·히스토리는 실행 파일 옆 `data/`에 저장됩니다 (`WEFT_DATA_DIR`로 변경 가능).

| 경로 | 내용 |
|---|---|
| `data/settings.json` | 폰트 크기, 호스트 프로필, 세션(탭/분할/작업 디렉터리) |
| `data/history/<host>.txt` | 호스트별 명령 히스토리 (컴포저 ↑↓/제안에 사용) |
| `data/logs/weft.log` | 앱 이벤트, PTY spawn 명령 |
| `data/logs/agent-<host>.log` | 에이전트와 주고받은 모든 JSON 요청/응답 |

## GUI와 TUI를 엮는 방식

| 기능 | 설명 |
|---|---|
| **명령 블록** | OSC 133으로 명령 경계를 추적. 블록마다 왼쪽 거터 바(실행 중 파랑 / 성공 초록 / 실패 빨강), 프롬프트 줄 오른쪽에 `✓ 0.3s` / `✕ exit 2 · 1.2s` 배지. 마우스를 올리면 툴바: 명령 복사, 출력 복사, 다시 실행, 컴포저로 편집, 출력 접기, 출력을 에디터로 열기. 거터 바 클릭 = 출력 복사. |
| **인라인 위젯** | 원격 헬퍼가 OSC 7777로 위젯을 보내면서 같은 줄 수만큼 텍스트 폴백을 출력 → GUI가 정확히 그 줄들 위에 덮어 그립니다. 복사/붙여넣기와 일반 터미널에서도 텍스트로 동작. |
| **컴포저** (`Ctrl+Space`) | 프롬프트 위치에 뜨는 GUI 다중 줄 입력창. Tab 경로 완성(로컬 캐시 우선, 없으면 에이전트), ↑↓ 접두어 히스토리, `→` 히스토리 제안 수락, Enter 실행(bracketed paste). |
| **스크롤백 검색** (`Ctrl+Shift+F`) | 로컬 검색, 일치 하이라이트, Enter/Shift+Enter 이동. |
| **Ctrl+클릭** | 경로(`src/main.rs:12:3` 포함) → 로컬 에디터에서 해당 줄, URL → 브라우저. Ctrl을 누르면 밑줄 미리보기. |
| **TUI 앱** | 대체 화면(vim/htop/less)은 순수 TUI로 표시, 마우스 리포팅(SGR 1006), 대체 화면 휠 → 화살표. |

### 원격 헬퍼 (rc에 포함, 원격 설치 불필요)

| 명령 | 동작 |
|---|---|
| `wls [DIR]` | 정렬 가능한 파일 테이블. 디렉터리 더블클릭 = `cd`, 파일 더블클릭 = 에디터 |
| `CMD \| wtable` | 공백/TSV 컬럼 출력(`ps aux`, `df -h` …)을 정렬 가능한 표로 |
| `x=$(wchoose a b c)` | GUI 선택 목록, 선택값을 stdout으로 (번호 입력도 가능) |
| `wprogress PCT [LABEL] [ID]` | 현재 줄에 진행 막대 |
| `wopen FILE [LINE]` | 로컬 에디터로 열기 (에이전트가 읽고, 저장 시에만 씀; 원격 mtime 충돌 감지) |
| `wnotify MSG` / `wclip` | 토스트 / 로컬 클립보드로 복사(OSC 52) |

## 마크다운 편집 (에디터)

`.md` 파일은 **편집창이 곧 렌더링된 문서**입니다(툴바 `Rich`; `Source`로 원문 보기).
- 제목 크기·굵게·기울임·취소선·`코드`·링크·인용 막대·목록 점·체크박스(클릭으로 토글)·코드 블록 배경·구분선
- 표: `|`가 숨겨지고 열이 글자 폭에 맞춰 정렬되며(`:--`/`:-:`/`--:` 정렬 지원) 헤더는 굵게, 격자선이 그려집니다. 커서를 넣으면 원문 표시.
- 개행: 기본은 CommonMark(저장소의 `.md` 파일 화면)처럼 문단·인용 안의 **한 번 개행은 공백**으로 이어집니다.
  빈 줄(인용은 `>`만 있는 줄), 줄 끝 공백 두 개, `\`, `<br>`는 줄바꿈. 커서가 있는 문단은 원래 줄대로 보입니다.
  GitHub PR 본문/댓글처럼 한 번 개행도 줄바꿈으로 보려면 `data/settings.json`의 `"markdown_breaks": true`
  또는 팔레트 `Markdown: toggle line breaks`.
- 커서가 있는 줄만 원문 기호(`#`, `**`, `[]()` …)가 흐리게 보여 그 자리에서 고칩니다. 나머지 줄은 기호가 숨겨집니다.
- ```` ```mermaid ```` 블록은 그림으로 보입니다(커서를 넣으면 코드로 바뀜). 로컬의 mermaid-cli(`mmdc`)로 그리며,
  없으면 블록에 `need mermaid-cli`가 표시됩니다. 설치하면 재시작 없이 잡힙니다(PATH, `%APPDATA%\npm`, Node.js 폴더 검색).
  그린 결과는 `data/cache/mermaid/`에 저장돼 다음부터 바로 뜹니다.

## 화면 구성

```
창 > 레이아웃(분할) > 패널 > (터미널 패널 안) 탭 > 분할 > 페인
```

| 패널 | 내용 |
|---|---|
| **사이드 패널** (`Ctrl+Shift+B`로 표시/숨김) | Files / Commands / Search / Procs |
| **에디터** (`Ctrl+Shift+J`) | 열린 파일 탭, 마크다운은 편집창이 곧 렌더링 |
| **터미널** | 탭바 → 탭마다 분할된 터미널 **페인** |

- 세 패널 모두 헤더 왼쪽의 손잡이(⋮⋮)를 **끌어서 어디로든 옮길 수 있습니다**. 터미널은 탭바 맨 왼쪽에 손잡이가 있습니다.
  - **다른 패널 위에 놓기**: 그 패널의 위·아래·왼쪽·오른쪽 중 가까운 절반이 강조되고, 놓으면 그쪽에 붙습니다
    (예: 에디터를 Files의 아래 절반에 → 왼쪽 열을 위아래로 나눠 씀).
  - **창 가장자리 근처에 놓기**: 창 전체의 그쪽 가장자리로 갑니다.
  - 손잡이 우클릭 = `Move to the … edge` 메뉴.
- 패널 사이 경계선을 끌면 비율 조절. 배치(비율 포함)는 `settings.json`의 `layout`에 저장되고,
  숨긴 패널은 다시 보이면 원래 자리로 돌아옵니다.
- 크기: 상태바 오른쪽 `A− 14 A+`(터미널·에디터 글자)와 `− 100% +`(아이콘·탭·패널까지 화면 전체). 숫자 클릭 = 기본값.
  `Ctrl+휠` = 글자 크기, `Ctrl+Shift+휠` = 화면 배율, `Ctrl+0` = 글자 크기 초기화.

구조와 데이터 흐름은 [docs/architecture.md](docs/architecture.md).

## 사이드 패널 (`Ctrl+Shift+B`)

- **Files**: 터미널 cwd(OSC 7)를 따라가는 트리. 필터는 로컬 캐시에서 즉시. 클릭=열기, 더블클릭=cd.
- **Commands**: 현재 패널의 명령 블록 목록 (클릭 시 스크롤).
- **Search**: 원격은 `grep -rnI`(POSIX), 로컬은 Rust로 검색. 결과 클릭 = 해당 줄 열기.
- **Procs**: 원격 `ps` 표, 정렬/필터, 우클릭 TERM/KILL.

## 연결

`Ctrl+Shift+N` → `user@host [-p 2222 …]` 또는 `~/.ssh/config`의 Host 선택.

- 패널: `ssh -tt host '<부트스트랩>'` (호스트 키 확인/비밀번호 입력도 패널에서 그대로)
- 에이전트: `ssh -T -o BatchMode=yes host '<부트스트랩>'` — 키/ssh-agent 인증이 필요합니다. 실패해도 터미널은 동작하고,
  첫 프롬프트가 뜬 뒤 한 번 자동 재시도합니다(상태바에서 수동 재연결 가능).
- 부트스트랩 문자열에는 `"`와 `\`가 없습니다 → sh/bash/zsh/fish 로그인 셸, MSVCRT `ssh.exe`, msys `ssh` 모두에서 같은 의미.
  rc는 gzip+base64로 보내 ConPTY의 약 8KB 명령줄 제한 안에 들어갑니다.
- `posix-loopback` 프로필: Git Bash를 원격처럼 사용(ssh와 완전히 같은 경로)해서 서버 없이 시험해 볼 수 있습니다.
- 로컬 패널은 PowerShell(+ PSReadLine 통합)이며 파일 작업은 에이전트 없이 Rust `std::fs`로 처리합니다.

## 단축키

| 키 | 동작 |
|---|---|
| `Ctrl+Shift+P` | 커맨드 팔레트 |
| `Ctrl+Shift+T` / `Ctrl+Shift+N` | 새 탭(같은 호스트·cwd) / 연결 |
| `Ctrl+Shift+D` / `Ctrl+Shift+E` / `Ctrl+Shift+W` | 오른쪽 분할 / 아래 분할 / 패널 닫기 |
| `Ctrl+Shift+Z` | 현재 패널만 탭 전체로 확대 / 원래 분할로 복귀 (패널 헤더 버튼, 우클릭 메뉴에도 있음) |
| `Alt+←↑→↓` | (분할 시) 패널 포커스 이동 |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 탭 이동 |
| `Ctrl+Shift+↑/↓` | 이전/다음 명령 블록으로 스크롤 |
| `Ctrl+C` | 선택 영역 있으면 복사, 없으면 `^C` |
| `Ctrl+V` | 붙여넣기 (bracketed paste) |
| `Ctrl+= / Ctrl+-` | 글꼴 크기 |
| `Ctrl+S` | (에디터) 저장 |

## 디자인

- 색은 전부 `src/theme.rs`의 토큰(배경 4단계, 텍스트 3단계, 단일 액센트, success/warn/error)에서만 나옵니다.
- 테마는 고를 수 있습니다: **Catppuccin Mocha(기본) · Macchiato · Frappé · Latte**, **Graphite Dark · Light**. Catppuccin은 공식 색과 공식 터미널 ANSI 매핑을 그대로 씁니다(MIT, https://catppuccin.com).
  - 탭바 오른쪽 해/달 버튼 → 색 견본이 있는 테마 목록, 또는 커맨드 팔레트의 `Theme: …`
  - "Follow system": OS가 다크면 마지막에 고른 다크 테마, 라이트면 마지막에 고른 라이트 테마
  - `data/settings.json`: `"theme"`(테마 id 또는 `"system"`), `"theme_dark"`, `"theme_light"`
  - 모든 테마의 본문 글자 대비(6.5:1 이상)를 테스트로 확인합니다. Catppuccin Latte의 노란색은 원래 옅은 색이라 밝은 배경에서 약하게 보입니다.
- UI 아이콘은 `src/icons.rs`에서 선으로 직접 그립니다 → 폰트에 글리프가 없어 □로 깨지는 일이 없습니다.
- 폰트 폴백 체인: 터미널 `Cascadia Mono → 맑은 고딕 → Segoe UI Symbol → egui 기본`, UI `Segoe UI → 맑은 고딕 → Segoe UI Symbol`.
  ASCII는 런 단위, 그 외 문자는 셀 단위로 그려 폴백 폰트가 열 정렬을 밀지 않습니다(한글 2칸 정렬).

## 테스트

```powershell
cargo test                      # 단위 테스트
cargo test -- --include-ignored # + Git Bash를 원격 대역으로 쓰는 E2E (인메모리 rc, OSC 133/7/7777, stdin 에이전트)
```

## 구조

```
remote/rc.sh      bash 통합 (OSC 133/7, w* 헬퍼) — 메모리로만 전달
remote/agent.py   python 표준 라이브러리 에이전트 (JSON lines) — stdin으로 전달
src/term.rs       VT/xterm 그리드, 절대 줄 번호 스크롤백, 블록, OSC 이벤트
src/host.rs       프로필, 부트스트랩 명령 조립, 에이전트 연결, 로컬 백엔드
src/pty.rs        portable-pty 래퍼 (Windows 프로세스 트리 종료)
src/app.rs        상태/로직, 레이아웃, 요청/응답, 액션
src/ui_term.rs    그리드 렌더링 + 블록 크롬 + 입력 + 컴포저/검색
src/widgets.rs    인라인 위젯
src/ui_panels.rs  탭/상태바/사이드 패널/에디터/팔레트/연결/토스트
src/theme.rs      색 토큰, 폰트 체인        src/icons.rs  벡터 아이콘
src/md.rs         마크다운 in-place 레이아웃 src/mermaid.rs mmdc 렌더링/캐시
```
