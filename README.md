# weft

TUI 그리드(날실) 사이에 GUI(씨실)를 엮어 넣은 터미널. Rust (`eframe/egui` + `portable-pty` + `vte`).

- **두꺼운 로컬, 얇은 원격**: 렌더링·편집·하이라이트·검색 필터·정렬·히스토리·자동완성 순위는 전부 로컬에서 처리합니다.
  원격에서 꼭 필요한 일(파일 읽기/쓰기, 디렉터리 목록, 프로세스 표, grep)만 **ssh + POSIX 도구 + python 표준 라이브러리**로 합니다.
- **원격 디스크에 아무것도 저장하지 않음**: 셸 통합 rc는 `bash --rcfile <(… | base64 -d | gzip -dc)` 프로세스 치환으로,
  에이전트는 `python3 -c "exec(stdin…)"`에 stdin으로 소스를 흘려 넣어 메모리에서만 실행됩니다.

## 빌드 / 실행

실행 스크립트: 필요하면 먼저 빌드(`cargo build --release`, 최신이면 1초 이내)하고, weft를 **터미널과 분리해** 띄운 뒤 바로 끝납니다.
띄운 터미널을 닫아도 weft는 계속 동작합니다.

| OS | 더블클릭 | 터미널에서 |
|---|---|---|
| Windows | 탐색기에서 `weft.bat` | cmd: `weft.bat`, PowerShell: `.\weft.bat` |
| Linux | 파일 관리자에서 `weft.desktop` (처음엔 "실행 허용"/"Allow Launching" 필요할 수 있음) | `./weft.sh` |
| macOS | (설치하면 Launchpad·Spotlight의 weft) | `./weft.sh` |

- Windows는 PowerShell `Start-Process`(탐색기 더블클릭과 같은 방식)로 띄워 터미널의 핸들을 하나도 물려주지 않습니다.
- Linux는 `setsid`(없으면 `nohup`)로 새 세션에서 띄워 터미널을 닫을 때의 종료 신호가 가지 않습니다. macOS에는 `setsid`가 없어 `nohup`으로 띄웁니다.
- Linux와 macOS는 빌드(컴파일)까지만 확인했고, 실제 창을 띄워 본 적은 아직 없습니다.
- 빌드에 실패하면 오류를 보여 주고 멈춥니다. `cargo`가 없으면 이미 빌드된 실행 파일을 띄웁니다.

개발 중에는 그냥 `cargo run --release`도 됩니다.

### Windows에 설치 / 제거

관리자 권한 없이 이 사용자에게만 설치합니다. weft는 실행 파일 하나(셸 스크립트와 원격 에이전트가 들어 있음)라 그 파일만 복사합니다.

| | 명령 |
|---|---|
| 설치 (`%LOCALAPPDATA%\Programs\weft`) | `install.bat` |
| 다른 위치에 | `install.bat D:\Apps\weft` |
| 바탕화면 바로가기도 / PATH에도 | `install.bat desktop path` |
| 제거 (데이터는 물어봄) | `uninstall.bat`, 또는 설정 > 앱에서 weft 제거 |
| 데이터까지 / 데이터는 남기고 | `uninstall.bat purge` / `uninstall.bat keep` |

- 설치: `cargo`가 있으면 먼저 빌드합니다. 시작 메뉴에 바로가기를 만들고 설정 > 앱 목록에 올립니다. 다시 실행하면 업데이트입니다. weft가 켜져 있어도 되고, 다음에 켤 때 새 버전으로 뜹니다.
- 데이터는 설치 폴더의 `data\`입니다. `weft.bat`으로 쓰던 `target\release\data`가 있으면 처음 설치할 때 복사해 옵니다.
- 제거: 바로가기, 앱 목록 항목, PATH 항목, 실행 파일을 지웁니다. 켜져 있는 weft가 있으면 닫을 때까지 기다립니다. 데이터는 지울지 묻고, 남기면 다시 설치할 때 그대로 이어 씁니다.

### Linux / macOS에 설치 / 제거

관리자 권한 없이 이 사용자에게만 설치합니다. 스크립트 하나가 두 OS를 다 맡습니다.

| | 명령 |
|---|---|
| 설치 (Linux `~/.local/opt/weft`, macOS `~/Applications/weft`) | `./install.sh` |
| 다른 위치에 | `./install.sh /some/dir` |
| 바탕화면 바로가기도 / 터미널에서 `weft`로도 | `./install.sh desktop path` |
| 제거 (데이터는 물어봄) | 설치 폴더의 `./uninstall.sh` (또는 저장소의 `./uninstall.sh`) |
| 데이터까지 / 데이터는 남기고 | `./uninstall.sh purge` / `./uninstall.sh keep` |

- 설치: `cargo`가 있으면 먼저 빌드합니다. 다시 실행하면 업데이트입니다. 켜져 있는 weft는 그대로 돌고, 다음에 켤 때(또는 weft의 restart 버튼으로) 새 버전이 뜹니다.
- Linux: 설치 폴더에 실행 파일, 아이콘, `data/`. 앱 메뉴에 weft가 생깁니다(`~/.local/share/applications/weft.desktop`).
- macOS: 설치 폴더에 `weft.app`과 `data/`가 나란히 있습니다. Launchpad·Spotlight·Dock에서 쓰는 보통 앱이고, 데이터는 앱 안이 아니라 옆의 `data/`에 둡니다(앱을 바꿔도 데이터는 그대로). 그 Mac에서 빌드한 것이라 처음 열 때 보안 경고가 뜨지 않습니다. 앱 아이콘은 macOS 내장 도구(`sips`, `iconutil`)로 만듭니다.
- `path`: `~/.local/bin/weft` 링크를 만들고, 그 폴더가 PATH에 없으면 쓰는 셸의 설정 파일에 한 줄을 넣습니다(zsh `~/.zshrc`, bash는 Linux `~/.bashrc`·macOS `~/.bash_profile`, fish `~/.config/fish/conf.d/weft.fish`, 그 밖 `~/.profile`). 어느 파일에 무엇을 넣었는지 보여 주고, 새로 연 터미널부터 적용됩니다. 제거할 때 그 줄만 뺍니다.
- 데이터: `./weft.sh`로 쓰던 `target/release/data`가 있으면 처음 설치할 때 복사해 옵니다.
- 제거: 설치가 만든 것(앱 메뉴 항목, 바탕화면 바로가기, PATH 링크)은 설치 폴더의 `.installed`에 적어 두었다가 그것만 지웁니다. 켜져 있는 weft가 있으면 닫을 때까지 기다립니다.
- macOS에서 `weft.app`을 먼저 휴지통에 버렸어도(Linux에서 실행 파일을 먼저 지웠어도) 설치 폴더의 `uninstall.sh`로 나머지(바로가기, 링크, 데이터)를 정리할 수 있습니다.
- 스크립트 시험: `python tools/install_test.py`(가짜 weft로 두 OS의 설치·업데이트·제거를 임시 홈 폴더에서 돌려 봄. Windows에서는 Git Bash의 `sh`로).

설정·로그·히스토리는 실행 파일 옆 `data/`에 저장됩니다 (`WEFT_DATA_DIR`로 변경 가능).

| 경로 | 내용 |
|---|---|
| `data/settings.json` | 폰트 크기, 호스트 프로필, 세션(탭/분할/작업 디렉터리), 기록 설정, 콘솔 호스트(`console_host`) |
| `data/records/<날짜>.jsonl` | 끝난 명령마다 한 줄: 명령, 출력, 종료 코드, 시작 시각, 걸린 시간, 디렉터리, 호스트 |
| `data/history/<host>.txt` | 호스트별 명령 줄 (컴포저 ↑↓/제안). 각 줄 앞에 bash 형식 시각 줄 `#<unix초>` |
| `data/logs/weft-<날짜>.log` | 앱 이벤트, 셸 실행 명령, 알림, 오류 |
| `data/logs/agent-<host>-<날짜>.log` | 에이전트와 주고받은 JSON 요청/응답 (한 줄 1500바이트까지) |
| `data/memos/`, `data/fonts/` | 메모, 직접 넣은 폰트 |

**기록 설정** (팔레트 "Privacy…", 또는 History 탭의 `Records…`): 명령·출력, 히스토리, weft 로그, 에이전트 로그 **항목마다** 켜기/끄기, 보관 기간(일, 0 = 계속), 최대 크기(MB, 0 = 제한 없음; 히스토리는 호스트당)를 정하고, 지금 쓰는 용량을 보고 바로 지울 수 있습니다. 명령 하나의 출력 한도(KB, 넘으면 앞뒤를 남기고 가운데를 뺌)도 있습니다. 한도는 시작할 때와 매시간 적용되고, 날짜별 파일은 하루 단위로 지워집니다.

**남기지 않는 것**: 키 입력은 어디에도 쓰지 않습니다. 비밀번호 프롬프트는 입력을 화면에 보이지 않으므로 출력 기록에도 없습니다. ssh 비밀번호는 디스크에 쓰지 않습니다(아래 "비밀번호"). **앞에 공백을 붙여 친 명령**(` export TOKEN=…`)은 bash의 `HISTCONTROL=ignorespace`처럼 명령 기록과 히스토리 모두에서 빠집니다. 대체 화면 앱(nvim, htop…)의 화면은 기록하지 않습니다.

**비밀번호** (같은 Privacy 창): 재접속하려고 세션 동안 들고 있는 ssh 비밀번호는
- 메모리에서 ChaCha20으로 암호화해 둡니다. Windows는 키를 `CryptProtectMemory`로 한 번 더 감싸고, 키 메모리는 스왑에 안 내려가게 잠급니다(`VirtualLock` / `mlock`).
- 키는 정해진 주기마다 새로 만들어 모두 다시 암호화하고 이전 키는 0으로 덮습니다. 주기, 그리고 안 쓰는 비밀번호를 잊는 시간을 초·분·시간 단위로 정합니다(0 = 안 함). 암호화 자체도 끌 수 있습니다.
- ssh에는 환경 변수가 아니라 **일회용 표**로 넘깁니다: ssh 환경에는 포트와 무작위 표만 있고, `weft --askpass`가 `127.0.0.1`로 표를 내고 비밀번호를 받습니다. 표는 2분 안에 최대 3번(비밀번호 재질문, 경유 호스트) 쓰이고 사라집니다.
- Linux는 프로세스를 dump 불가로(`PR_SET_DUMPABLE=0`: 같은 사용자의 ptrace·`/proc/<pid>/mem`·코어 덤프 차단), macOS는 디버거 연결과 코어 덤프를 막습니다. Windows엔 같은 스위치가 없어 같은 사용자 권한의 코드가 weft에 붙으면 복호화 순간을 볼 수 있습니다.
- 가장 좋은 건 비밀번호를 안 쓰는 것: 키 인증 + ssh-agent면 weft가 비밀번호를 가질 일이 없습니다.

## 도움말

`F1` 또는 탭 줄 `?` = 도움말 창: **Commands and keys**(모든 명령을 묶음별로, 단축키와 함께, 클릭하면 실행 + 키·마우스 동작), **Features**(기능 설명). 검색 가능. 탭 줄 오른쪽 끝 `☰` = 묶음별 명령 메뉴.

## GUI와 TUI를 엮는 방식

| 기능 | 설명 |
|---|---|
| **명령 블록** | OSC 133으로 명령 경계를 추적. 블록마다 왼쪽 거터 바(실행 중 파랑 / 성공 초록 / 실패 빨강), 프롬프트 줄 오른쪽에 `✓ 0.3s` / `✕ exit 2 · 1.2s` 배지. 마우스를 올리면 툴바: 명령 복사, 출력 복사, 다시 실행, 컴포저로 편집, 출력 접기, 출력을 에디터로 열기. 거터 바 클릭 = 출력 복사. |
| **인라인 위젯** | 원격 헬퍼가 OSC 7777로 위젯을 보내면서 같은 줄 수만큼 텍스트 폴백을 출력 → GUI가 정확히 그 줄들 위에 덮어 그립니다. 복사/붙여넣기와 일반 터미널에서도 텍스트로 동작. |
| **컴포저** (`Ctrl+Space`) | 프롬프트 위치에 뜨는 GUI 다중 줄 입력창. Tab 경로 완성(로컬 캐시 우선, 없으면 에이전트), ↑↓ 접두어 히스토리, `→` 히스토리 제안 수락, Enter 실행(bracketed paste). |
| **스크롤백 검색** (`Ctrl+Shift+F`) | 로컬 검색, 일치 하이라이트, Enter/Shift+Enter 이동. |
| **History** (`Ctrl+Shift+H`, 사이드 패널) | 모든 호스트·날짜의 명령 기록 검색. 명령은 fuzzy/glob/regex, "in output too"를 켜면 출력에서도(그대로 입력한 글자, 또는 regex). 호스트 필터. 클릭 = 명령과 출력을 에디터로, 우클릭 = 복사, 다시 실행. |
| **Ctrl+클릭** | 경로(`src/main.rs:12:3` 포함) → 로컬 에디터에서 해당 줄, URL → 브라우저. Ctrl을 누르면 밑줄 미리보기. |
| **TUI 앱** | 대체 화면(vim/htop/less)은 순수 TUI로 표시, 마우스 리포팅(SGR 1006), 대체 화면 휠 → 화살표. |
| **마우스를 쓰는 앱** (마우스 리포팅을 켠 앱 전부: nvim `mouse=a`, vim, htop, tmux, lazygit, mc…) | 드래그·우클릭은 앱으로 갑니다(nvim: 드래그 = visual 모드, 우클릭 = nvim 메뉴). **Shift+드래그** = weft 선택(Ctrl+C로 복사), **Shift+우클릭** = weft 메뉴. 앱이 마우스를 안 쓰면 그냥 드래그/우클릭이 weft 것. |
| **앱에서 복사** | OSC 52 클립보드 쓰기 지원을 DA1(`52`)과 XTGETTCAP(`Ms`)로 알립니다. ssh 너머 Neovim 0.10+는 이를 보고 `"+y`(또는 `clipboard=unnamedplus`의 `y`)를 이 컴퓨터 클립보드로 보냅니다. |
| **메모** (`Ctrl+Shift+M`) | 에디터 탭 옆 `+`. `data/memos/memo-N.md`에 마크다운으로, 자동 저장. 탭 이름은 첫 줄, 빈 채로 닫으면 삭제. **불러오기**: 탭 옆 `Memos` 메뉴, 팔레트 `Open memo…`, 빈 에디터의 최근 메모: 제목·고친 시각·미리보기, 검색, 클릭 = 열기, 우클릭 = 삭제. |
| **폰트** (탭 줄 `Aa`) | 터미널 폰트와 아이콘 폰트를 고릅니다. 내장 Nerd Font 3종(**D2Coding 기본**: 한글 포함·2칸 정확, JetBrains Mono(아이콘 보충 기본), Hack; 라이선스는 `fonts/`), `data/fonts/`에 넣은 폰트, 설치된 폰트, 또는 경로. 한글은 맑은 고딕으로 보충. |

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
툴바 `Read`(모든 텍스트 파일) = **읽기 모드**: 선택·복사만 되고 편집은 안 되며, 마크다운은 커서를 넣어도 원문으로 바뀌지 않고 렌더링된 그대로입니다. 다시 누르면 편집.
- 제목 크기·굵게·기울임·취소선·`코드`·링크·인용 막대·목록 점·체크박스(클릭으로 토글)·코드 블록 배경·구분선
- 표: `|`가 숨겨지고 열이 글자 폭에 맞춰 정렬되며(`:--`/`:-:`/`--:` 정렬 지원) 헤더는 굵게, 격자선이 그려집니다. 에디터보다 넓으면 좁은 열은 그대로 두고 넓은 열이 남은 폭을 나눠, 긴 칸은 그 열 안에서 줄바꿈됩니다. 커서를 넣으면 원문 표시.
- 읽기 모드의 표는 스프레드시트처럼 고릅니다: 한 칸 안에서 끌면 그 글자들, 여러 칸에 걸쳐 끌면 그 사이 칸들(사각형), 더블클릭은 한 칸. `Ctrl+C`로 복사(여러 칸은 탭으로 구분, 줄마다 한 행: 엑셀에 그대로 붙음). 우클릭 = `Copy selection` / `Copy table (markdown)` / `Copy as TSV`.
- 더블클릭 = 단어 선택: 공백·줄 끝·괄호·따옴표에서 멈추고, 경로·주소(`src/main.rs`, `user@host`)는 통째로. 한글도 단어 단위.
- 개행: 기본은 CommonMark(저장소의 `.md` 파일 화면)처럼 문단·인용 안의 **한 번 개행은 공백**으로 이어집니다.
  빈 줄(인용은 `>`만 있는 줄), 줄 끝 공백 두 개, `\`, `<br>`는 줄바꿈. 커서가 있는 문단은 원래 줄대로 보입니다.
  GitHub PR 본문/댓글처럼 한 번 개행도 줄바꿈으로 보려면 `data/settings.json`의 `"markdown_breaks": true`
  또는 팔레트 `Markdown: toggle line breaks`.
- 커서가 있는 줄만 원문 기호(`#`, `**`, `[]()` …)가 흐리게 보여 그 자리에서 고칩니다. 나머지 줄은 기호가 숨겨집니다.
- ```` ```mermaid ```` 블록은 그림으로 보입니다(커서를 넣으면 코드로 바뀜). 그리는 프로그램은 에디터 툴바 `Diagrams`에서 고릅니다
  (`settings.json`의 `"mermaid"`):
  - **built-in**(기본): weft에 들어 있는 렌더러(mermaid-svg + resvg). 설치할 것 없음, 시퀀스·플로차트·클래스·상태·간트·파이 등.
  - **mermaid-cli (PATH)**: PATH에 있는 `mmdc` (공식 mermaid와 똑같은 모양, Node.js 필요). 설치 후 재시작 없이 잡힘.
  - **mermaid-cli (path)**: 직접 지정한 `mmdc` 경로.
  그린 결과는 `data/cache/mermaid/`에 저장돼 다음부터 바로 뜹니다.
- 구문 강조: `bat`의 문법 묶음(syntect + two-face, 약 200개 언어, 순수 Rust 정규식)을 weft 테마 색으로.
  에디터의 소스 파일과 마크다운 코드 블록(```` ```rust ````, ```` ```c ```` …) 모두. 바뀐 줄만 다시 칠해서 입력이 끊기지 않고,
  큰 파일은 처음 열 때 간단한 강조로 바로 보여 준 뒤 백그라운드에서 정확한 색으로 바꿉니다. (PowerShell 문법은 묶음에 없음)
- 이미지 파일(png, jpg, gif, webp, bmp, ico)은 에디터에서 그림으로 보입니다(Fit / 1:1 / 확대·축소).
- PDF는 페이지가 세로로 이어져 보입니다(hayro, 순수 Rust). 보이는 페이지만 백그라운드에서 그리고, 확대하면 그 크기로 다시 그립니다.
  이미지·PDF는 64MB까지 읽습니다(원격도 에이전트가 메모리로만 전달).

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
  - 손잡이 우클릭 = `Move to the … edge` 메뉴, 그리고 사이드 패널·에디터는 `Split right` / `Split down` / `Close this panel`.
- **사이드 패널과 에디터는 여러 개**로 나눌 수 있습니다: 손잡이 옆 분할 버튼(클릭 = 아래에 하나 더, 우클릭 = 오른쪽에), 또는 팔레트의 `Side panel: another one below/right`, `Editor: another one below/right`.
  - 나뉜 사이드 패널은 **각자 다른 탭**(예: 위 Files, 아래 Search)을 보여 주고, 탭의 내용(폴더, 검색 결과)은 함께 씁니다.
  - 나뉜 에디터는 열린 파일 목록을 함께 쓰고 **각자 다른 파일**을 보여 줍니다(나눈 직후엔 같은 파일).
  - 나뉜 패널 하나하나도 손잡이로 어디로든 옮깁니다(터미널 옆, 창 가장자리, 다른 패널의 위아래). 여럿일 때 헤더의 `×`로 하나를 닫습니다.
  - 마지막으로 클릭한 사이드 패널·에디터가 명령(History 열기, 파일 열기 등)이 가는 곳입니다.
- **터미널 페인 옮기기**: 분할된 페인의 헤더(제목 부분)를 끌어 같은 탭의 다른 페인 위에 놓습니다. 가장자리 쪽 절반이면 그쪽으로 나뉘어 들어가고, 가운데면 두 페인의 자리를 바꿉니다.
- 패널 사이 경계선을 끌면 비율 조절. 경계가 만나는 점(세 패널이 만나는 ㅓ·ㅗ 모양, 네 패널이 만나는 +)을 잡으면 거기 모인 경계 전부를 한꺼번에(커서가 ✥로 바뀜). 터미널 분할 페인도 같음. 배치(비율 포함)는 `settings.json`의 `layout`에 저장되고,
  숨긴 패널은 다시 보이면 원래 자리로 돌아옵니다.
- 크기: 상태바 오른쪽 `A− 14 A+`(터미널·에디터 글자)와 `− 100% +`(아이콘·탭·패널까지 화면 전체). 숫자 클릭 = 기본값.
  `Ctrl+휠` = 글자 크기, `Ctrl+Shift+휠` = 화면 배율, `Ctrl+0` = 글자 크기 초기화.

구조와 데이터 흐름은 [docs/architecture.md](docs/architecture.md).

## 닫기 전에 묻기, 다시 열기

- **저장 안 한 파일**: 바뀐 내용이 있는 에디터 탭을 닫거나, 그런 파일이 있는 채로 창을 닫거나 재시작하면 묻습니다: `Save and close` / `Close without saving` / `Cancel`(Esc). 메모는 스스로 저장되므로 묻지 않습니다.
- **실행 중인 명령**: 명령이 아직 돌고 있거나 전체 화면 프로그램(vim, htop…)이 떠 있는 터미널을 닫을 때(페인·탭·창·재시작) 무엇이 끝나는지 보여 주고 묻습니다. 셸 통합(OSC 133)이 없는 셸에서는 전체 화면 프로그램만 알 수 있습니다.
- 끄고 켜기: 묻는 창의 체크 상자, 또는 팔레트 `Ask before closing unsaved files` / `Ask before ending running commands` (`settings.json`의 `confirm_unsaved`, `confirm_running`).
- **다시 열기**: 닫을 때 열려 있던 것이 다음에 그대로 열립니다: 터미널 탭·분할·폴더, 에디터의 파일들(Rich / Source, Read 모드, 앞에 있던 탭), Files에서 펼쳐 둔 폴더, 나눈 패널마다 보던 폴더. 그사이 없어진 파일은 탭에서 그렇다고 알려 줍니다.

## 사이드 패널 (`Ctrl+Shift+B`)

- **Files**: 터미널 cwd(OSC 7)를 따라가는 트리. 클릭=선택(폴더는 펼침), 더블클릭=파일 열기 / 폴더로 cd.
  - **↵ 버튼**: 지금 보고 있는 폴더로 터미널을 cd (위로 올라갔다 다시 찾을 필요 없음). 경로 표시줄의 각 폴더를 우클릭해도 `cd terminal here`.
  - **눈 아이콘**: `Hidden files`(점으로 시작하는 이름, Windows 숨김 속성; 기본 끔) / `Git-ignored files`(`.gitignore` 등으로 git이 무시하는 것; 기본 켬) 보기 켜고 끄기. 무시 여부는 그 폴더에서 `git check-ignore`로(원격은 에이전트가) 정하고, 저장소가 아니거나 git이 없으면 모두 보입니다. 둘 중 하나라도 기본과 다르면 아이콘이 강조됩니다.
  - **필터**: 루트 아래 전체에서 찾기. `fuzzy`(기본, 글자 순서만 맞으면: `srcmain` → `src/main.rs`) / `glob`(`*.rs`, `src/**/test_*`, `{a,b}.md`) / `regex`(경로 대상, 대문자가 있으면 대소문자 구분).
    이름 목록은 한 번만 받아 오고(원격은 에이전트가 python으로, `.git`·`node_modules`·`target` 등은 안으로 안 들어감, 최대 2만 개) 매칭·정렬은 로컬에서 즉시.
    결과 더블클릭 = 파일 열기 / 폴더는 트리에서 펼쳐 보여 주기, Enter = 첫 결과.
  - **새 폴더 / 새 파일 / 이름 변경 / 삭제**: 행 우클릭 `New folder…` `New file…` `Rename…` `Delete…`(폴더에서는 그 안에, 파일에서는 그 옆에 만듦), 머리의 `+` = 보고 있는 폴더에 만들기. 로컬·원격 모두. 이름이 이미 있으면 덮어쓰지 않고 알려 줍니다.
    삭제는 묻고 나서 합니다: 이 PC(Windows)에서는 **휴지통으로**, 원격과 그 밖에서는 되돌릴 수 없다고 알린 뒤 지웁니다. 이름을 바꾼 파일이 에디터에 열려 있으면 탭도 새 이름을 따라갑니다.
- **파일 전송** (로컬 ↔ 원격, 원격 ↔ 원격): 파일이나 폴더를
  - Files의 행에서 **끌어** 다른 사이드 패널의 Files(그 폴더), 펼친 폴더 행, 또는 터미널(그 셸의 현재 폴더) 위에 놓거나,
  - 우클릭 `Download`(원격 → 로컬 `Downloads`) / `Send to ▸`(다른 사이드 패널이 보는 폴더, 다른 호스트 터미널의 폴더),
  - Windows 탐색기에서 끌어 Files나 터미널 위에 놓으면 복사됩니다.
  - 사이드 패널을 나눠(손잡이 옆 분할 버튼) 각 패널 머리의 호스트 이름(`local ▾`)에서 다른 호스트를 고르면 양쪽 창(FileZilla처럼)이 됩니다. 새로 나눈 패널은 터미널을 따라가지 않습니다.
  - weft가 중계합니다: 보내는 쪽에서 512 KB씩 읽어 받는 쪽에 씁니다. 호스트끼리 접속하지 않고 원격에 아무것도 설치·저장하지 않습니다.
  - 받는 쪽엔 `이름.weft-part`로 쓰다가 끝나면 진짜 이름으로 바꿉니다(중간에 끊겨도 반쪽 파일이 남지 않음). 같은 이름이 있으면 덮어쓰지 않고 `이름 (1)`.
  - 오른쪽 아래 목록에 진행률·속도·파일 수, `×`로 취소(그때까지 쓴 부분은 지움).
- **Commands**: 현재 패널의 명령 블록 목록 (클릭 시 스크롤).
- **Search**: 파일 내용 검색. 모드 `text`(그대로) / `fuzzy`(글자 순서, `gst`→`git status`) / `glob`(`*` `?` `[ab]`) / `regex`(확장 정규식, `\(`는 괄호), `ignore case`, `hidden`(점으로 시작하는 파일·폴더도, 기본 끔). 원격은 `grep -E`(fuzzy·glob도 같은 뜻의 정규식으로 바꿔 보냄), 로컬은 Rust. 잘못된 정규식은 오류로 표시. 결과 클릭 = 해당 줄 열기, 우클릭 = 그 폴더로 cd·Files에서 보기·복사.
- **Procs**: 프로세스 표(PID, CPU%, MEM%, 사용자, 명령줄), 정렬/필터, 우클릭 TERM/KILL. 원격은 `ps`, 이 PC는 weft가 직접 읽습니다(CPU%는 작업 관리자처럼 전체 코어 기준; Windows에는 TERM이 없어 둘 다 종료).

## 콘솔 호스트 (Windows)

Windows에서 터미널은 콘솔 호스트 위에서 돕니다. weft는 **Windows에 들어 있는 것**(`conhost.exe`)을 씁니다. 다른 것을 쓰려면 직접 넣습니다(weft에는 들어 있지 않습니다):

- `conpty.dll`과 `OpenConsole.exe`를 `weft.exe` 옆에 두거나,
- `data/settings.json`에 `"console_host": "C:/폴더 또는 conpty.dll 경로"`.

설정 → 옆에 둔 파일 → Windows 내장 순으로 고르고, 시작할 때 한 번 정합니다. PATH에 있는 다른 프로그램의 `conpty.dll`은 쓰지 않습니다(전에는 WezTerm이 설치된 PC에서 그것을 썼습니다). 지금 무엇을 쓰는지는 팔레트 `About weft`, `Display test` 창, 로그의 `console host:` 줄에 나옵니다. 설정한 경로에 파일이 없으면 알림으로 알리고 Windows 내장을 씁니다. Linux와 macOS에는 해당 없습니다.

## 연결

`Ctrl+Shift+N` → `user@host [-p 2222 …]` 또는 `~/.ssh/config`의 Host 선택.

### 시리얼 콘솔 (UART)

팔레트 `Serial console…` → 호스트(이 PC 또는 원격), 포트, 속도(9600 … 3000000, 기본 115200; 8N1) → `Open`이면 그 포트가 터미널 탭으로 열립니다. 탭·분할·세션 복원·재시작 모두 다른 탭과 같습니다(설정에 `COM3`, `pi:/dev/ttyUSB0` 같은 이름의 호스트로 남음).
- **이 PC**: weft가 COM 포트를 직접 엽니다. 장치가 LF만 보내도 줄이 맞도록 CRLF로 보여 줍니다. 빼면 탭이 끝나고 Enter로 다시 엽니다.
- **원격**: 그 호스트의 ssh로 파이썬 표준 라이브러리만 쓰는 중계(`remote/serial.py`)를 명령에 실어 보내 돌립니다. 원격에 설치·저장하는 것은 없습니다. `Ctrl+]`로 끝. 권한이 없으면 `dialout` 그룹 안내.
- **누가 쓰고 있나**: 두 프로그램이 한 포트를 읽으면 데이터가 나뉘어 둘 다 깨집니다.
  - 원격: 목록에 그 포트를 연 프로세스(pid·사용자·명령, minicom 등의 잠금 파일 포함)가 보이고, 열 때 계속할지 묻습니다(다른 사용자의 프로세스는 root에게만 보임).
  - 이 PC: Windows는 한 프로그램만 열게 하므로 쓰고 있으면 열기가 거부되고 탭에 그렇게 나옵니다. weft 안의 다른 탭이 열고 있으면 그 탭을 알려 줍니다. (보드가 리셋될 수 있어 목록을 볼 때 포트를 미리 열어 보지 않습니다.)
- 장치 출력은 셸처럼 믿지 않습니다: 파일 열기·클립보드 같은 weft 요청(OSC)은 무시합니다.

- 패널: `ssh -tt host '<부트스트랩>'` (호스트 키 확인/비밀번호 입력도 패널에서 그대로)
- 에이전트: `ssh -T -o BatchMode=yes host '<부트스트랩>'` — 키/ssh-agent 인증이 필요합니다. 실패해도 터미널은 동작하고,
  첫 프롬프트가 뜬 뒤 한 번 자동 재시도합니다(상태바에서 수동 재연결 가능).
- 부트스트랩 문자열에는 `"`와 `\`가 없습니다 → sh/bash/zsh/fish 로그인 셸, MSVCRT `ssh.exe`, msys `ssh` 모두에서 같은 의미.
  rc는 gzip+base64로 보내 ConPTY의 약 8KB 명령줄 제한 안에 들어갑니다.
- `posix-loopback` 프로필: Git Bash를 원격처럼 사용(ssh와 완전히 같은 경로)해서 서버 없이 시험해 볼 수 있습니다.
- 로컬 패널은 PowerShell(+ PSReadLine 통합)이며 파일 작업은 에이전트 없이 Rust `std::fs`로 처리합니다.
- **Windows ssh 호스트**(셸이 cmd/PowerShell): weft의 셸 연동 명령은 POSIX 셸용이라 거기서 실패합니다. 그러면(ssh 자체 실패인 종료 코드 255가 아닐 때) 그 터미널을 평범한 `ssh -tt`로 다시 열고 프로필에 기억(`"plain": true`)해서 다음부터는 처음부터 그렇게 엽니다. 터미널은 정상으로 쓰지만 weft의 셸 연동(프롬프트 표시, 폴더 따라가기)은 없고, Files·에디터·검색은 sh와 Python이 필요해 그 호스트에서는 쓸 수 없다고 표시합니다.

### 그래픽 백엔드

팔레트 `Graphics backend…`: `auto`(기본: 그 OS에서 가장 안정적인 것) / `wgpu` / `OpenGL`. `settings.json`의 `"renderer"`에 저장되어 다음 시작부터 쓰입니다(`Save and restart`는 바로 다시 시작). 창에 지금 쓰는 백엔드와 GPU가 나오고, 로그에도 `graphics: …`로 남습니다.

| OS | auto | 고를 수 있는 것 |
|---|---|---|
| Windows | wgpu (DirectX 12) | OpenGL |
| macOS | wgpu (Metal) | OpenGL |
| Linux | OpenGL | wgpu (Vulkan) |

Windows에서 OpenGL로 그리면 절전·잠금을 몇 번 거친 뒤 다시 그릴 때마다 까만 프레임이 섞이는 경우가 있어 DirectX 12가 기본입니다. wgpu를 쓸 수 없는 PC(드라이버 없음)에서는 자동으로 OpenGL로 다시 시작하고 로그에 남깁니다. 한 번만 바꿔 보려면 환경 변수 `WEFT_RENDERER=opengl` / `wgpu`.
- **원격의 Python**: 에이전트와 시리얼 중계는 원격의 Python 3.6 이상으로 돕니다. 배포판마다 이름·위치가 달라서, 팔레트 `Python on remote hosts…`의 목록(한 줄에 하나: PATH의 이름이나 경로, `*` 가능)을 위에서부터 실행해 보고 3.6 이상인 첫 것을 씁니다. 기본: `python3`, `python`, `/usr/libexec/platform-python`(RHEL/Rocky 8 최소 설치), `/usr/local/bin/python3`, `/opt/homebrew/bin/python3`. 하나도 없으면 상태바에 시도한 목록과 할 일(원격에서 `command -v python3`로 확인, 또는 목록에 이름·경로 추가)이 나오고 `python…` 버튼으로 목록을 엽니다.

## 단축키

**모든 명령에 키를 지정할 수 있습니다**: 팔레트·☰ 메뉴·도움말의 "Keyboard shortcuts…". 명령마다 `+`를 누른 뒤 원하는 키를 누르면 추가(Esc 취소), 키를 누르면 제거, `Reset`은 기본값으로. 한 키가 두 명령에 걸리면 빨간색. 바꾼 것만 `settings.json`의 `"keys"`에 명령 id별로 저장됩니다(`"keys": {"new_tab": ["Ctrl+T"], "zoom": []}`, 빈 목록 = 키 없음). 지정된 키는 터미널로 가지 않습니다. 아래는 기본값:

| 키 | 동작 |
|---|---|
| `Ctrl+Shift+P` | 커맨드 팔레트. 기본은 **fuzzy**(글자 순서만 맞으면: `spldn` → Split down, 가장 잘 맞는 것이 위). 입력칸 옆 버튼을 누르면 `text`(그대로) / `glob`(`*`, `?`) / `regex`로 바뀌고 기억됩니다 |
| `Ctrl+Shift+T` / `Ctrl+Shift+N` | 새 탭(같은 호스트·cwd) / 연결 |
| `Ctrl+Shift+D` / `Ctrl+Shift+E` / `Ctrl+Shift+W` | 오른쪽 분할 / 아래 분할 / 패널 닫기 |
| `Ctrl+Shift+Enter` | **집중 모드**: 터미널(분할 포함)만 화면 가득. 사이드 패널·에디터·탭 줄·상태바를 숨기고, 다시 누르면(또는 오른쪽 위 `FOCUS` 클릭) 원래대로 |
| `Ctrl+Shift+Z` | 현재 패널만 탭 전체로 확대 / 원래 분할로 복귀 (패널 헤더 버튼, 우클릭 메뉴에도 있음) |
| `Alt+←↑→↓` | (분할 시) 패널 포커스 이동 |
| `Alt+Shift+←↑→↓` | (분할 시) 페인 경계를 그 방향으로 옮겨 크기 조절 (한 번에 5%) |
| `Alt+1` … `Alt+8` / `Alt+9` | 그 번호의 탭 / 마지막 탭 |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | 다음 / 이전 탭 (끝에서는 처음으로) |
| 탭 줄의 `‹ 3` / `5 ›` / `12 tabs` | 탭이 다 안 들어가 옆으로 넘어갈 때: 왼쪽·오른쪽에 가려진 탭 수와 전체 탭 수. 화살표를 누르면 가려진 다음 탭을 보여 줌(탭은 바뀌지 않음). 휠 한 칸에 탭 한 칸씩 움직이고, 스크롤바로 끌어도 탭 경계에서 멈춰 양 끝에 잘린 탭이 없음 |
| `Ctrl+Shift+↑/↓` | 이전/다음 명령 블록으로 스크롤 |
| 드래그 / 더블클릭 / 세 번 클릭 / `Alt+드래그` | 글자 / 단어 / 줄 / 사각형(열) 단위 선택 (더블·세 번 클릭 후 드래그하면 그 단위로 늘어남; 마우스를 쓰는 앱 안에서는 Shift와 함께) |
| `Ctrl+Shift+X` | **복사 모드**(WezTerm처럼): `hjkl` `w b e` `0 ^ $` `gg G` `H M L` `Ctrl+u/d/b/f`로 이동, `v` 글자 / `V` 줄 / `Ctrl+v` 사각형 선택, `o` 반대쪽 끝, `y`·`Enter` 복사(선택 없으면 그 줄), `Esc`·`q` 나가기. 스크롤백과 nvim 같은 전체 화면 앱 화면 모두 |
| `Ctrl+C` | 선택 영역 있으면 복사, 없으면 `^C` |
| `Ctrl+V` | 붙여넣기 (bracketed paste) |
| `Ctrl+= / Ctrl+-` | 글꼴 크기 |
| `Ctrl+S` | (에디터) 저장 |
| `Ctrl+F` / `Ctrl+H` | (에디터) 찾기 / 바꾸기: `text` / `fuzzy` / `glob` / `regex`(바꿀 말에 `$1`), 대소문자(`Aa`), 일치마다 배경·현재 위치는 진하게, 바꾸기는 `Ctrl+Z`로 되돌림, `Enter`·`F3` 다음, `Shift+Enter`·`Shift+F3` 이전, `Replace`(하나) / `All`(전부), `Esc` 닫기 |

## 디자인

- 색은 전부 `src/theme.rs`의 토큰(배경 4단계, 텍스트 3단계, 단일 액센트, success/warn/error)에서만 나옵니다.
- 테마는 고를 수 있습니다: **Catppuccin Mocha(기본) · Macchiato · Frappé · Latte**, **Graphite Dark · Light**. Catppuccin은 공식 색과 공식 터미널 ANSI 매핑을 그대로 씁니다(MIT, https://catppuccin.com).
  - 탭바 오른쪽 해/달 버튼 → 색 견본이 있는 테마 목록, 또는 커맨드 팔레트의 `Theme: …`
  - "Follow system": OS가 다크면 마지막에 고른 다크 테마, 라이트면 마지막에 고른 라이트 테마
  - `data/settings.json`: `"theme"`(테마 id 또는 `"system"`), `"theme_dark"`, `"theme_light"`
  - 모든 테마의 본문 글자 대비(6.5:1 이상)를 테스트로 확인합니다. Catppuccin Latte의 노란색은 원래 옅은 색이라 밝은 배경에서 약하게 보입니다.
- UI 아이콘은 `src/icons.rs`에서 선으로 직접 그립니다 → 폰트에 글리프가 없어 □로 깨지는 일이 없습니다.
- 폰트 폴백 체인: 터미널 `고른 폰트(기본 D2Coding Nerd Font) → 맑은 고딕 → 아이콘 폰트(Nerd Font) → Segoe UI Symbol → egui 기본`, UI `Segoe UI → 맑은 고딕 → 아이콘 폰트 → Segoe UI Symbol`.
  ASCII는 런 단위, 그 외 문자는 셀 단위로 그려 폴백 폰트가 열 정렬을 밀지 않습니다(한글 2칸 정렬).
  런을 그려 본 폭이 칸 수와 맞지 않으면(고정폭이 아닌 폰트, 글자를 잇는 폰트) 글자마다 자기 칸 가운데에 그립니다: 어떤 폰트를 골라도 글자가 격자를 벗어나지 않습니다.
- 상자 선 문자(`─ │ ┌ ┼ ━ ═ ║ ╔ ╭` 등 U+2500–257F의 가는 선·굵은 선·이중선·둥근 모서리)도 직접 그립니다: 칸 가장자리까지 이어서 그려 위아래 줄과 끊기지 않습니다(테두리, 표, 트리). 점선과 대각선은 폰트 글리프 그대로.
- 블록 문자(`█ ▀ ▄ ▌ ▐ ▛ ▜ ▁…▇ ░▒▓` 등 U+2580–259F)는 폰트 글리프가 아니라 **칸 크기의 사각형으로 직접 그립니다**(가장자리는 픽셀에 맞춤). 폰트의 블록이 줄 높이를 다 못 채워 배경이 띠처럼 비치는 일이 없고, 폰트마다 모양이 달라지지 않습니다.

## 테스트

무엇을 어떤 테스트가 잡는지, 종류별(단위 / 화면 / 픽셀 / 통합) 수와 커버리지, 자동으로 못 보는 것과 손으로 확인할 것은 [docs/regression.md](docs/regression.md)에 있습니다. 테스트를 추가하고 그 문서에 적지 않으면 테스트가 실패합니다.

눈으로 직접 확인하려면 도움말(F1)의 `Display test…`(팔레트·☰ 메뉴에도 있음): 블록 문자와 로고, 색과 속성, 열 정렬, 상자 선을 지금 고른 폰트로 실제 터미널과 같은 코드로 그린 네 화면이 뜨고, 화면마다 무엇을 봐야 하는지 적혀 있습니다.

```powershell
cargo test --workspace                                # 세 크레이트 전부(약 20초): 단위 테스트 + 실제 앱을 창 없이 돌리는 화면·픽셀 테스트
cargo test --workspace -- --ignored --test-threads=1  # 통합 테스트: 실제 셸과 에이전트 (Git Bash 필요, 약 2분)
python tools/testdoc.py                               # 종류별 테스트 수와 줄 커버리지를 재서 docs/regression.md에 기록
cargo test --workspace -- --include-ignored # + Git Bash를 원격 대역으로 쓰는 E2E (인메모리 rc, OSC 133/7/7777, stdin 에이전트)
```

## 라이선스

weft는 MIT 라이선스입니다([LICENSE](LICENSE)).

함께 들어 있는 것들은 각자의 라이선스를 따릅니다.

| 무엇 | 라이선스 | 파일 |
|---|---|---|
| D2Coding (Nerd Font 판) | SIL Open Font License 1.1 | `fonts/D2Coding-LICENSE.txt` |
| JetBrains Mono (Nerd Font 판) | SIL Open Font License 1.1 | `fonts/JetBrainsMono-OFL.txt` |
| Hack (Nerd Font 판) | MIT, Bitstream Vera | `fonts/Hack-LICENSE.md` |
| Nerd Fonts 아이콘 글리프 | 여러 라이선스(파일에 정리) | `fonts/NerdFonts-LICENSE` |
| Catppuccin 색 | MIT (https://catppuccin.com) | |

글꼴은 weft 실행 파일 안에도 들어 있습니다. 실행 파일을 나눠 줄 때는 위 라이선스 파일을 함께 두면 됩니다. Rust 라이브러리들은 각자의 라이선스(대부분 MIT 또는 Apache-2.0)를 따릅니다.

## 구조

셸 통합·OSC·원격 도구: [docs/shell-integration.md](docs/shell-integration.md)

```
shell/            셸 통합 (bash·zsh·fish·pwsh, OSC 133/7) — 압축 묶음으로 메모리로만 전달
remote/agent.py   python 표준 라이브러리 에이전트 (JSON lines) — stdin으로 전달
crates/weft-core/ 화면 없이 도는 부분 (GUI 의존성 없음)
  term.rs         VT/xterm 그리드, 절대 줄 번호 스크롤백, 블록, OSC 이벤트
  host.rs         프로필, 부트스트랩 명령 조립, 에이전트 연결, 로컬 백엔드(파일·프로세스)
  pty.rs          portable-pty 래퍼 (Windows 프로세스 트리 종료, 콘솔 호스트 닫기)
  find.rs editfind.rs records.rs secret.rs   찾기, 기록, 메모리 속 비밀번호
crates/weft-draw/ 그리기 공통 (egui): theme.rs fontfix.rs icons.rs highlight.rs md.rs keys.rs
src/              앱: 상태와 화면 (아래)
src/app.rs        상태/로직, 레이아웃, 요청/응답, 액션
src/ui_term.rs    그리드 렌더링 + 블록 크롬 + 입력 + 컴포저/검색
src/widgets.rs    인라인 위젯
src/ui_panels.rs  탭/상태바/사이드 패널/에디터/팔레트/연결/토스트
src/mermaid.rs    다이어그램 렌더링(내장/mmdc)/캐시   src/media.rs  이미지·PDF 보기
src/fileops.rs confirm.rs transfer.rs   파일 작업, 닫기 전에 묻기, 파일 전송
src/frame.rs      한 프레임 (창과 테스트가 같은 것을 돌림)   src/testkit.rs regress.rs  화면 테스트
```
