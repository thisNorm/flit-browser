# Flit Browser 0.5.0 · Windows 개발 미리보기

Windows 11 x64 개발용 소스입니다. [설치 및 GitHub 배포 방법](installer/README.md)

## 사용

- 주소창은 웹 주소와 검색어를 구분한다. `Ctrl+L`, `Alt+D`, `F6`으로 주소창으로 이동하고 `Enter`로 연다. 주소창과 검색 입력에서 `Ctrl+Backspace`·`Ctrl+Delete`로 앞·뒤 단어를 지운다.
- 기본은 **왼쪽 세로 탭**이다. `Ctrl+T`로 새 탭, `Ctrl+W` 또는 탭 오른쪽 ×로 닫기, `Ctrl+Shift+T`로 이번 실행 중 닫은 웹 주소를 다시 연다. `Ctrl+Tab`·`Ctrl+Shift+Tab`으로 다음·이전 탭, `Ctrl+1`~`Ctrl+8`로 해당 탭, `Ctrl+9`로 마지막 탭으로 이동한다.
- 탭을 오른쪽 클릭하면 고정·복제·음소거할 수 있다. 고정 탭은 목록 위에 모인다. 창을 정상 종료하면 최대 128개 탭의 주소, 순서, 고정 상태가 저장된다. 재실행 시 마지막 활성 탭부터 로드하고 나머지는 선택할 때 로드한다. **Continue last tabs on startup**을 끄면 빈 탭으로 시작한다.
- `Ctrl+Shift+A`로 탭을 검색하고 `Enter`로 첫 결과를 연다. 왼쪽 위 버튼이나 `Ctrl+Shift+B`로 패널을 열고 닫는다. 움직이는 도중 다시 눌러 방향을 바꿀 수 있다. Windows에서 애니메이션을 끄면 즉시 전환한다.
- `Ctrl+F`로 페이지를 검색하고 `F3`·`Shift+F3`로 다음·이전 일치 항목으로 이동한다. `Ctrl+F5`로 캐시를 건너뛰어 새로고침하고 `F11`로 전체 화면을 전환한다. `Ctrl++`·`Ctrl+-`·`Ctrl+0`은 페이지 배율을 바꾸거나 초기화한다. 배율은 사이트별로 기억한다.
- 별 버튼 또는 `Ctrl+D`로 현재 웹페이지를 북마크에 저장하고, **Manage bookmarks**에서 목록을 연다. `Ctrl+J`로 다운로드를 확인한다. 메뉴에서 방문 기록, 사이트 권한, 인터넷 사용 기록 삭제 화면, 설정, 비밀번호 관리자, 확장 관리, 인쇄를 열 수 있다.
- Windows 11 22H2 이상에서는 창 프레임에 Mica Alt를 요청한다. 메뉴에서 **Use acrylic window frame**을 선택하면 해당 실행에서 Desktop Acrylic을 요청한다. Windows의 투명 효과와 고대비 설정에 따라 단색이 될 수 있다. 주소창·탭 텍스트와 웹 콘텐츠는 불투명하게 유지한다.
- **About Flit and updates**에서 버전과 업데이트 채널 상태를 확인한다. [자동 업데이트](installer/UPDATES.md)는 서명 키와 배포 저장소를 지정한 빌드에서만 켜진다. 지금 제공한 개발 미리보기에는 실제 업데이트 채널을 연결하지 않았다.
- 기본 프로필은 `%LOCALAPPDATA%\Flit Browser\User Data`에 저장된다. 제거해도 프로필은 남는다. 기존 Chrome·Aside 프로필을 열거나 복사하지 않는다.

## 범위

현재 빌드는 **Rust + Win32 + CEF Chrome Runtime**이다. 빈 탭에는 웹 렌더러를 만들지 않고 주소를 열 때만 Chromium 탭을 만든다. [색과 RAM 설계](DESIGN.md)

Windows의 새 테스트 프로필에서 탭 복원, 북마크 저장·재시작, 배율 변경·재시작, 페이지 이동·찾기, 관리 화면 진입, 방문 기록 저장, HTTP 캐시·쿠키 재사용, 제한된 렌더러 토큰을 확인했다.

확장 설치, 비밀번호 자동완성, 사이트 권한 요청 처리, 데이터 삭제 결과, DevTools, 별도 창·비공개 모드, 충돌 직전 세션 복구, 한국어 IME 조합, Exact 통합, Agent Tabs와 MCP는 이 빌드에서 지원을 확인하지 않았다. 현재는 서명되지 않은 개발 미리보기다.

[기능 비교 및 검증 범위](FEATURES.md) · 0.5.0 실행 검사 · 실제 실행 화면

## 빌드

[Rust CEF 바인딩](https://github.com/tauri-apps/cef-rs/tree/9aa78c450489eddb828eb982eaee6c104a89d348) `156.4.0+156.0.26`, CEF `156.0.26+g8ea0c73` / Chromium `156.0.8078.88`, Rust `1.100.0`을 사용한다. 저장소 루트에서 실행한다.

~~~bash
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS='-C target-feature=+crt-static'
cargo test --release --locked --target x86_64-pc-windows-msvc --lib
cargo build --release --locked --target x86_64-pc-windows-msvc
~~~

툴체인 기록과 [설치 파일 빌드 안내](installer/README.md)를 참고한다. `tests/windows_*.py`는 별도의 테스트 프로필로 실행한다. `tests/windows_shortcuts.py --exe <Flit.exe>`는 잠금이 풀린 Windows에서 실제 키 입력을 검사한다. `tests/windows_memory.py`는 두 버전을 같은 로컬 페이지와 탭 상태로 비교한다.
