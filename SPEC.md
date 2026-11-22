# 팝써치 (PopSearch) 설계

전역 단축키로 바로 띄우는 AI 검색창. Windows 먼저, macOS 공용 구조(Apple Silicon 맥에서 빌드·동작 확인, 서명 없는 `.app` 번들).

## 확정 사항
| 항목 | 결정 |
|---|---|
| 스택 | Tauri 2 + Vite + TypeScript (프레임워크 없는 vanilla) |
| 백엔드 | Rust. AI 호출·키 저장·히스토리·단축키·클립보드는 모두 Rust 쪽 |
| 인증 | 두 가지 공급자, 설정에서 선택: **API**(API 키) / **CLI**(Claude Code CLI, 구독) |
| 기본 모델 | Haiku (`claude-haiku-5-5`, CLI는 `haiku` 별칭). effort `low` |
| 대화 | 이어서 묻기 가능. Esc로 닫으면 초기화. 단축키로 숨기거나 다른 곳을 클릭해 숨으면 대화 유지 |
| 웹 검색 | 두 방식 모두 켜고 끌 수 있음. API는 `web_search`·`web_fetch` 서버 도구(페이지 열기), CLI는 켜면 Claude Code의 WebSearch/WebFetch 도구 사용(끄면 도구 없음; 설정을 바꾸면 CLI 세션을 다시 띄움). 검색 중에는 답변 아래에 `🔎 웹 검색 중…`/`🌐 페이지 읽는 중…` 상태 줄 표시(답변·복사·히스토리에 포함 안 됨). 시스템 프롬프트 규칙: 검색했으면 검색 결과에 나온 URL만 본문에 마크다운 링크로 넣고 URL을 지어내지 않으며, 이미지·영상 요청에는 찾은 페이지·영상 링크를 직접 제시하고, 따로 요청하지 않아도 가장 관련 있는 결과 페이지 1~2개를 fetch 도구로 열어 이미지·영상 직접 주소를 제시, 특정 페이지 속 이미지·파일·링크 요청에는 fetch 도구(API `web_fetch`, CLI `WebFetch`)로 그 페이지를 열어 본문 속 주소를 그대로 제시(`//`는 https:, 상대경로는 페이지 origin으로 보완). 입력창 Alt+Enter는 설정과 무관하게 그 질문만 웹 검색으로 보냄(macOS는 Cmd+Enter도 같음 — 조합 중 Option+Enter는 한국어 입력기의 한자 변환이 가져감)(CLI는 모델별 웹 켬 세션을 따로 띄움). |
| 단축키 | 기본 `Ctrl+Shift+Space` (macOS `Cmd+Shift+Space`), 설정에서 변경. 등록 실패(다른 프로그램이 사용 중) 시 창과 설정을 열어 알림. F5 = 설정 열기/닫기 |
| 복사 | 전체 복사 + 블록별 복사. 클립보드에 HTML과 일반 텍스트를 함께 넣음 (웹페이지 복사와 같음). 코드 블록의 텍스트는 울타리 없는 본문만 |
| 결과 미리보기 | 답변 완료 후, 답변 속 유튜브 링크는 썸네일 카드로, 이미지 링크는 작은 미리보기로 답변 아래에 표시(복사 대상 아님). 답변 속 마크다운 이미지는 화면 폭에 맞춤, 깨진 이미지는 숨김 |
| 웹 검색 출처 | API 모드 웹 검색 시 답변 아래에 출처(제목·도메인) 표시, 클릭 시 브라우저. 히스토리·MD 저장에 포함, 복사 대상 아님 |
| 히스토리 | 로컬 SQLite(앱 데이터 폴더 `history.db`). 목록·검색(부분 일치 LIKE — FTS5 기본 토크나이저는 한국어 부분 검색이 안 됨)·다시 열기·삭제, MD 파일로 저장. 기록을 연 채 질문하면 그 기록을 텍스트로 첫 질문 앞에 붙여 이어서 묻고, 같은 기록에 계속 저장. 단축키 Ctrl+H(macOS는 Cmd+H, 물리 키 H로 판별해 한국어 입력기에서도 동작). macOS 앱 메뉴는 Tauri 기본 메뉴에서 Hide(Cmd+H)만 뺀 명시적 메뉴(앱·Edit)를 사용 |
| 모델 선택 | 입력 맨 앞 `/s ` = Sonnet, `/o ` = Opus, `/h ` = Haiku (이번 질문만; 라틴 문자 h/s/o가 먼저 적용되고(대소문자 무관), 라틴이 아닌 자판 배열이면 물리 키 위치(`KeyboardEvent.code`)로 판정하며, 붙여넣은 한글 `/ㄴ`·`/ㅐ`·`/ㅗ`는 마지막 대체 규칙). 프리픽스가 없으면 설정의 **기본 모델**(Haiku/Sonnet/Opus, 기본 Haiku)을 쓰며 CLI 예열도 기본 모델로 한다. 입력창의 모델 표시(작은 알약)가 다음 질문에 쓸 모델을 보여 주고, 프리픽스 입력 중에는 `모델 (이번만)`으로 강조한다. `Alt+↑/↓`로 이번 질문의 모델을 한 단계 올리거나 내림(접두어가 우선, 전송·Esc 후 기본으로 복귀). 맥락별(API는 공유 대화 하나, CLI는 모델별 프로세스)로 아직 못 본 이전 대화를 텍스트 프리픽스로 따라잡아 보낸다. API의 Sonnet/Opus는 서버 측 거절 폴백(`fallbacks: "default"`)을 쓴다 |
| 첨부 | 붙여넣기(Ctrl+V)·드래그 앤 드롭. Claude가 그대로 읽는 형식만 변환 없이 전달: 이미지(png/jpeg/gif/webp, 미리보기 칩), PDF(document base64, 최대 25MB), UTF-8로 읽히는 모든 텍스트 파일(document text, 최대 2MB). 그 외 형식은 거부 안내. 최대 5개. API·CLI 모두(CLI document 블록 2026-11-17 실측). 히스토리에는 이미지 축소본과 파일명만 저장. docx/xlsx/pptx/hwpx는 앱에서 텍스트(마크다운)만 추출해 텍스트 첨부로 보냄(그림·서식 제외). doc/xls/hwp는 미지원. |
| 선택 텍스트 | 다른 앱에서 선택한 채 단축키 → 입력창에 넣음. Windows: ① UI Automation(TextPattern)으로 선택 텍스트를 직접 읽음(클립보드 무관) ② 실패하면 Ctrl+C로 복사해 텍스트·이미지(비트맵)를 읽고 원래 클립보드를 모든 형식 그대로 되돌림. 이미지만 선택한 경우 첨부 이미지로 붙음, 글·그림을 섞어 선택하면 ①이 텍스트만 줌. 터미널·코드 에디터(목록은 selection.rs)에서는 Ctrl+C가 명령 중단·줄 복사가 될 수 있어 ②를 쓰지 않는다. 클립보드 백업에 실패하면 키를 보내지 않는다. 복사 결과에 "빈 선택에서 줄 복사" 표식(MSDEVLineSelect, VisualStudioEditorOperationsLineCutCopyClipboardTag, vscode-editor-data의 isFromEmptySelection)이 있으면 선택 없음으로 본다. UIA로 텍스트를 얻어도 (제외 앱이 아니면) 복사→HTML에서 <img>를 찾아 data:/file:/http(s) 이미지를 최대 5장 내려받아 뒤이어 첨부한다(창은 먼저 뜬다). macOS는 아직 Cmd+C 방식(복원 없음)이며, 입력기와 무관하게 물리 C 키코드로 Cmd+C를 보내고 팝써치 자신이 활성 앱이면 복사하지 않는다. 설정에서 끌 수 있음(기본 켬). 입력창은 여러 줄(Enter 전송, Shift+Enter 줄바꿈) |
| 자동 실행·설치 | 설치본(NSIS, 사용자 단위 설치)에서 로그인 시 자동 실행(설정에서 끄기 가능, 기본 켬). 개발 빌드는 자동 실행 항목을 건드리지 않음. macOS: `npm run tauri build -- --bundles app`으로 만든 `.app`을 `/Applications`에 두고 실행하면 LaunchAgent(`~/Library/LaunchAgents/PopSearch.plist`)로 등록. Dock에 뜨지 않는 Accessory 앱(메뉴 막대 아이콘), API 키는 키체인 |

## 구조
```
src/            프론트엔드 (TS) — 입력창, 결과 렌더링, 복사, 히스토리, 설정 화면
  provider_api.ts   API 공급자 — 프론트에서 공식 TS SDK(@anthropic-ai/sdk)로 호출
src-tauri/src/  Rust
  provider_cli.rs   CLI 공급자 — claude.exe를 stream-json 모드로 상주시켜 사용
  settings.rs       설정(JSON, 앱 설정 폴더) 저장, API 키는 OS 자격 증명 저장소(keyring)
  history.rs        SQLite (rusqlite bundled), LIKE 검색
  ...
```
두 공급자는 같은 이벤트를 프론트로 보낸다: `chunk {text}`, `done {usage?}`, `error {message}`.

## CLI 공급자 메모 (2026-11-17 실측)
- 실행 인자: `-p --model <m> --tools "" --strict-mcp-config --setting-sources "" --disable-slash-commands --no-session-persistence --system-prompt <p> --input-format stream-json --output-format stream-json --include-partial-messages --verbose`
- 이 인자면 사용자 `CLAUDE.md`·MCP·도구가 빠지고 입력은 약 1천 토큰. `--bare`는 OAuth(구독)를 쓰지 않아 사용 불가.
- Windows에서 `claude.cmd`를 셸로 실행하면 빈 문자열 인자(`""`)가 사라진다 → **실제 `claude.exe`를 직접 실행**한다. npm 설치 시 위치: `%APPDATA%\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe`. `POPSEARCH_CLAUDE_BIN` 환경 변수로 경로를 지정할 수 있다. macOS/Linux에서는 PATH에 없으면 `~/.local/bin`, `~/.claude/local`, `/opt/homebrew/bin`, `/usr/local/bin`도 찾는다(Finder·Dock으로 실행한 앱은 PATH가 최소이기 때문).
- 입력 한 줄: `{"type":"user","message":{"role":"user","content":"..."}}`
- 출력: `stream_event`(`content_block_delta`)로 글자 조각, `result`로 한 턴 종료.
- 실측: 시작 포함 첫 응답 1.7초, 상주 상태 0.6초. 같은 프로세스는 대화를 기억 → 새 대화는 프로세스 교체 (다음 프로세스를 미리 띄워 둔다).
- 이미지: `content`를 배열로 `[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"..."}},{"type":"text","text":"..."}]` 보내면 인식됨 (2026-11-17 실측).
- 모델은 프로세스 시작 시 고정 → `/s`·`/o`는 해당 모델 프로세스를 따로 띄운다.

## 백틱(코드 울타리) 처리
- 시스템 프롬프트: 코드 안에 ``` 가 있으면 바깥 울타리를 더 길게 쓰라고 지시.
- 렌더링: CommonMark 파서(markdown-it). 스트리밍 중 닫히지 않은 울타리도 깨지지 않게.
- 저장·복사: 코드 안 최장 백틱 연속 길이 + 1 (최소 3)로 울타리를 다시 계산.
