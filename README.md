# Flit Browser 0.3.0 · Windows 개발 미리보기

Windows 11 x64 개발용 소스입니다. [설치 및 GitHub 배포 방법](installer/README.md)

## 사용

- 주소창은 웹 주소와 검색어를 구분한다. `Ctrl+L`로 주소창으로 이동하고, `Enter`로 연다. 상단에서 뒤로·앞으로·새로고침을 쓸 수 있다.
- 기본은 **왼쪽 세로 탭**이다. `Ctrl+T`로 새 탭, `Ctrl+W` 또는 탭 오른쪽 ×로 닫기, `Ctrl+Shift+T`로 이번 실행 중 닫은 웹 주소를 다시 연다. 탭 목록에 포커스가 있으면 위·아래 방향키로 전환한다.
- 검색창은 열린 탭의 제목과 URL을 찾는다. `Ctrl+Shift+A`로 열고 `Enter`로 첫 결과로 이동한다. 왼쪽 위 버튼으로 사이드바를 접을 수 있다.
- 별 버튼은 Chromium 북마크 관리 화면으로 이동한다. 오른쪽 메뉴에는 방문 기록, 다운로드, 설정, 비밀번호 관리자, 확장 관리, 페이지 내 찾기, 인쇄가 있다. `Ctrl+F`는 페이지 검색이다.
- Windows 11 22H2 이상에서는 창 프레임에 Mica Alt를 요청한다. 메뉴에서 **Use acrylic window frame**을 선택하면 해당 실행에서 Desktop Acrylic을 요청한다. Windows의 투명 효과와 고대비 설정에 따라 단색이 될 수 있다. 주소창·탭 텍스트와 웹 콘텐츠는 불투명하게 유지한다.
- **About Flit and updates**에서 버전과 업데이트 채널 상태를 확인한다. [자동 업데이트](installer/UPDATES.md)는 서명 키와 배포 저장소를 지정한 빌드에서만 켜진다. 지금 제공한 개발 미리보기에는 실제 업데이트 채널을 연결하지 않았다.
- 기본 프로필은 `%LOCALAPPDATA%\Flit Browser\User Data`에 저장된다. 제거해도 프로필은 남는다. 기존 Chrome·Aside 프로필을 열거나 복사하지 않는다.

## 범위

현재 빌드는 **Rust + Win32 + CEF Chrome Runtime**이다. 빈 탭에는 웹 렌더러를 만들지 않고 주소를 열 때만 Chromium 탭을 만든다. [색과 RAM 설계](DESIGN.md)

Windows에서 세로 탭 배치, 접기, 검색, 비활성 탭 닫기, 페이지 이동·찾기, 네이티브 관리 화면과 방문 기록 저장, HTTP 캐시·쿠키 재사용, 제한된 렌더러 토큰을 확인했다. 관리 화면이 열리는 것을 실제 다운로드 완료·확장 설치·비밀번호 자동완성까지 검증했다는 뜻으로 해석하지 않는다.

확대·축소, DevTools, 별도 창·비공개 모드, 충돌 후 세션 복구, 한국어 IME 조합, Exact 통합, Agent Tabs와 MCP는 이 빌드에서 지원을 확인하지 않았다. 현재는 서명되지 않은 개발 미리보기다.

0.3.0 실행 검사 · 관리 화면 검사 · 실제 실행 화면

## 빌드

[Rust CEF 바인딩](https://github.com/tauri-apps/cef-rs/tree/9aa78c450489eddb828eb982eaee6c104a89d348) `156.4.0+156.0.26`, CEF `156.0.26+g8ea0c73` / Chromium `156.0.8078.88`, Rust `1.100.0`을 사용한다. 저장소 루트에서 실행한다.

~~~bash
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS='-C target-feature=+crt-static'
cargo test --release --locked --target x86_64-pc-windows-msvc --lib
cargo build --release --locked --target x86_64-pc-windows-msvc
~~~

툴체인 기록과 [설치 파일 빌드 안내](installer/README.md)를 참고한다. `tests/windows_smoke.py`와 `tests/windows_builtins.py`는 별도의 테스트 프로필로 실행한다. `tests/windows_memory.py`는 두 버전을 같은 로컬 페이지와 탭 상태로 비교한다.
