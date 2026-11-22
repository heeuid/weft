//! Help window text: keys and mouse beyond the command list, and what each
//! feature does, in Korean and in English (the system's language picks one
//! at first; the window switches). Kept short; README.md has the long form.

/// Whether help is shown in Korean: as set in the window ("ko", "en"),
/// else by the system's language.
pub fn korean(setting: &str) -> bool {
    match setting {
        "ko" => true,
        "en" => false,
        _ => system_korean(),
    }
}

/// The system's language is Korean (Windows', macOS' settings; LANG and
/// the like on Linux), asked once.
fn system_korean() -> bool {
    static KO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *KO.get_or_init(|| sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("ko")))
}

/// Keys and mouse actions that are not commands (those are listed, with the
/// keys bound to them, from ui_panels::COMMANDS).
pub fn keys(ko: bool) -> &'static [(&'static str, &'static str)] {
    if ko { KEYS_KO } else { KEYS_EN }
}

/// (title, text) for the Features tab.
pub fn features(ko: bool) -> &'static [(&'static str, &'static str)] {
    if ko { FEATURES_KO } else { FEATURES_EN }
}

const KEYS_KO: &[(&str, &str)] = &[
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

const KEYS_EN: &[(&str, &str)] = &[
    ("Ctrl+C", "copy the selected text (with nothing selected, ^C: interrupt)"),
    ("Ctrl+V", "paste (bracketed paste)"),
    ("Ctrl+S", "editor: save the file"),
    ("Ctrl+F / Ctrl+H", "editor: find / replace (text·fuzzy·glob·regex, Aa for case, Enter·F3 next, Shift+Enter·Shift+F3 previous, Replace one / All, Esc closes)"),
    ("Ctrl+click", "a path (src/main.rs:12 too) opens in the editor, a URL in the browser"),
    ("drag", "select by characters"),
    ("double-click (+drag)", "select by words"),
    ("triple-click (+drag)", "select by lines"),
    ("copy mode (Ctrl+Shift+X)", "select with the keyboard: move with hjkl·w b e·0 ^ $·gg G·H M L·Ctrl+u/d/b/f, v characters·V lines·Ctrl+v block, o the other end, y/Enter copies, Esc/q leaves"),
    ("Alt+drag", "block (column) selection: the same cells of each line, copied as a line each"),
    ("Shift+drag, Shift+right-click", "weft's selection / weft's menu inside an app that uses the mouse (nvim, htop…)"),
    ("Ctrl+wheel", "text size"),
    ("Ctrl+Shift+wheel", "zoom (icons, tabs and panels too)"),
    ("middle-click a tab", "close it (right-click: close the others, to the right, to the left, all)"),
    ("drag a panel's ⋮⋮ handle", "move the panel beside another or to the window's edge"),
];

const FEATURES_KO: &[(&str, &str)] = &[
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
         Commands: 지금 페인의 명령 목록. History: 저장된 명령 기록. Search: 내용 검색. Procs: 프로세스 표(정렬, TERM/KILL; 원격과 이 PC 모두).",
    ),
    (
        "파일 만들기, 이름 바꾸기, 지우기",
        "Files의 행을 우클릭: New folder / New file(폴더에서는 그 안에, 파일에서는 그 옆에), Rename, Delete. 머리의 + = 보고 있는 폴더에 만들기. \
         로컬과 원격 모두 되고, 같은 이름이 있으면 덮어쓰지 않습니다. 지우기는 먼저 묻고, 휴지통으로 보낼지 완전히 지울지 고릅니다 \
         (이 PC, 원격 Linux·macOS: 파일 관리자에서 되살릴 수 있는 그 시스템의 휴지통). 원격 Windows는 휴지통 없이, 되돌릴 수 없다고 알린 뒤 지웁니다.",
    ),
    (
        "닫기 전에 묻기, 다시 열기, 버전",
        "저장 안 한 파일이 있는 탭을 닫거나 명령이 아직 도는 터미널을 닫을 때(창 닫기·재시작 포함) 먼저 묻습니다. 묻는 창의 체크 상자나 \
         팔레트 'Ask before…'로 끄고 켭니다. 닫을 때 열려 있던 터미널, 에디터 파일(Rich/Source, Read), 펼친 폴더는 다음에 그대로 열립니다. \
         버전은 이 창 위쪽, 팔레트 'About weft', 명령줄 weft --version 으로 봅니다. 그 옆의 Display test = 블록 문자·색·열 정렬·상자 선을 \
         지금 폰트로 그린 시험 화면(무엇을 봐야 하는지 적혀 있음).",
    ),
    (
        "파일 전송 (로컬 ↔ 원격, 원격 ↔ 원격)",
        "Files의 파일·폴더를 끌어 다른 사이드 패널의 Files, 폴더, 터미널(그 셸의 폴더) 위에 놓으면 복사합니다. 우클릭 Download = \
         이 PC의 다운로드 폴더(시스템이 정한 곳)로, Send to = 다른 패널·터미널의 폴더로. 탐색기에서 끌어 놓아도 됩니다. 사이드 패널을 나눠 머리의 호스트 이름(▾)에서 \
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
         키 인증과 ssh-agent를 권장합니다: 그러면 weft가 비밀번호를 가질 일이 없습니다. 처음 보는 호스트의 키는 weft 창에서 \
         지문을 보여 주고 믿을지 묻습니다.",
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
         탭 이동: Alt+1…8 / Alt+9(마지막), Ctrl+Tab / Ctrl+Shift+Tab. 분할된 페인 크기: Alt+Shift+←↑→↓. 탭 줄의 Aa = 터미널 폰트와 아이콘 폰트 \
         (내장 Nerd Font, data/fonts, 설치된 폰트), 해/달 = 테마. 상태바의 A− A+ = 글자 크기, − % + = 화면 배율. \
         폰트에 없는 글자(다른 나라 문자 등)는 이 컴퓨터에 설치된 폰트에서 찾아 보여 줍니다.",
    ),
    (
        "기록과 비밀번호 (Privacy…)",
        "끝난 명령과 출력, 명령 히스토리, weft 로그, 에이전트 로그를 항목마다 켜고 끄고, 보관 기간과 최대 크기를 정합니다. \
         키 입력은 어디에도 쓰지 않고, 앞에 공백을 붙인 명령은 기록에서 빠집니다. ssh 비밀번호는 디스크에 쓰지 않고 \
         메모리에서 암호화해 두며(키는 주기적으로 교체), ssh에는 한 번 쓰고 사라지는 표로 넘깁니다.",
    ),
    (
        "셸",
        "로컬 셸은 Windows에서 PowerShell 7 / Windows PowerShell / cmd / nushell, macOS·Linux에서 로그인 셸이나 고른 셸입니다(팔레트 Shells…). \
         bash, zsh, fish, PowerShell은 셸 통합으로 명령 블록과 현재 폴더를 정확히 알고, 편집 모드(vi·emacs)와 상관없이 weft의 실행이 줄을 비웁니다. \
         그 밖의 셸은 추정합니다.",
    ),
];

const FEATURES_EN: &[(&str, &str)] = &[
    (
        "Command blocks",
        "Where the shell says where each command starts and ends (OSC 133), each command is a block. The bar on the left is its state \
         (running / succeeded / failed), the badge on the right its exit code and how long it took. Over a block: copy the command, copy its \
         output, run it again, edit it in the composer, fold it, open it in the editor. A shell that says nothing gets blocks guessed from where \
         its output stops (no exit code).",
    ),
    (
        "Composer (Ctrl+Space)",
        "A many-line input where the prompt is. Tab = complete a path, ↑↓ = history starting the same, → = take the suggestion, \
         Enter = run, Shift+Enter = a new line, Esc = close.",
    ),
    (
        "Finding: scrollback, History, files, grep",
        "Ctrl+Shift+F = find in this pane's scrollback. The History tab (Ctrl+Shift+H) = find commands (fuzzy / glob / regex) and their output \
         in the record of every host and day. The Files tab's filter = file names under its root. The Search tab = file contents (grep).",
    ),
    (
        "Side panel (Ctrl+Shift+B)",
        "Files: a tree that follows the terminal's folder. Double-click = open a file / cd to a folder, the ↵ button = cd to the folder shown. \
         Commands: this pane's commands. History: the commands kept. Search: contents. Procs: a table of processes (sorted, TERM/KILL; remote \
         and on this PC).",
    ),
    (
        "Making, renaming and deleting files",
        "Right-click a row in Files: New folder / New file (inside a folder, beside a file), Rename, Delete. The + at the top makes one in the \
         folder shown. Local and remote alike; a name taken is never overwritten. Delete asks first: to the trash or for good (this PC, and \
         remote Linux and macOS: that system's own trash, which its file manager puts back from). On a remote Windows there is no trash: it \
         says it cannot be undone, then deletes.",
    ),
    (
        "Asking before closing, opening again, the version",
        "Closing a tab with a file not saved, or a terminal with a command still running (the window or a restart too), asks first; the box in \
         that question or the palette's 'Ask before…' turns it off and on. The terminals, editor files (Rich/Source, Read) and open folders there \
         at closing are there again next time. The version is at the top of this window, in the palette's 'About weft' and from weft --version. \
         Beside it, Display test = test screens of block characters, colours, column alignment and box lines in the font in use (with what to \
         look for).",
    ),
    (
        "Copying files (local ↔ remote, remote ↔ remote)",
        "Drag a file or folder in Files onto another side panel's Files, a folder or a terminal (its shell's folder) to copy it there. Right-click \
         Download = to this PC's downloads folder (where the system keeps it), Send to = another panel's or terminal's folder. Dragging in from \
         the file manager works too. Split the side panel and pick another host at its top (▾) for two sides. weft carries it 512 KB at a time, \
         written as .weft-part and renamed when whole; a name taken becomes 'name (1)'. Progress and cancel at the bottom right.",
    ),
    (
        "Editor (Ctrl+Shift+J)",
        "Local and remote files in tabs. A remote file is read by the agent and written only when saved; changed there meanwhile, it says so. \
         Syntax colours (about 200 languages), images and PDFs shown. Markdown is edited as the rendered document (Rich / Source), and \
         ```mermaid blocks show as diagrams.",
    ),
    (
        "Memos (Ctrl+Shift+M)",
        "The + on the editor's tab row makes a memo; Memos beside it (or the palette's Open memo…) finds, opens and deletes them. They are \
         Markdown files under data/memos, saved as they are typed. A memo's tab is named by its first line; closed empty, it is deleted.",
    ),
    (
        "Remote hosts (Ctrl+Shift+N)",
        "user@host or a Host of ~/.ssh/config. The terminal is ssh as it is; the file panel, editor, grep and process table are served over \
         ssh by a Python agent. Nothing is left on the remote disk (the shell integration and the agent run in memory only). Keys and \
         ssh-agent are best: weft then never holds a password. A host's key not seen before is shown in weft's window with its fingerprint, \
         to trust or not.",
    ),
    (
        "Remote helpers",
        "wls [DIR] = a sortable table of files, CMD | wtable = a command's columns as a table, wchoose a b c = a list to choose from, \
         wprogress = a progress bar, wopen FILE [LINE] = open in the local editor, wnotify MSG = a notification, wclip = copy to this \
         computer's clipboard.",
    ),
    (
        "Apps that use the mouse",
        "In an app that asks for the mouse (nvim with mouse=a, vim, htop, tmux), drags and right-clicks go to the app; with Shift they are \
         weft's selection / weft's menu. Elsewhere they are weft's, Shift or not. In Neovim 0.10+ over ssh, \"+y copies to this computer's \
         clipboard (OSC 52).",
    ),
    (
        "Layout and looks",
        "Drag a panel's handle (⋮⋮) anywhere, and its edges to size it. The side panel and the editor split with the button beside the handle \
         (click = below, right-click = to the right; each side panel its own tab, each editor its own file). Drag a terminal pane's header to \
         another pane's edge (split that way) or middle (swap). Tabs: Alt+1…8 / Alt+9 (the last), Ctrl+Tab / Ctrl+Shift+Tab. Split panes' size: \
         Alt+Shift+←↑→↓. Aa on the tab row = the terminal's font and the icon font (built-in Nerd Font, data/fonts, installed fonts), sun/moon = \
         the theme. In the status bar, A− A+ = text size, − % + = zoom. A character the fonts lack (another script) is drawn with a font \
         installed on this computer that has it.",
    ),
    (
        "Records and passwords (Privacy…)",
        "Finished commands and their output, command history, weft's log and the agent's log: each on or off, how long they are kept and how \
         big they may grow. Keys typed are written nowhere, and a command typed with a space first is left out. ssh passwords are never \
         written to disk: kept encrypted in memory (the key changed now and then), and handed to ssh by a ticket used once.",
    ),
    (
        "Shells",
        "The local shell: PowerShell 7 / Windows PowerShell / cmd / nushell on Windows; the login shell or the one chosen on macOS and Linux \
         (the palette's Shells…). bash, zsh, fish and PowerShell know weft's command blocks and the folder exactly through its integration, \
         and weft's runs empty the line in any editing mode (vi or emacs). Other shells are guessed.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The help is whole in both languages: the same topics and keys, each
    /// written in its language (no Korean left in the English).
    #[test]
    fn the_help_is_in_korean_and_english() {
        assert_eq!(KEYS_KO.len(), KEYS_EN.len());
        assert_eq!(FEATURES_KO.len(), FEATURES_EN.len());
        let hangul = |s: &str| s.chars().any(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c));
        assert!(KEYS_EN.iter().chain(FEATURES_EN).all(|(a, b)| !hangul(a) && !hangul(b)), "English with no Hangul");
        assert!(FEATURES_KO.iter().all(|(_, b)| hangul(b)), "Korean");
        assert!(korean("ko") && !korean("en"));
        assert_eq!(keys(true), KEYS_KO);
        assert_eq!(features(false), FEATURES_EN);
    }
}
