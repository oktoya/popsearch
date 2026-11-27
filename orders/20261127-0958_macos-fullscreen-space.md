# 20261127-0958_macos-fullscreen-space

Power: mid

## 요약
맥에서 다른 앱(유튜브 전체화면 등)이 전체화면이면 단축키를 눌러도 팝써치 창이 보이지 않는다. 전체화면 앱은 자기 공간(Space)으로 옮겨 가는데, 팝써치 창은 일반 데스크탑 공간에 묶여 있어 그쪽에 뜨기 때문이다(포커스만 가져감). 창의 공간 설정에 "모든 공간에 함께 있기(CanJoinAllSpaces)"와 "전체화면 앱 위 보조 창(FullScreenAuxiliary)"을 넣어 지금 보이는 공간에 뜨게 한다. 안 되면 다음 단계로 NSPanel 방식을 검토한다.

## Order 1
Target files: `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml`, `SPEC.md`

### Background
- The main window (`label: "main"`, frameless, `alwaysOnTop: true`, `visible: false`) is shown by `show_window_with` in `src-tauri/src/lib.rs`. The app runs with `ActivationPolicy::Accessory` on macOS (set in `setup`).
- On macOS, a full-screen app lives in its own Space. Our NSWindow belongs to the desktop Space, so `show()` makes it appear there (invisible to the user) while the app still takes key focus.
- tao 0.37.2 `set_visible_on_all_workspaces(true)` only adds `NSWindowCollectionBehavior::CanJoinAllSpaces`; it never adds `FullScreenAuxiliary`. `alwaysOnTop` sets `NSFloatingWindowLevel`.
- `objc2-app-kit = "0.3.2"` is already a macOS-only dependency (feature `NSRunningApplication`).

### Changes
1. `src-tauri/Cargo.toml` — add only the `objc2-app-kit` features needed to call `NSWindow::collectionBehavior` / `setCollectionBehavior` (e.g. `NSWindow`, `NSResponder` if required). Do not add new crates; versions must stay as in `Cargo.lock`.
2. `src-tauri/src/lib.rs` — macOS only (`#[cfg(target_os = "macos")]`):
   - Add a helper, e.g. `fn apply_macos_space_behavior(win: &tauri::WebviewWindow)`, that gets the NSWindow via `win.ns_window()` and sets its collection behavior to the existing value OR `CanJoinAllSpaces | FullScreenAuxiliary` (keep any existing bits; do NOT add `MoveToActiveSpace` together with `CanJoinAllSpaces` — they are mutually exclusive). AppKit calls must run on the main thread: use `win.run_on_main_thread(...)` (or `app.run_on_main_thread`) and obtain the NSWindow pointer inside that closure.
   - Call it once in `setup` for the `main` window after the activation policy is set, and also at the start of `show_window_with` before `win.show()` (cheap, guards against tao resetting the behavior).
   - Keep the window level as is (floating via alwaysOnTop). Do not change focus/hide logic.
   - Add a `#[cfg(all(debug_assertions, target_os = "macos"))]` `[popsearch-debug]` log line printing the resulting collection behavior bits once at setup.
3. `SPEC.md` — add one Korean sentence (in the 단축키 row or the macOS notes): macOS에서는 창이 모든 공간에 함께 있고 전체화면 앱 위에도 뜨도록(CanJoinAllSpaces + FullScreenAuxiliary) 설정함.

### Constraints
- Do not modify, create, or delete any file not listed above (Cargo.lock may change only through feature flags, no version changes).
- Never rewrite or regenerate a whole file; make targeted edits only.
- No Windows behavior change; all new code inside `#[cfg(target_os = "macos")]`.
- Keep existing debug logs.
- Do not start a GUI instance. Delete any file you created for testing before finishing.

### Done when
- `cargo check --manifest-path src-tauri/Cargo.toml` passes with no new warnings; `cargo test --manifest-path src-tauri/Cargo.toml` passes.
- `git diff src-tauri/Cargo.lock` shows no crate version changes.
- (GUI check by user: YouTube full screen in Safari/Zen → hotkey → PopSearch appears on top of the full-screen video; Esc hides it and the video stays full screen.)

## Review 1
- 판정: PASS
- 확인 내용: diff 대조 결과 Changes 3항목(Cargo.toml features, lib.rs 헬퍼+setup 호출+show 전 호출, SPEC.md 한 문장) 모두 반영. 헬퍼는 macOS cfg 안, run_on_main_thread 클로저 안에서 ns_window() 결과를 null 검사 후 NSWindow로 캐스팅하고 기존 비트에 CanJoinAllSpaces|FullScreenAuxiliary를 OR(MoveToActiveSpace 없음). cargo check 통과(경고 6개는 keyhook 등 기존 Windows 전용 코드 관련, 신규 없음), cargo test 통과, Cargo.lock 변경 없음, 불필요한 파일 없음. GUI는 실행하지 않음.
- 직접 수정: 없음
- 문서: SPEC.md 반영됨
- 참고: (1) 디버그 로그가 show마다 찍힘(디버그 빌드 한정, 무해). (2) run_on_main_thread와 이어지는 win.show()는 모두 이벤트 루프 메시지로 전달되어 순서가 유지될 가능성이 높고, 설정은 setup에서 이미 한 번 적용되며 창에 유지되므로 순서는 중요하지 않음. (3) CanJoinAllSpaces로 창이 모든 공간에 있으므로 실제 전체화면 위 표시는 사용자 GUI 확인 필요(안 되면 NSPanel 방식 검토).

## 사용자 확인 (Order 1 이후)
- 전체화면 유튜브 위에 여전히 안 보임. 로그: `show() ok → set_focus() ok → Focused(true) → Focused(false) → hide executed`가 사용자 조작 없이 연달아 일어남(bits=0x101 적용 확인). 일반 창은 앱을 활성화하므로 전체화면 공간의 주인 앱에 포커스가 되돌아가 바로 숨겨지는 것으로 판단.
- 결정: NSPanel(앱 활성화 없이 키 입력을 받는 패널)로 전환. 외부 확장(tauri-nspanel) 대신 objc2로 직접 구현(B안).

## Order 2
Target files: `src-tauri/src/lib.rs`, `src-tauri/Cargo.toml`, `SPEC.md`

### Background
- Spotlight-style apps show over other apps' full-screen Spaces by using an `NSPanel` with the `NonactivatingPanel` style: the panel becomes key (receives keyboard input) without activating the app, so the full-screen app stays frontmost and no focus is bounced back.
- Our main window is a tao-created `NSWindow` (tao uses its own NSWindow subclass that overrides `canBecomeKeyWindow` → YES, needed because the window is borderless/frameless). tao's window delegate produces the `WindowEvent::Focused` events we rely on (hide on focus loss); the delegate must keep working.
- `win.set_focus()` on macOS calls `NSApp.activateIgnoringOtherApps` (activates the app) — this must NOT be used for the panel path.
- Order 1's `apply_macos_space_behavior` (CanJoinAllSpaces | FullScreenAuxiliary) stays; keep it.

### Changes
1. `src-tauri/Cargo.toml` — macOS-only deps: add the `objc2-app-kit` features needed (e.g. `NSPanel`, `NSApplication` if used) and, if needed for class definition / messaging, `objc2 = "0.6.5"` as a direct macOS-only dependency (same version as `Cargo.lock`; no new crate versions).
2. `src-tauri/src/lib.rs` — macOS only (`#[cfg(target_os = "macos")]`):
   - Define (once, with objc2's `define_class!` or equivalent) a runtime subclass of `NSPanel`, e.g. `PopSearchPanel`, that overrides `canBecomeKeyWindow` → YES and `canBecomeMainWindow` → NO. It must not add ivars (so `object_setClass` on the existing instance is layout-safe).
   - In `setup`, on the main thread, convert the `main` window: `object_setClass(ns_window, PopSearchPanel::class())`, then add `NSWindowStyleMask::NonactivatingPanel` to the style mask, `setFloatingPanel(true)`, `setHidesOnDeactivate(false)`, `setBecomesKeyOnlyIfNeeded(false)`, keep the existing level (floating) or set `NSStatusWindowLevel` if floating is not enough over full screen — start with the floating level and document the choice. Keep the delegate untouched. Reapply `apply_macos_space_behavior` after conversion.
   - Add a macOS show path used by `show_window_with` instead of `win.show()` + `win.set_focus()`: on the main thread, `orderFrontRegardless()` then `makeKeyWindow()` on the panel (no app activation). Keep the existing positioning code and the `popsearch://shown` emit. Keep Windows/other platforms on the current `show()` + `set_focus()`.
   - Hiding stays `win.hide()` (orderOut). Check `toggle_window`'s "visible && focused → hide" still works: `win.is_focused()` should reflect the panel's key state; if it doesn't, use the NSWindow `isKeyWindow` on macOS.
   - The Order 2 (previous work file) check `NSRunningApplication::currentApplication().isActive()` will now usually be false because the app no longer activates; that is intended — leave it.
   - Debug logs (`#[cfg(all(debug_assertions, target_os = "macos"))]`, `[popsearch-debug]`): log once after conversion (class name, styleMask bits, level) and on each macOS show (isKeyWindow after makeKeyWindow).
3. `SPEC.md` — one Korean sentence in the macOS notes: macOS에서는 메인 창을 NSPanel(앱을 활성화하지 않는 패널)로 바꿔 전체화면 앱 위에서도 뜨고 키 입력을 받음.

### Constraints
- Do not modify, create, or delete any file not listed above (Cargo.lock may change only by adding an existing-version direct dependency; no version changes).
- Never rewrite or regenerate a whole file; make targeted edits only.
- No Windows behavior change; all new code inside `#[cfg(target_os = "macos")]`.
- Keep existing debug logs and Order 1's space behavior.
- `unsafe` blocks must have a short `// SAFETY:` comment.
- Do not start a GUI instance. Delete any file you created for testing before finishing.

### Done when
- `cargo check --manifest-path src-tauri/Cargo.toml` passes with no new warnings; `cargo test --manifest-path src-tauri/Cargo.toml` passes.
- `git diff src-tauri/Cargo.lock` shows no crate version changes.
- (GUI check by user — all must pass: ① YouTube full screen (Safari/Zen) → hotkey → panel visible over the video, can type Korean, Enter sends; ② Esc hides it and video stays full screen; ③ normal desktop: hotkey shows, click outside hides; ④ in the input, Cmd+C / Cmd+V / Cmd+A / Cmd+Z work; ⑤ Cmd+H history, Cmd+Enter web search still work; ⑥ selected-text capture still works.)

## Review 2
- 판정: PASS
- 확인 내용: cargo check 통과(경고 6개는 Windows 전용 미사용 코드로 기존 것), cargo test 통과. Cargo.lock은 "objc2" 한 줄만 추가. 새 코드는 모두 cfg(macos). tao 0.37.2 소스 확인: TaoWindow ivar는 WindowState{AtomicBool}(align 1, Drop 없음)로 오프셋 456, 우리 u64 ivar도 456(8 정렬 일치)이며 인스턴스 크기 464 동일, 런타임 비교로 불일치 시 변환을 건너뜀. u64는 읽거나 쓰지 않고 Drop 없음 -> drop flag 문제 없음. 변환·표시 모두 run_on_main_thread에서 실행. macOS 표시는 orderFrontRegardless+makeKeyWindow(앱 활성화 없음), 그 외 플랫폼은 기존 show()+set_focus() 유지. 1단계 실행 로그(class=PopSearchPanel, panic 없음) 일치.
- 직접 수정: 없음
- 문서: SPEC.md에 NSPanel 문장 반영됨
- 참고: (1) 변환이 조용히 건너뛰어지면(tao 업데이트로 크기 변동) 전체화면 위 표시가 다시 안 될 수 있음 - 디버그 로그로만 알림. tao 버전 올릴 때 재확인 필요. (2) toggle_window의 "보임 && 포커스 -> 숨김"이 패널에서도 is_focused로 동작하는지는 GUI 확인(③)에서 검증 필요. (3) 사용자 GUI 항목 ①~⑥은 미확인.

## 사용자 확인 (Order 2 이후)
- 맥에서 GUI 확인 ①~⑥ 모두 통과: 유튜브 전체화면 위 표시·한글 입력·전송, Esc 시 전체화면 유지, 바깥 클릭 숨김, Cmd+C/V/A/Z, Cmd+H·Cmd+Enter, 선택 텍스트 가져오기.
- 미션 컨트롤에서 창이 모든 공간에 보이는 것은 CanJoinAllSpaces에 따른 정상 동작으로 두기로 함.
