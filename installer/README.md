# Flit Browser 설치와 배포

Windows 11 x64에서 `FlitSetup-x64.exe`를 실행하면 내 계정에 Flit Browser가 설치된다. 시작 메뉴에서 **Flit Browser**를 열거나, **설정 → 앱 → 설치된 앱**에서 제거할 수 있다. 제거해도 `%LOCALAPPDATA%\Flit Browser\User Data`의 프로필은 남는다. 이 개발 미리보기는 서명되지 않아 Windows에서 경고를 표시할 수 있다.

## 한 파일로 설치

**`Install-Flit.cmd`**를 더블클릭하거나 터미널에서 아래 명령을 실행한다. 같은 폴더에 `FlitSetup-x64.exe`가 있으면 그 파일을 사용한다. 공개 릴리스용 스크립트는 설치 파일이 없으면 GitHub에서 받아 온다. 어느 경우든 SHA-256을 확인한 뒤 설치 화면을 연다.

```powershell
.\Install-Flit.cmd
```

현재 제공한 스크립트는 저장소 설정 전이므로 `FlitSetup-x64.exe`와 같은 폴더에서 실행한다. GitHub CLI나 PowerShell 실행 정책 변경은 필요 없다.

공개 후에는 GitHub 릴리스에서 스크립트만 받아도 된다. 정식 릴리스의 최신 스크립트를 명령어로 받으려면 실제 `OWNER/REPO`로 바꿔 실행한다.

```powershell
curl.exe -fLO 'https://github.com/OWNER/REPO/releases/latest/download/Install-Flit.cmd'
```

## GitHub에 올리기

1. `Install-Flit.cmd`의 `FLIT_INSTALL_REPO`에 공개 저장소의 `OWNER/REPO`를 넣는다. 새 버전을 낼 때는 `FLIT_INSTALL_TAG`와 설치 EXE의 SHA-256도 같이 갱신한다.
2. GitHub **Releases**에서 `v0.5.0` 릴리스에 같은 버전의 `Install-Flit.cmd`, `FlitSetup-x64.exe`, `FlitSetup-x64.exe.sha256`를 첨부한다.
3. 공개 후 별도 폴더에 `Install-Flit.cmd`만 내려받아 설치 화면까지 열리는지 확인한다.

최신 정식 릴리스를 찾아 무인 설치하려면 `.\Install-Flit.ps1 -Repository OWNER/REPO -Silent`를 사용한다. [GitHub 릴리스 다운로드 링크](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)

설치 이후 Flit과 CEF를 함께 갱신하는 [서명된 자동 업데이트 채널](UPDATES.md)은 별도로 설정한다.

## 설치 파일 빌드

1. 툴체인 기록의 Rust x64/CEF 환경에서 `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS`를 `-C target-feature=+crt-static`으로 설정한 뒤 `cargo build --release --locked --target x86_64-pc-windows-msvc`를 실행한다.
2. `tools/New-WindowsBundle.ps1`에 CEF 배포 디렉터리와 새 `flit.dll`을 넘겨 앱 번들을 만든다. CEF 라이선스와 Rust 의존성 고지(`third-party-licenses`)를 포함한다. 사용자 프로필과 도구 캐시는 넣지 않는다.
3. [Inno Setup 7.1.1](https://github.com/jrsoftware/issrc/releases/tag/is-7_1_1)의 `ISCC.exe` 경로와 번들 경로를 넘긴다. 빌드 결과의 SHA-256은 `Get-FileHash -Algorithm SHA256`으로 계산한다.

```powershell
.\Build-Setup.ps1 -IsccPath 'C:\Tools\Inno Setup\ISCC.exe' -BundleDir 'C:\Build\Flit-Browser' -OutputDir 'C:\Build\release'
```

초기 설치·제거와 브라우저 실행 검증 · 초기 PowerShell 스크립트 검증
