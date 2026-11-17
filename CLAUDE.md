# 팝써치 프로젝트 규칙

- 설계 문서: `SPEC.md`. 설계가 바뀌는 작업은 `SPEC.md`도 함께 고친다 (문서 동기화 항목).
- 실데이터: 앱 데이터(히스토리 DB, 설정)는 OS 앱 데이터 폴더에 생긴다. 테스트에서 실제 사용자의 앱 데이터 폴더를 지우거나 덮어쓰지 않는다.
- API 키·토큰을 코드, 로그, 커밋, 작업 파일에 쓰지 않는다.
- 빌드 확인: `npm run build` (프론트), `cargo check --manifest-path src-tauri/Cargo.toml` (Rust).
- 실행: `npm run tauri dev`. GUI 동작(단축키·창)은 실행해서 확인한다.
- 플랫폼별 코드는 `#[cfg(target_os = ...)]`로 나누고, Windows 동작을 먼저 완성한다. macOS 경로도 컴파일되도록 둔다.
