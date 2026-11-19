# 셸 통합, OSC, 원격에서 쓰는 것

weft가 셸에서 "프롬프트가 어디고, 명령이 언제 시작해서 끝났고(종료 코드), 지금 디렉터리가 어디인지"를
어떻게 아는지, 원격에 무엇을 어떻게 보내는지, 어떤 터미널 시퀀스(OSC)를 처리하는지를 정리합니다.

## 1. 세 가지 방법

| 방법 | 알 수 있는 것 | 못 하는 것 | 쓰는 곳 |
|---|---|---|---|
| **셸 통합** (셸의 프롬프트 훅이 OSC 133/7을 출력) | 프롬프트·입력·명령 시작/끝, **종료 코드**, cwd | 셸마다 몇 줄씩 필요 | bash, zsh, fish, PowerShell, nushell, cmd(종료 코드 제외) |
| **프로세스 관찰** (셸 PID의 `/proc/<pid>/cwd`, `lsof`) | cwd | 종료 코드, 블록 경계 | 통합이 없는 원격 셸의 cwd |
| **추정** (출력 모양) | 프롬프트, 명령 시작/끝 | 종료 코드, 틀릴 수 있음 | 통합이 없는 모든 셸 (dash, ksh, tcsh, 처음 보는 셸) |

셸이 OSC 133을 한 번이라도 보내면 추정은 멈추고 통합을 따릅니다.

## 2. 셸별 지원

| 셸 | 로컬 | 원격 | 통합 방법 | 종료 코드 | cwd | 실제 셸로 테스트 |
|---|---|---|---|---|---|---|
| bash | ✓ (macOS·Linux) | ✓ | `--rcfile <(…)` (`shell/bash.sh`) | ✓ | OSC 7 | ✓ |
| zsh | ✓ (macOS·Linux) | ✓ | `precmd`/`preexec` 훅 (`shell/zsh.zsh`), 아래 "zsh" 참고 | ✓ | OSC 7 | ✗ (설치돼 있지 않음, 스크립트만 단위 테스트) |
| fish | ✓ (macOS·Linux) | ✓ | `fish_prompt` 감싸기 + `fish_preexec` (`shell/fish.fish`), `fish -C` | ✓ | OSC 7 | ✗ (상동) |
| PowerShell 7 / 5.1 | ✓ | ✓ | `prompt` 함수 + PSReadLine Enter (`shell/pwsh.ps1`) | ✓ | OSC 7 | ✓ (로컬·원격 방식 모두) |
| nushell | ✓ | ✓ | nu 내장 통합(`shell_integration.osc133/osc7`)을 `-e`로 켬 | ✓ | OSC 7 (첫 명령부터) | ✓ |
| cmd | ✓ | – | `PROMPT` 변수에 표시를 넣음 | ✗ (cmd가 알려 주지 않음) | OSC 9;9 | ✓ |
| 그 외 (dash, ksh, tcsh, xonsh …) | – | ✓ | 없음: 추정 | ✗ | 프로세스 관찰 (python 에이전트 필요) | ✓ (dash) |

- 로컬 셸은 Windows에서 `settings.json`의 `"local_shell"`(`auto`/`pwsh`/`powershell`/`cmd`/`nu`) 또는 팔레트 `Local shell: …`로 고릅니다.
- 원격에 통합을 풀 도구(`base64`나 `openssl`, `gzip`, `awk`, 또는 `python3`)가 하나도 없으면 터미널에 그 이유를 한 줄 적고 사용자 셸을 통합 없이 그대로 띄웁니다. 에이전트는 이유를 알리고 끝납니다.
- macOS의 `/usr/bin/python3`는 Command Line Tools가 없으면 Python이 아니라 설치 창을 띄우는 대행 프로그램이라 실행하지 않습니다(`xcode-select -p`로 확인). "Python 없음" 안내에 `xcode-select --install`이 함께 나옵니다.
- macOS·Linux의 로컬 터미널은 사용자 셸(`$SHELL`)을 원격과 같은 시작 명령으로(ssh 없이 `/bin/sh -c '…'`) 띄워, 원격과 같은 통합이 붙습니다.
- 원격은 **로그인 셸(`$SHELL`)**을 그대로 띄웁니다. 시험이나 강제를 위해 `WEFT_SHELL` 환경 변수로 바꿀 수 있습니다.
- bash는 `--rcfile`이 bash 자신의 시작 파일을 대신하므로, weft의 rc가 로그인 셸이 읽는 파일을 bash와 같은 순서로 읽습니다: `/etc/profile`(과 그것이 부르는 `/etc/profile.d/*.sh`), 이어서 `~/.bash_profile`·`~/.bash_login`·`~/.profile` 중 처음 있는 것 하나, 끝날 때 `~/.bash_logout`. 그냥 ssh로 로그인한 것과 같은 환경이 됩니다.

### zsh

zsh는 시작 코드를 파일(`$ZDOTDIR/.zshrc`)로만 받고, 원격 디스크에는 아무것도 쓰지 않는 원칙 때문에 파일을 만들 수 없습니다.
그래서 통합 코드를 환경 변수 `WEFT_ZSH`로 넘기고, 첫 프롬프트에서 weft가 ` eval "$WEFT_ZSH"`를 대신 입력합니다.
- 앞에 공백을 두고 `HIST_IGNORE_SPACE`를 켜서 기록에 남지 않습니다.
- 입력한 줄은 통합 코드가 바로 지우고, 그 자리에 새 프롬프트가 그려집니다.
- 이 "대신 입력"(OSC 7777 `inject`)은 패널마다 한 번, 셸 통합이 시작되기 전에만 받습니다.

### cmd

cmd에는 훅이 없지만 `PROMPT`에 이스케이프(`$E`)를 넣을 수 있습니다. weft가 설정하는 `PROMPT`는
`이전 명령 끝(133;D, 코드 없음) → cwd(OSC 9;9) → 프롬프트 시작(133;A) → 원래 프롬프트 → 입력 시작(133;B)`입니다.
명령 시작 신호가 없어서 Enter를 누른 시점을 시작으로 씁니다.

## 3. 원격에 보내는 것 (디스크에 저장하지 않음)

```
패널:      ssh -tt host 'exec sh -c '\''W=<묶음>; WEFT_CWD=…; B=<풀기 스크립트>; eval …'\'''
            └ 풀기 스크립트: base64 해독기·gunzip 찾기(POSIX 도구, 없으면 python) → 묶음 해제 → boot 구간 실행
               └ boot: 셸/PID 알림(OSC 7777 shell) → 로그인 셸에 맞는 통합으로 exec
에이전트:  ssh -T host '…python3 -c "exec(stdin)"' ← agent.py를 stdin으로 (python 표준 라이브러리만)
```

- **통합 묶음**: `shell/` 안의 boot 스크립트와 bash/zsh/fish/pwsh 통합을 `#@이름` 구간으로 이어 붙이고,
  주석·빈 줄을 뺀 뒤 gzip + base64로 명령줄에 실어 보냅니다. Windows ConPTY가 명령줄을 약 8KB에서 자르기 때문입니다(현재 약 5.6KB, 테스트로 확인).
- 셸 코드는 프로세스 치환(`<(…)`), 환경 변수, 명령줄 인자로만 전달되고, 원격 파일로 쓰지 않습니다.
- 부트스트랩 문자열에는 `"`와 `\`가 없습니다. sh/bash/zsh/fish 로그인 셸, MSVCRT `ssh.exe`, msys `ssh`가 모두 같은 의미로 읽습니다.

### 원격에서 쓰는 도구

| 무엇 | 필요한 것 | 없으면 |
|---|---|---|
| 셸 통합 | 그 셸 + `base64`(또는 `base64 -D`, 없으면 python) + `gzip`(없으면 python) + `awk` | python도 없으면 통합 없이 셸만 뜸 → 추정 |
| 에이전트 (파일 트리·에디터·검색·Procs·`tree`·`cwd`) | python 3 표준 라이브러리 | 원격 파일 기능과 프로세스 관찰이 빠짐. 터미널은 동작 |
| bash 헬퍼 `wls`, `wtable` | python | `ls -la`, `cat`으로 대신 |

에이전트 작업: `hello` `list` `stat` `read` `write`(mtime 충돌 감지) `complete` `procs` `kill` `sys` `grep`(POSIX `grep -rnI`) `tree`(파일 필터용 이름 목록) `cwd`(프로세스 디렉터리).

### `w*` 헬퍼

`wopen` `wnotify` `wclip` `wchoose` `wprogress` `wls` `wtable`은 **bash 통합에만** 있는 함수입니다.
weft로 연 bash에서만 쓸 수 있고, 다른 셸이나 스크립트 안에서는 아래의 표준 OSC를 직접 출력하면 됩니다.
셸마다 헬퍼를 만들지 않는 대신, 같은 기능을 weft 쪽(로컬)으로 옮기는 것을 검토 중입니다
(예: 명령 블록 출력을 표로 보기).

## 4. weft가 처리하는 OSC

터미널 명령은 프로그램 **출력** 속에 섞인 `ESC ] 번호 ; 인자 … BEL(또는 ESC \)` 바이트입니다.
누가 출력했는지 구별할 수 없으므로(셸, `cat`한 파일, 원격 프로그램 모두 같은 흐름), weft는 다른 터미널과 같은 표준대로 받습니다.

| OSC | 하는 일 | 보내는 쪽 |
|---|---|---|
| `0`, `2` | 창/탭 제목 | 셸, vim, tmux … |
| `7` | 현재 디렉터리 `file://호스트/경로` | 셸 통합 |
| `9;9;경로` | 현재 디렉터리 (Windows 터미널 관례) | cmd 통합, oh-my-posh |
| `9;메시지`, `777;notify;…` | 알림 토스트 (`9;4` 진행률은 무시) | 아무 프로그램 |
| `52;c;base64` | 로컬 클립보드에 쓰기 | `wclip`, neovim(ssh), tmux |
| `52;c;?` | 클립보드 **읽기** 요청 → **응답하지 않음** (출력만으로 클립보드를 빼 가는 것을 막음) | – |
| `133;A/B/C/D[;코드]` | 프롬프트 시작 / 입력 시작 / 명령 시작 / 명령 끝(종료 코드, 없으면 모름) | 셸 통합 |
| `7777;base64(JSON)` 또는 `7777;{JSON}` | weft 전용 이벤트: `open`(파일 열기), `notify`, `widget`(files/table/choose/progress), `inject`(zsh 통합 입력, 1회), `shell`(셸 이름·PID) | bash 헬퍼, 부트스트랩 |

### 검토했다가 뺀 것: OSC 가드(비밀 토큰)

`cat`한 파일 속 OSC가 클립보드나 cwd를 바꾸는 것을 막으려고, 패널별 비밀 토큰이 붙은 OSC만 받는 방식을 만들었다가
되돌렸습니다(`09f1939` → `0c82d12`). neovim·tmux 같은 프로그램이 출력 속 OSC(특히 52 클립보드)로 터미널과 통신하므로
호환이 깨지기 때문입니다. 대신 위험이 가장 큰 클립보드 읽기만 응답하지 않습니다.

## 5. 추정 규칙

통합이 없는(OSC 133을 보내지 않은) 셸에서:

1. **프롬프트**: 출력이 250ms 동안 멈췄고 커서가 그 줄의 글자 뒤에 있으며(커서 뒤는 비어 있음) 직전 블록이 끝난 상태 →
   그 줄을 프롬프트로, 커서 위치를 입력 시작으로 봅니다. 줄의 마지막 글자(`$`, `>`, `#`, `%`, `❯` …)를 기억합니다.
2. **명령 시작**: 그 프롬프트에서 Enter를 보낸 순간. 입력 시작부터 커서까지가 명령 문자열입니다.
3. **명령 끝**: 출력이 400ms 동안 멈췄고, 커서가 같은 마지막 글자로 끝나는 줄의 글자 뒤에 있을 때.
   명령 출력은 보통 줄바꿈으로 끝나 커서가 0열에 있으므로 프롬프트와 구별됩니다.
   `[sudo] password:`나 python의 `>>> `처럼 끝 글자가 다른 대기 상태에서는 끝내지 않습니다.
4. 전체 화면 프로그램(vim 등, 대체 화면) 중에는 판단하지 않습니다.
5. 추정한 블록에는 종료 코드가 없어 중립 색으로 표시합니다. 끝날 때 셸 PID를 알면 에이전트로 cwd를 다시 읽습니다.

**한계**: 프롬프트가 출력 도중 잠깐 멈춘 줄과 같은 글자로 끝나면 일찍 끝난 것으로 볼 수 있습니다.
여러 줄 프롬프트는 마지막 줄만 프롬프트로 봅니다.

## 6. 테스트

`cargo test -- --include-ignored`가 실제 셸로 E2E를 돌립니다(이 PC 기준).

| 테스트 | 셸 |
|---|---|
| `login_shell_*` (Git Bash가 원격 대역, `WEFT_SHELL`로 셸 선택) | nushell, pwsh 7, Windows PowerShell 5.1, dash(추정) |
| `shell_integration_over_in_memory_rc` | bash (rc, 헬퍼, 종료 코드) |
| `local_*` | cmd, nushell, Windows PowerShell |
| `powershell_block_time_is_the_command_only`, `powershell_cursor_follows_typing` | 로컬 pwsh (starship 프로필) |
| `unknown_shell_blocks_are_guessed` | dash 직접 실행 |

zsh·fish는 이 PC에 없어 통합 스크립트의 구성만 단위 테스트합니다. 에이전트의 `cwd`(`/proc`, `lsof`)는 Linux·macOS에서만 동작해 이 PC에서는 테스트하지 못했습니다.
