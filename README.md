# Flit Browser 0.8.0 · Windows 개발 미리보기

Windows 11 x64 개발용 소스입니다. [설치 및 GitHub 배포 방법](installer/README.md)

## 사용

- 주소창은 웹 주소와 검색어를 구분한다. `Ctrl+L`, `Alt+D`, `F6`으로 주소창으로 이동하고 `Enter`로 연다. 조합 입력 중에는 입력기가 `Enter`·`Esc` 등을 먼저 처리한다. 확정된 주소창·검색 텍스트는 `Ctrl+Backspace`·`Ctrl+Delete`로 앞·뒤 단어를 지운다.
- 기본은 **왼쪽 세로 탭**이다. `Ctrl+T`로 새 탭, `Ctrl+W`·`Ctrl+F4` 또는 탭 오른쪽 ×로 닫기, `Ctrl+Shift+T`로 이번 실행 중 닫은 웹 주소를 다시 연다. `Ctrl+Tab`·`Ctrl+Shift+Tab`으로 다음·이전 탭, `Ctrl+1`~`Ctrl+8`로 해당 탭, `Ctrl+9`로 마지막 탭으로 이동한다.
- 탭을 오른쪽 클릭하면 고정·복제·음소거할 수 있다. 고정 탭은 목록 위에 모인다. **Unload tab to free memory…**는 선택하지 않은 탭의 페이지를 닫고 주소·제목·고정 상태를 남긴다. 다시 선택하면 페이지를 로드한다.
- 마지막으로 사용한 일반 창의 탭 주소·순서·고정 상태를 최대 128개까지 저장한다. 재실행 때 마지막 활성 탭만 먼저 로드한다. 갑작스럽게 종료된 경우 마지막 저장 시점의 탭을 복원한다. **Continue last tabs on startup**을 끄면 빈 탭으로 시작한다.
- `Ctrl+N`은 새 일반 창, `Ctrl+Shift+N`은 비공개 창, `Ctrl+Shift+W`는 현재 창 닫기다. 비공개 창은 일반 창과 사이트 데이터를 공유하지 않는다. 마지막 비공개 창을 닫으면 비공개 사이트 데이터가 사라진다. 내려받은 파일은 남는다.
- 사용자가 클릭해서 여는 팝업은 원래 창과 통신할 수 있는 별도 창으로 연다. 최상위 페이지가 카메라·마이크를 요청하면 사이트 주소를 표시하고, **Allow this time**을 선택한 요청만 허용한다. 사용 중인 장치는 세로 탭과 활성 창 제목에 표시한다. 알림·화면 공유 등은 Chromium의 기본 확인창을 사용한다.
- `Ctrl+Shift+A`로 탭을 검색하고 `Enter`로 첫 결과를 연다. 왼쪽 위 버튼이나 `Ctrl+Shift+B`로 패널을 열고 닫는다. 움직이는 도중 다시 눌러 방향을 바꿀 수 있다. Windows에서 애니메이션을 끄면 즉시 전환한다.
- `Ctrl+F`로 페이지를 검색하고 `F3`·`Shift+F3`로 다음·이전 일치 항목으로 이동한다. `Ctrl+F5`로 캐시를 건너뛰어 새로고침하고 `F11`로 전체 화면을 전환한다. `Ctrl++`·`Ctrl+-`·`Ctrl+0`은 페이지 배율을 바꾸거나 초기화한다. 배율은 사이트별로 기억한다.
- 네이버처럼 넓은 화면을 전제로 만든 사이트가 좁은 창에서 잘리면 창을 최대화하거나, 왼쪽 패널을 접거나, `Ctrl+-`로 해당 사이트의 배율을 낮춘다.
- 별 버튼 또는 `Ctrl+D`로 현재 웹페이지를 북마크에 저장하고, `Ctrl+Shift+O`로 북마크 관리 화면을 연다. `Ctrl+J`로 다운로드를 확인한다. 전송이 진행 중인 원본 탭이나 창을 닫으면 확인 메시지가 나타난다.
- 주소창 왼쪽 버튼으로 현재 사이트의 연결 상태를 보고, 일반 창에서 해당 사이트 설정을 바로 열 수 있다. `Ctrl+Shift+Delete`는 인터넷 사용 기록 삭제다. 일반 설정과 기록 삭제는 별도 창에서 열리므로 현재 탭을 그대로 볼 수 있다.
- `F12`·`Ctrl+Shift+I`로 현재 탭의 개발자 도구를 열고 닫으며, `Ctrl+Shift+J`로 콘솔을 연다. 메뉴에서 방문 기록, 비밀번호 관리자, 확장 관리, 인쇄도 열 수 있다.
- Windows 11 22H2 이상에서는 창 프레임에 Mica Alt를 요청한다. 메뉴에서 **Use acrylic window frame**을 선택하면 해당 실행에서 Desktop Acrylic을 요청한다. Windows의 투명 효과와 고대비 설정에 따라 단색이 될 수 있다. 주소창·탭 텍스트와 웹 콘텐츠는 불투명하게 유지한다.
- **About Flit and updates**에서 버전과 업데이트 채널 상태를 확인한다. [자동 업데이트](installer/UPDATES.md)는 서명 키와 배포 저장소를 지정한 빌드에서만 켜진다. 지금 제공한 개발 미리보기에는 실제 업데이트 채널을 연결하지 않았다.
- 기본 프로필은 `%LOCALAPPDATA%\Flit Browser\User Data`에 저장된다. 제거해도 프로필은 남는다. 기존 Chrome·Aside 프로필을 열거나 복사하지 않는다.

## 범위

현재 빌드는 **Rust + Win32 + CEF Chrome Runtime**이다. 빈 탭에는 웹 렌더러를 만들지 않고 주소를 열 때만 Chromium 탭을 만든다. 이번 빌드는 **CEF 158.0.3 베타**를 사용한다. [색과 RAM 설계](DESIGN.md) · 엔진 확인 기록

0.8.0은 Windows 조합 메시지가 들어왔을 때 주소창의 초안을 보존하고, 조합이 끝난 뒤에만 다음 Enter로 이동하는지 검사했다. 네이버 홈은 150% 디스플레이에서 패널을 접기 전후 실제 페이지 폭을 측정하고, 같은 CSS 너비의 Aside와 비교했다. 버전별 검증 범위는 [기능 비교](FEATURES.md)에 정리했다.

Chrome 웹 스토어, 패스키, 실제 사이트 로그인, Microsoft 한국어 IME로 직접 타이핑하는 흐름, Exact 통합, Agent Tabs와 MCP는 아직 확인하지 않았다. 카메라·마이크 확인창은 가상 장치로 검사했으며 알림 허용·실제 화면 공유는 검증하지 않았다. 복원은 URL을 다시 여는 기능이며, 페이지 내부 입력이나 뒤로 가기 이력은 복원하지 않는다. 현재는 서명되지 않은 개발 미리보기다.

[기능 비교 및 검증 범위](FEATURES.md) · 0.8.0 실행 검사 · 입력과 네이버 표시 검사

## 빌드

[Rust CEF 바인딩](https://github.com/tauri-apps/cef-rs/tree/9aa78c450489eddb828eb982eaee6c104a89d348) `156.4.0+156.0.26`과 Rust `1.100.0`으로 API `15600`을 대상으로 컴파일한다. 배포할 때는 동일한 API를 지원하는 CEF `158.0.3+g8c29be2` / Chromium `158.0.8127.18`의 Windows x64 부트스트랩과 런타임을 묶는다. 저장소 루트에서 실행한다.

~~~bash
export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS='-C target-feature=+crt-static'
cargo test --release --locked --target x86_64-pc-windows-msvc --lib
cargo build --release --locked --target x86_64-pc-windows-msvc
~~~

툴체인 기록과 [설치 파일 빌드 안내](installer/README.md)를 참고한다. `tests/windows_*.py`는 별도의 테스트 프로필로 실행한다. `tests/windows_shortcuts.py --exe <Flit.exe>`는 잠금이 풀린 Windows에서 실제 키 입력을 검사한다. `tests/windows_memory.py`는 두 버전을 같은 로컬 페이지와 탭 상태로 비교한다.
