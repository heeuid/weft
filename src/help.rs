//! Help window text: keys and mouse beyond the command list, and what each
//! feature does. Kept short; README.md has the long form.

/// Keys and mouse actions that are not commands (those are listed, with the
/// keys bound to them, from ui_panels::COMMANDS).
pub const KEYS: &[(&str, &str)] = &[
    ("Ctrl+C", "선택한 글자 복사 (선택이 없으면 ^C로 중단)"),
    ("Ctrl+V", "붙여넣기 (bracketed paste)"),
    ("Ctrl+S", "에디터: 파일 저장"),
    ("Ctrl+F / Ctrl+H", "에디터: 찾기 / 바꾸기 (text·fuzzy·glob·regex, Aa 대소문자, Enter·F3 다음, Shift+Enter·Shift+F3 이전, Replace 하나 / All 전부, Esc 닫기)"),
    ("Ctrl+클릭", "경로(src/main.rs:12 포함)는 에디터로, URL은 브라우저로"),
    ("드래그", "글자 단위로 선택"),
    ("더블클릭 (+드래그)", "단어 단위로 선택"),
    ("세 번 클릭 (+드래그)", "줄 단위로 선택"),
    ("복사 모드 (Ctrl+Shift+X)", "키보드로 선택: hjkl·w b e·0 ^ $·gg G·H M L·Ctrl+u/d/b/f로 이동, v 글자·V 줄·Ctrl+v 사각형, o 반대쪽 끝, y/Enter 복사, Esc/q 나가기"),
    ("Alt+드래그", "사각형(열) 선택: 각 줄의 같은 칸들, 복사하면 줄마다 한 줄"),
    ("Shift+드래그, Shift+우클릭", "마우스를 쓰는 앱(nvim, htop…) 안에서 weft 선택 / weft 메뉴"),
    ("Ctrl+휠", "글자 크기"),
    ("Ctrl+Shift+휠", "화면 배율 (아이콘·탭·패널까지)"),
    ("탭 가운데 클릭", "탭 닫기 (우클릭: 다른 탭·오른쪽·왼쪽·모두 닫기)"),
    ("패널 손잡이 ⋮⋮ 끌기", "패널을 다른 패널 옆이나 창 가장자리로 옮기기"),
];

/// (title, text) for the Features tab.
pub const FEATURES: &[(&str, &str)] = &[
    (
        "명령 블록",
        "셸이 명령의 시작과 끝을 알려 주면(OSC 133) 명령마다 블록이 됩니다. 왼쪽 막대 색이 상태(실행 중 / 성공 / 실패), \
         오른쪽 배지가 종료 코드와 걸린 시간입니다. 블록에 마우스를 올리면 명령 복사, 출력 복사, 다시 실행, 컴포저로 편집, \
         접기, 에디터로 열기 버튼이 나옵니다. 알려 주지 않는 셸은 출력이 멈춘 것을 보고 블록을 추정합니다(종료 코드 없음).",
    ),
    (
        "컴포저 (Ctrl+Space)",
        "프롬프트 자리에 뜨는 여러 줄 입력창. Tab = 경로 완성, ↑↓ = 앞부분이 같은 히스토리, → = 제안 받기, \
         Enter = 실행, Shift+Enter = 줄바꿈, Esc = 닫기.",
    ),
    (
        "찾기: 스크롤백, History, 파일, grep",
        "Ctrl+Shift+F = 지금 페인의 스크롤백에서 찾기. History 탭(Ctrl+Shift+H) = 모든 호스트·날짜의 명령 기록에서 \
         명령(fuzzy / glob / regex)과 출력을 찾기. Files 탭의 필터 = 루트 아래 파일 이름. Search 탭 = 파일 내용(grep).",
    ),
    (
        "사이드 패널 (Ctrl+Shift+B)",
        "Files: 터미널이 있는 폴더를 따라가는 트리. 더블클릭 = 파일 열기 / 폴더로 cd, ↵ 버튼 = 보고 있는 폴더로 cd. \
         Commands: 지금 페인의 명령 목록. History: 저장된 명령 기록. Search: 내용 검색. Procs: 프로세스 표(정렬, TERM/KILL).",
    ),
    (
        "파일 전송 (로컬 ↔ 원격, 원격 ↔ 원격)",
        "Files의 파일·폴더를 끌어 다른 사이드 패널의 Files, 폴더, 터미널(그 셸의 폴더) 위에 놓으면 복사합니다. 우클릭 Download = \
         로컬 Downloads로, Send to = 다른 패널·터미널의 폴더로. 탐색기에서 끌어 놓아도 됩니다. 사이드 패널을 나눠 머리의 호스트 이름(▾)에서 \
         다른 호스트를 고르면 양쪽 창이 됩니다. weft가 512 KB씩 중계하고, 받는 쪽엔 .weft-part로 쓰다 끝나면 이름을 바꿉니다. 같은 이름이 \
         있으면 '이름 (1)'. 오른쪽 아래 목록에서 진행률과 취소.",
    ),
    (
        "에디터 (Ctrl+Shift+J)",
        "로컬·원격 파일을 탭으로 엽니다. 원격 파일은 에이전트가 읽고 저장할 때만 쓰며, 그사이 바뀌었으면 알려 줍니다. \
         구문 강조(약 200개 언어), 이미지와 PDF 보기. 마크다운은 편집창이 곧 렌더링된 문서이고(Rich / Source), \
         ```mermaid 블록은 그림으로 보입니다.",
    ),
    (
        "메모 (Ctrl+Shift+M)",
        "에디터 탭 줄의 + 로 새 메모, 그 옆 Memos(또는 팔레트 Open memo…)로 목록에서 찾아 열기·삭제. data/memos 아래 마크다운 파일이고 따로 저장할 필요 없이 바로 저장됩니다. \
         탭 이름은 첫 줄, 빈 채로 닫으면 지워집니다.",
    ),
    (
        "원격 호스트 (Ctrl+Shift+N)",
        "user@host 또는 ~/.ssh/config의 Host. 터미널은 ssh 그대로이고, 파일 패널·에디터·grep·프로세스 표는 ssh 너머 \
         python 에이전트가 처리합니다. 원격 디스크에는 아무것도 남기지 않습니다(셸 통합과 에이전트는 메모리에서만 실행). \
         키 인증과 ssh-agent를 권장합니다: 그러면 weft가 비밀번호를 가질 일이 없습니다.",
    ),
    (
        "원격 헬퍼",
        "wls [DIR] = 정렬되는 파일 표, CMD | wtable = 표로 보기, wchoose a b c = 선택 목록, wprogress = 진행 막대, \
         wopen FILE [LINE] = 로컬 에디터로 열기, wnotify MSG = 알림, wclip = 이 컴퓨터 클립보드로 복사.",
    ),
    (
        "마우스를 쓰는 앱",
        "nvim(mouse=a), vim, htop, tmux처럼 마우스를 요청한 앱에서는 드래그·우클릭이 앱으로 갑니다. \
         Shift를 누르면 weft 선택 / weft 메뉴입니다. 다른 곳에서는 Shift가 있든 없든 weft 것입니다. \
         ssh 너머 Neovim 0.10+에서 \"+y 하면 이 컴퓨터 클립보드로 복사됩니다(OSC 52).",
    ),
    (
        "레이아웃과 모양",
        "패널 손잡이(⋮⋮)를 끌어 어디로든 옮기고, 경계선을 끌어 크기를 바꿉니다. 사이드 패널과 에디터는 손잡이 옆 \
         분할 버튼으로 여러 개로 나눕니다(클릭 = 아래, 우클릭 = 오른쪽; 사이드 패널은 각자 다른 탭, 에디터는 각자 다른 파일). \
         터미널 페인은 헤더를 끌어 같은 탭의 다른 페인 가장자리(그쪽으로 분할)나 가운데(자리 바꾸기)에 놓습니다. \
         탭 이동: Alt+1…8 / Alt+9(마지막), Alt+Shift+←→. 탭 줄의 Aa = 터미널 폰트와 아이콘 폰트 \
         (내장 Nerd Font, data/fonts, 설치된 폰트), 해/달 = 테마. 상태바의 A− A+ = 글자 크기, − % + = 화면 배율.",
    ),
    (
        "기록과 비밀번호 (Privacy…)",
        "끝난 명령과 출력, 명령 히스토리, weft 로그, 에이전트 로그를 항목마다 켜고 끄고, 보관 기간과 최대 크기를 정합니다. \
         키 입력은 어디에도 쓰지 않고, 앞에 공백을 붙인 명령은 기록에서 빠집니다. ssh 비밀번호는 디스크에 쓰지 않고 \
         메모리에서 암호화해 두며(키는 주기적으로 교체), ssh에는 한 번 쓰고 사라지는 표로 넘깁니다.",
    ),
    (
        "셸",
        "로컬 셸은 PowerShell 7 / Windows PowerShell / cmd / nushell 중에서 고릅니다(팔레트 Local shell). \
         bash, zsh, fish, PowerShell은 셸 통합으로 명령 블록과 현재 폴더를 정확히 알고, 그 밖의 셸은 추정합니다.",
    ),
];
