# 20261122-1055_macos-hotkey-cli-path

Power: mid

## 요약
맥 첫 실행 점검. ① 단축키를 눌러도 창이 안 보인다 — 창 위치가 옮겨진 흔적으로 보아 창이 떴다가 바로 숨겨지는 것으로 추정되어, 원인 확인용 진단 로그(개발 빌드 전용)를 넣는다. ② Finder·Dock으로 실행한 맥 앱은 PATH가 `/usr/bin:/bin:/usr/sbin:/sbin`뿐이라 `~/.local/bin/claude`를 못 찾으므로, 맥(유닉스)에서 흔한 설치 위치를 PATH 다음으로 직접 찾게 한다.

## Order 1
Target files: `src-tauri/src/provider_cli.rs`, `src-tauri/src/lib.rs`, `SPEC.md`

### Background
- On macOS, apps launched from Finder/Dock/login items get `PATH=/usr/bin:/bin:/usr/sbin:/sbin` only. The official Claude Code installer puts the binary at `~/.local/bin/claude`, so `find_claude_bin()` returns `None` there.
- On macOS the global hotkey seems to fire (the hidden main window was moved to the position `show_window_with` computes), but the user never sees the window. Suspected: the window is shown and then immediately hidden by the `WindowEvent::Focused(false)` handler, because the app runs with `ActivationPolicy::Accessory`. We need logs to confirm before fixing.

### Changes
1. `src-tauri/src/provider_cli.rs` — in `find_claude_bin()`, non-Windows branch (`else { dirs.iter().map(|d| d.join("claude")).find(|p| is_file(p)) }`):
   - Keep checking PATH dirs first, unchanged.
   - If not found in PATH, check these fallback locations in this order, using `std::env::var_os("HOME")` for `~` (skip HOME-based entries if HOME is unset):
     1. `$HOME/.local/bin/claude`
     2. `$HOME/.claude/local/claude`
     3. `/opt/homebrew/bin/claude`
     4. `/usr/local/bin/claude`
   - Put the fallback list in a small helper (e.g. `fn unix_fallback_claude_paths(home: Option<&Path>) -> Vec<PathBuf>`) so it can be unit-tested without touching the real filesystem; add a unit test in the existing `#[cfg(test)]` module that checks the order and that HOME-based entries are omitted when home is `None`.
   - Do not change the Windows branch or the `POPSEARCH_CLAUDE_BIN` handling.
2. `src-tauri/src/lib.rs` — add temporary diagnostic logging that is compiled ONLY in debug builds (`#[cfg(debug_assertions)]`, use `eprintln!` with a `[popsearch-debug]` prefix). Log:
   - in the global shortcut handler: every event with its state (Pressed/Released);
   - in `toggle_window`: which path is taken (hide because visible+focused / capture thread / direct show);
   - in `show_window_with`: right after `win.show()` and `win.set_focus()`, the `Result` of each (`is_ok()` / error);
   - in `on_window_event`: every `WindowEvent::Focused(_)` for the main window with its bool, and whether the hide was executed or suppressed by `SUPPRESS_HIDE`.
   - Release builds must produce no extra output and no behavior change.
3. `SPEC.md` — line 39 says "설정에서 경로 지정 가능." which is not implemented. Replace that sentence with a Korean sentence stating: the path can be overridden with the `POPSEARCH_CLAUDE_BIN` environment variable; on macOS/Linux, if not on PATH, it also looks in `~/.local/bin`, `~/.claude/local`, `/opt/homebrew/bin`, `/usr/local/bin` (because apps launched from Finder/Dock get a minimal PATH).

### Constraints
- Do not modify, create, or delete any file not listed above.
- Never rewrite or regenerate a whole file; make targeted edits only.
- Do not change hotkey, focus, or hide behavior in this Order — logging only.
- Do not touch the real app data folders (`~/Library/Application Support/com.popsearch.app`).
- Delete any file you created for testing before finishing.

### Done when
- `cargo check --manifest-path src-tauri/Cargo.toml` passes with no new warnings.
- `cargo test --manifest-path src-tauri/Cargo.toml` passes, including the new fallback-path test.
- `npm run build` passes.
- `grep -n "popsearch-debug" src-tauri/src/lib.rs` shows each log is inside a `#[cfg(debug_assertions)]` scope.

## Review 1
- 판정: PASS
- 확인 내용: 스냅샷 대비 diff로 Changes 3항목을 대조했다. (1) provider_cli.rs는 PATH 검색 뒤에 HOME/.local/bin, HOME/.claude/local, /opt/homebrew/bin, /usr/local/bin 순서로 폴백하고, Windows 분기와 POPSEARCH_CLAUDE_BIN 처리는 그대로다. 헬퍼와 순서·HOME 없음 테스트가 있다. (2) lib.rs의 [popsearch-debug] 로그(단축키 이벤트, toggle 3경로, show/set_focus 결과, Focused 값과 SUPPRESS_HIDE, hide 실행)는 모두 #[cfg(debug_assertions)] 안에 있고 동작 변경은 없다. 억제된 경우는 Focused 로그의 suppress 값으로 알 수 있다. (3) SPEC.md 39행이 교체됐다. cargo check 통과(경고 6개는 keyhook.rs의 기존 경고), cargo test --lib 48개 통과. 이전 Order 변경이 사라진 부분과 깨진 문자는 없다.
- 직접 수정: 없음
- 문서: SPEC.md 반영됨
- 참고: PATH 환경 변수가 아예 없으면 기존 `?`가 폴백 전에 None을 반환한다. Finder 실행 앱은 최소 PATH를 갖고 있어 실제 영향은 거의 없다. 로그는 개발 빌드 전용이라 단축키 창 문제의 원인은 `npm run tauri dev` 로그를 보고 판단해야 한다. npm run build는 실행하지 않았고 프론트 변경이 없다.

## 진단 결과 (1단계)
- 로그: 단축키 → capture thread → show ok / set_focus ok → Focused(true) → Focused(false) → hide. 창은 실제로 떴다가 포커스를 잃어 숨겨진다. 키체인 접근 확인 창(API 키 읽기)도 포커스를 가져간다.
- 팝써치 창이 숨겨진 직후(팝써치가 여전히 활성 앱일 때) 단축키를 누르면 capture thread 진입 후 창을 띄우기 전에 앱이 종료 코드 0으로 꺼졌다(2회 재현, 사용자는 종료하지 않음). 합성 키가 팝써치 자신에게 들어가 기본 앱 메뉴 단축키(종료 등)로 처리된 것으로 추정. 입력기는 한국어 두벌식 — enigo `Key::Unicode('c')`는 현재 입력기 배열로 키코드를 찾으므로 엉뚱한 키가 될 수 있다.

## Order 2
Target files: `src-tauri/src/selection.rs`, `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml`, `SPEC.md`

### Background
On macOS, `capture_other()` sends `Meta` + `Key::Unicode('c')` via enigo 0.6. `Key::Unicode` resolves the keycode through the current keyboard layout; with the Korean 2-Set input source active this may not be the C key. Also, when PopSearch itself is the active application (window hidden but app still active, e.g. after a Keychain prompt closes), the synthetic keystroke goes to PopSearch itself, and the app was observed to quit (exit code 0) right after the capture thread started.

### Changes
1. `src-tauri/src/selection.rs` — macOS only (`#[cfg(target_os = "macos")]`): send the copy shortcut using the physical key code for C (`kVK_ANSI_C` = `8`) instead of `Key::Unicode('c')`, so it is independent of the input source. Use enigo 0.6's raw/keycode API (check the enigo 0.6.1 source in `~/.cargo/registry` for the correct call, e.g. `Keyboard::raw(8, Direction::Click)` or `Key::Other(8)`). Keep the Meta press/release as is. Non-macOS non-Windows (Linux) behavior stays unchanged (`Key::Unicode('c')`).
2. `src-tauri/src/lib.rs` — in `toggle_window`, on macOS, if PopSearch is the currently active application, do NOT start the capture thread; show the window directly (same as the `capture == false` path). Detect "active" with `NSRunningApplication::currentApplication().isActive()` from `objc2-app-kit` (thread-safe; do not require the main thread). Add a `#[cfg(debug_assertions)]` `[popsearch-debug]` log line stating the app-active value when this check runs on macOS.
3. `src-tauri/Cargo.toml` — add a macOS-only dependency section `[target.'cfg(target_os = "macos")'.dependencies]` with `objc2-app-kit = { version = "0.3.2", features = [...] }` (only the features needed for `NSRunningApplication`) and `objc2-foundation`/`objc2` only if required. Use versions already in `Cargo.lock` (objc2 0.6.5, objc2-app-kit 0.3.2, objc2-foundation 0.3.2) so no new crate versions are pulled.
4. `SPEC.md` — in the 선택 텍스트 row, the sentence "macOS는 아직 Cmd+C 방식(복원 없음)." : extend it in Korean to say macOS sends Cmd+C with the physical C key code (independent of input source) and skips copying when PopSearch itself is the active app.

### Constraints
- Do not modify, create, or delete any file not listed above (Cargo.lock may change only as a result of the dependency addition; it must not change any existing crate version).
- Never rewrite or regenerate a whole file; make targeted edits only.
- Do not change Windows code paths or the focus/hide logic in `on_window_event`.
- Keep the existing debug logs.
- Do not touch the real app data folders (`~/Library/Application Support/com.popsearch.app`) or the Keychain.
- Delete any file you created for testing before finishing.

### Done when
- `cargo check --manifest-path src-tauri/Cargo.toml` passes with no new warnings.
- `cargo test --manifest-path src-tauri/Cargo.toml` passes.
- `git diff src-tauri/Cargo.lock` shows no version change of existing crates.
- `grep -n "Unicode('c')" src-tauri/src/selection.rs` shows it is no longer used on macOS.

## Review 2
- 판정: PASS
- 확인 내용: 스냅샷 대비 diff로 Changes 4항목을 대조했다. (1) selection.rs: macOS에서만 `enigo.raw(8, Direction::Click)`를 쓰고 그 밖에는 `Key::Unicode('c')`를 유지한다. enigo 0.6.1 소스(macos_impl.rs `raw`)는 keycode를 `CGEvent::new_keyboard_event`에 그대로 넘기므로 8은 kVK_ANSI_C다. Meta 누름 상태의 플래그가 이벤트에 붙는다. (2) lib.rs: `NSRunningApplication::currentApplication().isActive()`를 macOS에서만 호출하고 `capture && !own_focus && !app_active`로 캡처 스레드를 건너뛴다. objc2-app-kit 0.3.2에서 두 메서드는 `MainThreadOnly` 제약이 없는 안전한 함수라 메인 스레드가 아니어도 된다. 디버그 로그는 `cfg(all(debug_assertions, macos))` 안에 있고 기존 로그는 유지됐다. 비 macOS는 `app_active=false`라 Windows 동작이 그대로다. (3) Cargo.toml: macOS 전용 섹션에 objc2-app-kit 0.3.2 + `NSRunningApplication` feature만 추가했다. (4) SPEC.md 문구가 반영됐다. Cargo.lock diff는 추가뿐이다. 기존 crate의 버전 변경은 없고, 기존 objc2-app-kit 항목의 의존 목록에 feature 연동 항목이 늘었으며, 신규 crate는 objc2-core-video 0.3.2 하나다(1단계가 허용). cargo check 통과(경고 6개는 기존 keyhook.rs 경고), cargo test --lib 48개 통과. `Unicode('c')`는 비 macOS 분기에만 남았다. 실제 GUI와 키 합성은 실행하지 않았다.
- 직접 수정: 없음
- 문서: SPEC.md 반영됨
- 참고: 실제 동작은 맥에서 한국어 입력기 상태로 선택 텍스트 캡처와 "팝써치가 활성 앱일 때 단축키" 시나리오를 직접 확인해야 한다. 팝써치가 활성일 때는 이전 앱 선택 텍스트를 가져오지 않고 창만 뜬다(의도된 동작). Accessory 앱이 숨겨진 직후에도 isActive가 true로 남는다는 가정에 기댄 수정이라 재현 여부를 확인할 필요가 있다. 창이 곧바로 숨겨지는 Focused(false) 문제(키체인 확인 창 포함)는 이번 Order 범위가 아니라 별도 수정이 필요하다.

## 사용자 확인 (Order 2 이후)
- 단축키로 창이 정상으로 뜨고 유지됨. 창이 숨겨진 직후 단축키로 앱이 꺼지는 문제는 재현되지 않음.
- 맥 한국어 입력기는 조합 중일 때 `Option+Enter`를 한자 변환으로 가로챈다(띄어쓰기로 조합을 끝내면 웹 검색 전송이 됨). → 맥에서는 `Cmd+Enter`도 웹 검색 전송으로 추가 (Order 3).

## Order 3
Target files: `src/main.ts`, `README.md`, `SPEC.md`

### Background
The input box sends "this question only with web search" on `Alt+Enter` (`const forceWeb = e.altKey;` in the `input` keydown handler around `src/main.ts:822`). On macOS, `Option+Enter` is intercepted by the Korean input method for Hanja conversion while a syllable is being composed. Add `Cmd+Enter` as an equivalent on macOS only.

### Changes
1. `src/main.ts` — in the `input` keydown handler that handles `Enter`: on macOS, `forceWeb` must be true when `e.altKey` OR `e.metaKey`. On other platforms keep `e.altKey` only (Ctrl/Win+Enter behavior on Windows must not change). Detect macOS the same way `src/settings.ts` does (`navigator.userAgent.includes("Mac")`); compute it once at module level. Do not change the `isComposing`/`keyCode === 229` guard or the Shift+Enter newline behavior.
2. `README.md` — the shortcut table row for `Alt+Enter` (line ~80): state in Korean that on macOS it is `Option+Enter` or `Cmd+Enter`, and that the Korean input method uses `Option+Enter` for 한자 변환 while composing, so `Cmd+Enter` is recommended on macOS.
3. `SPEC.md` — in the 웹 검색 row, the sentence "입력창 Alt+Enter는 설정과 무관하게 그 질문만 웹 검색으로 보냄": add in Korean that macOS also accepts `Cmd+Enter` (Option+Enter is taken by the Korean IME's 한자 변환 while composing).

### Constraints
- Do not modify, create, or delete any file not listed above.
- Never rewrite or regenerate a whole file; make targeted edits only.
- Do not change any other keyboard shortcut handling.
- Delete any file you created for testing before finishing.

### Done when
- `npm run build` passes.
- `grep -n "metaKey" src/main.ts` shows the new check only inside the Enter handler, gated by the macOS flag.

## Review 3
- 판정: PASS
- 확인 내용: 스냅샷 대비 diff가 지시 3건(main.ts 모듈 상단 IS_MAC 상수 + forceWeb 한 줄, README 표 행, SPEC 웹 검색 행)만 포함, 다른 변경·삭제 없음. `npm run build` 통과. metaKey는 Enter 핸들러에서만 IS_MAC 조건으로 쓰임(다른 metaKey 사용처는 기존 가드). isComposing/229 가드·Shift+Enter 그대로. window keydown(841~)에는 Enter/Cmd+Enter 처리가 없어 이중 발화 없음(대부분 metaKey를 제외하는 가드).
- 직접 수정: 없음
- 문서: README, SPEC 반영됨
- 참고: macOS에서 Cmd+Enter는 이제 항상 웹 검색 강제 전송이다. IME 조합 중(isComposing)에 Cmd+Enter를 누르면 가드에 막혀 전송되지 않을 수 있어, 조합 확정 후 눌러야 할 수 있음(Option+Enter와 동일 조건).

## 1단계 직접 수정 (아이콘)
- `index.html`: 설정 버튼 `⚙`(텍스트 글리프) → `⚙️`(U+2699 U+FE0F, 이모지 표시). 맥에서 🕘(컬러 이모지)와 크기·모양이 달라 보이던 문제. 동작 변경 없음.

## Order 4
Target files: `src/main.ts`, `src-tauri/src/lib.rs`, `README.md`, `SPEC.md`

### Background
- History toggle is `if (e.ctrlKey && !e.shiftKey && !e.altKey && (e.key === "h" || e.key === "H"))` in the window keydown handler (~`src/main.ts:905`). Problems on macOS: (a) Cmd is not accepted (macOS users expect Cmd+H); (b) with the Korean input source, `e.key` is `"ㅗ"`, so it never matches.
- Tauri 2 enables the macOS default app menu (`enable_macos_default_menu` defaults to true; this app sets no app menu — the `.menu(&menu)` in lib.rs is the tray menu). That default menu has "Hide PopSearch" bound to Cmd+H, which can hide the app before the webview sees the key. It also has the Edit menu, which is REQUIRED for Cmd+C/V/X/A/Z in the textarea, so the menu must not simply be disabled.

### Changes
1. `src/main.ts` — history toggle condition: modifier is `e.ctrlKey` OR (`IS_MAC` && `e.metaKey`) (reuse the existing module-level `IS_MAC`); key match is `e.key === "h" || e.key === "H" || e.code === "KeyH"` (physical key, input-source independent); keep `!e.shiftKey && !e.altKey`. Windows behavior (Ctrl+H) must stay the same.
2. `src/main.ts` — on macOS, set the history button tooltip (`#history-btn` `title`) to `기록 (Cmd+H)` at startup; leave `index.html` as is (Windows keeps `기록 (Ctrl+H)`).
3. `src-tauri/src/lib.rs` — macOS only (`#[cfg(target_os = "macos")]`): set an explicit app menu instead of Tauri's default, equal to the default but WITHOUT a Cmd+H accelerator for Hide (drop the Hide item, or keep it without accelerator). It must contain: an app submenu (About, Services, Hide Others / Show All optional, Quit with Cmd+Q) and an Edit submenu with Undo, Redo, Cut, Copy, Paste, Select All using `PredefinedMenuItem`s. Use `tauri::menu::{Menu, Submenu, PredefinedMenuItem}` (check tauri 2.12.6 source `menu/menu.rs` `Menu::default` in `~/.cargo/registry` and mirror it). Set it with `app.set_menu(...)` in `setup` or `Builder::menu`. Do not touch the tray menu.
4. `README.md` — line 19 (`Ctrl+H`) and shortcut table line 77: note macOS uses `Cmd+H` (also works with the Korean input source). `SPEC.md` — if it mentions the history shortcut, add the same; if not, add one short Korean line in the relevant UI/shortcut place describing Ctrl+H (macOS Cmd+H, physical key so Korean input works) and the macOS app menu without Hide's Cmd+H.

### Constraints
- Do not modify, create, or delete any file not listed above.
- Never rewrite or regenerate a whole file; make targeted edits only.
- Do not change Windows behavior or any other shortcut.
- Keep existing debug logs.
- Delete any file you created for testing before finishing.

### Done when
- `npm run build` passes.
- `cargo check --manifest-path src-tauri/Cargo.toml` passes with no new warnings; `cargo test --manifest-path src-tauri/Cargo.toml` passes.
- The macOS menu code is entirely inside `#[cfg(target_os = "macos")]`.

## Review 4
- 판정: PASS
- 확인 내용: 스냅샷 대비 diff가 Order 4의 Changes 4항목과 일치(main.ts 조건·툴팁, lib.rs macOS 메뉴, README 2곳, SPEC 1줄). Windows 경로는 `e.ctrlKey` 조건 그대로라 동작 불변. lib.rs 메뉴 코드는 전부 `#[cfg(target_os = "macos")]` 블록 안, 트레이 메뉴 미변경. tauri 2.12.6 `Menu::default`와 대조: 앱 서브메뉴(About/Services/Hide Others/Quit)와 Edit(Undo/Redo/Cut/Copy/Paste/Select All)는 동일하고 Hide(Cmd+H)만 제외됨. `npm run build` 통과, `cargo check` 통과(경고 6개는 Windows 전용 코드의 dead_code로 기존과 동일), `cargo test` 48개 통과. 테스트 잔여 파일 없음. GUI 실행은 지시대로 하지 않음.
- 직접 수정: 없음
- 문서: README(19행, 77행)·SPEC(히스토리 행) 반영됨.
- 참고: (1) 기본 메뉴의 File/View/Window/Help가 빠져 Cmd+W(창 닫기)·Cmd+M(최소화)·전체화면 단축키가 사라진다. 프레임 없는 항상-위 팝업 창이라 영향은 작고(Esc 등 자체 숨김 사용 시) 필요하면 Window 메뉴만 추가하면 된다. (2) About 항목의 메타데이터(버전·저작권)를 None으로 넘겨 기본값과 약간 다르다(표시만 다름). (3) macOS 실기기에서 Cmd+H가 앱 숨김 없이 기록 토글로 동작하는지는 실행 확인이 필요하다.

## 사용자 확인 (Order 4 이후)
- 맥 dev 앱에서 Cmd+H(한글·영어 입력 모두) 히스토리 열림, Cmd+C/V/A 정상, 아이콘 크기 일치 확인.

## 1단계 직접 수정 (문서)
- README·SPEC를 맥 지원 기준으로 갱신: 지원 현황, 기본 단축키(Cmd+Shift+Space), macOS claude 탐색 경로, 선택 텍스트 맥 동작, 데이터 위치(Application Support·키체인·LaunchAgent), 알려진 제한(서명 없음·클립보드 미복원·Intel 미테스트), macOS 빌드 절차, SPEC 자동 실행 행. 동작 변경 없음.
- README·SPEC: "빌드한 맥에서만 실행" 표현 정정 — 다른 Apple Silicon 맥에 복사해 쓸 수 있고 첫 실행 때 Gatekeeper 허용 필요. "다른 맥에서 쓰기" 절 추가. Intel 맥은 대상 아님(사용자 결정).
