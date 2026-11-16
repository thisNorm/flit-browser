# Flit Browser 설치와 배포

Windows 11 x64에서 `FlitSetup-x64.exe`를 실행하면 내 계정에 Flit Browser가 설치된다. 시작 메뉴에서 **Flit Browser**를 열거나, **설정 → 앱 → 설치된 앱**에서 제거할 수 있다. 제거해도 `%LOCALAPPDATA%\Flit Browser\User Data`의 프로필은 남는다. 이 개발 미리보기는 서명되지 않아 Windows에서 경고를 표시할 수 있다.

설치하면 Flit이 Windows 기본 앱 목록에 HTTP·HTTPS용 브라우저로 등록된다. 기본값은 사용자가 **설정 → 앱 → 기본 앱**에서 직접 고른다. 링크는 일반 창의 새 탭으로 열고, Flit을 처음 실행하는 링크라면 저장된 탭도 함께 복원한다. 저장된 탭은 선택할 때 로드한다.

## 한 파일로 설치

**`Install-Flit.cmd`**를 더블클릭하거나 터미널에서 아래 명령을 실행한다. 같은 폴더에 `FlitSetup-x64.exe`가 있으면 그 파일을 사용한다. 공개 릴리스용 스크립트는 설치 파일이 없으면 GitHub에서 받아 온다. 어느 경우든 SHA-256을 확인한 뒤 설치 화면을 연다.

```powershell
.\Install-Flit.cmd
```

소스에 들어 있는 스크립트는 템플릿이다. 아래 빌드 단계에서 생성된 스크립트와 `FlitSetup-x64.exe`를 같은 폴더에 두고 실행한다. GitHub CLI나 PowerShell 실행 정책 변경은 필요 없다.

설치 파일이 GitHub Releases에 올라온 뒤에는 스크립트만 내려받아 설치할 수 있다.

```powershell
curl.exe -fLO 'https://github.com/thisNorm/flit-browser/releases/latest/download/Install-Flit.cmd'
```

## GitHub에 올리기

1. 빌드하기 전에 `Install-Flit.cmd`의 `FLIT_INSTALL_REPO`에 `thisNorm/flit-browser`를 넣는다. 빌드가 출력 폴더의 스크립트에 설치 EXE의 버전과 SHA-256을 넣는다.
2. GitHub **Releases**에서 빌드한 버전의 릴리스(예: `v0.32.0`)에 출력 폴더의 `Install-Flit.cmd`, `FlitSetup-x64.exe`, `FlitSetup-x64.exe.sha256`를 첨부한다.
3. 공개 후 별도 폴더에 `Install-Flit.cmd`만 내려받아 설치 화면까지 열리는지 확인한다.

설치 파일을 제공하는 최신 정식 릴리스가 생긴 뒤에는 `.\Install-Flit.ps1 -Repository thisNorm/flit-browser -Silent`로 무인 설치할 수 있다. [GitHub 릴리스 다운로드 링크](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)

설치 이후 Flit과 CEF를 함께 갱신하는 [서명된 자동 업데이트 채널](UPDATES.md)은 별도로 설정한다.

## 설치 파일 빌드

1. [README의 빌드 절차](../README.md#빌드)에 따라 `flit.dll`을 빌드한다.
2. [CEF 158.0.3 Windows x64 minimal 배포본](https://cef-builds.spotifycdn.com/cef_binary_158.0.3%2Bg8c29be2%2Bchromium-158.0.8127.18_windows64_beta_minimal.tar.bz2)에서 `Release`의 부트스트랩과 런타임, `Resources`의 리소스·로캘, 루트의 `LICENSE.txt`와 `CREDITS.html`을 한 디렉터리에 모은다. 저장소 루트에서 `tools/New-WindowsBundle.ps1`의 `-CefRuntime`에 이 경로를, `-FlitDll`에 새 DLL을, `-Destination`에 새 번들 경로를 넘긴다. 설치 파일로 배포할 때 번들에 Rust 의존성 고지(`third-party-licenses`)를 포함한다.
3. [Inno Setup 7.1.1](https://github.com/jrsoftware/issrc/releases/tag/is-7_1_1)의 `ISCC.exe` 경로와 번들 경로를 넘긴다. 새 출력 폴더에 설치 EXE, 버전과 SHA-256이 맞는 스크립트, `.sha256` 파일이 함께 생긴다. Windows 서명 등으로 EXE를 수정하면 최종 파일로 SHA-256과 스크립트를 다시 맞춰야 한다.

```powershell
.\installer\Build-Setup.ps1 -IsccPath 'C:\Tools\Inno Setup\ISCC.exe' -BundleDir 'C:\Build\Flit-Browser' -OutputDir 'C:\Build\release'
```
