# weft 구조

## 1. UI 계층

```
창 (eframe 창 하나: WeftApp → App)
├─ 상태바                          TopBottomPanel::bottom → App::ui_status
└─ 나머지 전체 (CentralPanel)       → App::ui_layout
    레이아웃 트리  settings.layout: Layout
    ├─ Layout::Split { id, vertical, ratio, a, b }     패널끼리 나누기 (중첩)
    └─ Layout::Panel { region }                         패널 하나
        ├─ Region::Side    → ui_sidebar   Files / Commands / Search / Procs
        ├─ Region::Editor  → ui_editor    파일 탭 → 편집창 (마크다운 in-place)
        └─ Region::Terms   → ui_tabbar + ui_panes
             App::tabs: Vec<Tab>                  탭
              └─ Tab::root: Node                  탭의 분할 트리
                  ├─ Node::Split { vertical, ratio, a, b }
                  └─ Node::Leaf(pane_id)          페인
                      └─ App::panes[pane_id]: Pane
                          ├─ term: Term   VT 격자, 스크롤백, 명령 블록 (term.rs)
                          ├─ pty: Pty     셸 프로세스 (pty.rs)
                          └─ widgets      인라인 위젯 (widgets.rs)
떠 있는 레이어 (마지막에 그려서 위에 덮임)
    팔레트, 연결 창, 비밀번호 창, 테마 메뉴, 토스트, 패널 드래그 표시 (ui_dock_drop)
```

용어:

| 용어 | 뜻 | 코드 |
|---|---|---|
| 패널 | 레이아웃 트리의 잎. 끌어서 옮길 수 있는 큰 구역 | `Region`, `Layout::Panel` |
| 탭 | 터미널 패널 안의 작업 화면 하나. 탭마다 자기 분할 트리 | `Tab` |
| 분할 | 둘로 나누기. 패널 층(`Layout`)과 페인 층(`Node`) 두 곳에 있음 | `Layout::Split`, `Node::Split` |
| 페인 | 터미널 하나 | `Pane` |

egui의 `Panel`/`Window`와 헷갈리지 않게: egui `SidePanel` 등은 상태바에만 쓰고, weft의 "패널"은
CentralPanel 안을 `Layout` 트리로 직접 나눈 사각형입니다. egui `Window`/`Area`는 떠 있는 창(팔레트 등)입니다.

## 2. 한 프레임

모든 상태는 `App` 하나에 있고, `update()`가 매 프레임 그 상태로 화면 전체를 다시 만듭니다 (immediate mode).

```
update()                                   main.rs
├─ 테마·배율 적용
├─ poll()            PTY 출력 → Term, 에이전트 응답 처리
├─ handle_global_keys
├─ 상태바
├─ ui_layout         Layout 트리 → 패널 사각형 → 패널마다 그리기, 경계선 드래그
├─ 떠 있는 것들 (팔레트, 연결, 토스트, 드래그 표시 …)
└─ apply_actions()   그리는 중에 쌓인 Action 처리 (분할, 연결, 파일 열기 …)
```

입력은 `App`을 고치고, 화면은 다음 프레임에 바뀐 `App`대로 그려집니다. 패널을 옮기는 것도
`settings.layout`을 바꾸는 것뿐입니다(`Layout::moved`).

## 3. 활성 터미널을 따라 바뀌는 것

패널은 하나씩이지만, 일부 내용은 "지금 활성인 페인"(활성 탭의 `Tab::active`)을 기준으로 보여 줍니다.

| 패널 | 기준 |
|---|---|
| Files | follow가 켜져 있으면 활성 페인의 cwd (OSC 7), 원격이면 원격 파일 |
| Commands | 활성 페인의 명령 블록 |
| Procs, Search | 활성 페인의 호스트 |
| 상태바 | 호스트, cwd, 칸 수 |
| 에디터 | 무관 (열린 파일 목록은 하나) |

## 4. 터미널 페인 그리기

- 칸 크기가 고정이라 위치는 (줄, 열)에서 바로 계산됩니다. 격자 글자, 커서, 선택은 `painter`로 직접 그립니다(ui_term.rs).
- ASCII는 런 단위, 그 외 문자는 칸 단위로 그려 폴백 폰트가 열을 밀지 않습니다.
- **명령 블록**: OSC 133으로 경계를 추적(term.rs `Block`) → 거터 막대, 상태 배지, hover 툴바를 격자 위에 겹쳐 그림.
- **인라인 위젯**: 원격 헬퍼가 OSC 7777로 위젯을 보내며 같은 줄 수의 텍스트 폴백을 출력 → GUI가 그 줄들 위에 덮어 그림.
- 크기가 바뀌면 열×행을 다시 계산해 리플로우(term.rs `reflow`)하고 PTY에 알림. 끄는 중에는 120ms 디바운스.
- 한글 IME: 조합 중에는 키를 IME에 맡기고 commit만 보냄, IME 창 위치는 커서 칸으로 계속 알림(`typed_bytes`).

## 5. 로컬 / 원격

```
로컬 페인:  pwsh / Windows PowerShell / cmd / nushell (+ 셸 통합)  ── ConPTY ── Pty
원격 페인:  ssh -tt host '<부트스트랩>'  → 로그인 셸($SHELL) + 그 셸의 통합 (메모리로만)
원격 에이전트: ssh -T host '<부트스트랩>' → python3 -c "exec(stdin)" (agent.py를 stdin으로)
```

- 원격에는 아무것도 저장하지 않습니다(rc는 프로세스 치환, 에이전트는 stdin).
- 원격에서 쓰는 건 ssh + POSIX 도구 + python 표준 라이브러리뿐. 렌더링·검색 필터·정렬·히스토리는 전부 로컬.
- 비밀번호는 입력했을 때만 SSH_ASKPASS(weft.exe 자신)로 넘기고, 아니면 키/ssh-agent.
- 셸별 통합, 통합이 없는 셸의 추정, OSC 목록, python이 필요한 곳: [shell-integration.md](shell-integration.md).

## 6. immediate mode에서 레이아웃을 푼 방법

egui는 UI를 만드는 코드 안에서 클릭을 바로 확인하므로(`if ui.button().clicked()`), 위젯을 만드는 순간
위치가 정해져 있어야 합니다. 여러 위젯을 함께 봐야 정해지는 배치는 이렇게 풉니다.

| 경우 | 방법 | 예 |
|---|---|---|
| 칸 크기가 고정 | 좌표를 바로 계산해 직접 그림 | 터미널 격자, 블록 막대 |
| 바깥 크기를 먼저 앎 | 트리로 사각형을 나눠 준 뒤 그 안에서 그림 | `Layout`, `Node` 분할 |
| 내용을 다 그려야 크기를 앎 | 빈 도형으로 자리 예약 → 나중에 채움 (`Shape::Noop` → `painter.set`) | 마크다운 코드 블록 배경 |
| 형제들 크기를 다 알아야 함 | 미리 한 번 재고 그 값으로 배치 | 마크다운 표 열 폭 (`sections_width`) |
| 다른 패널 위치가 필요 | 그 프레임에 그려진 사각형을 기록해 둠 | 드래그 놓을 곳 (`panel_rects`) |
