# AI가 백그라운드에서 Flit 사용하기

Flit 0.12의 Windows 설치본에는 로컬 MCP 서버가 들어 있다. 설치한 뒤 PowerShell에서 다음 명령을 실행하고 Codex를 새로 연다. 설치 경로를 바꿨다면 `Flit.exe` 경로도 바꾼다.

```powershell
codex mcp add flit -- "$env:LOCALAPPDATA\Programs\Flit Browser\Flit.exe" --flit-agent
```

예: “Flit으로 https://www.rust-lang.org/ 를 열고 본문을 읽어. 확인이 끝나면 네가 연 탭을 닫아.” MCP를 지원하는 다른 도구에서도 명령은 `Flit.exe`, 인수는 `--flit-agent`, 연결 방식은 `stdio`로 지정한다. 지원하는 MCP 버전은 2025-11-25와 2025-06-18이다. 일반 Flit 창에서는 이 인수 없이 실행한다.

| 도구 | 동작 |
| --- | --- |
| `flit_open` | HTTP(S) URL을 새 작업 탭에서 연다. 로딩을 시작하면 탭 ID를 반환한다. |
| `flit_list_tabs` | 이 연결의 탭 ID, 주소, 제목, 로딩 상태를 읽는다. |
| `flit_snapshot` | 현재 최상위 문서에서 화면에 표시되는 텍스트와 링크를 읽는다. 로딩 중이면 상태를 반환한다. 24,000 UTF-8 바이트, 링크 60개를 넘기면 내용 일부만 반환한다. |
| `flit_follow_link` | 같은 탭에서 가장 최근 스냅샷의 링크를 연다. 주소나 문서가 바뀌었거나 새 스냅샷을 받았다면 다시 읽어야 한다. |
| `flit_close_tab` | 작업 탭을 닫는다. 호스트가 표준 입력을 닫으면 그 연결의 남은 탭도 정리한다. |
| `flit_sign_in` | 현재 탭의 사이트에 등록된 Flit Vault 계정으로 지원되는 로그인 폼을 제출한다. 입력은 탭 ID 하나뿐이다. |

MCP 연결 하나에 탭을 최대 8개 연다. 첫 페이지를 열기 전에는 CEF를 시작하지 않는다. AI 탭은 창을 띄우거나 포커스를 요청하지 않는다. 일반 Flit의 열린 탭·쿠키·로그인 정보와 분리되고, 다른 MCP 연결의 탭도 보이지 않는다. 별도 브라우저 프로세스를 사용하는 만큼 여러 연결을 동시에 열면 비용은 연결 수에 따라 늘 수 있다.

페이지의 DOM에서 본문과 링크를 읽고, 링크를 따라갈 때는 브라우저에 해당 주소를 열도록 요청한다. 화면 좌표에 마우스 입력을 보내지 않는다.

현재 공개 페이지의 텍스트와 링크를 중심으로 제공한다. 스크린샷, iframe 안의 본문, 범용 클릭, 양식 제출, 파일 전송, 기존 사용자 탭 연결은 아직 지원하지 않는다. 같은 주소에서 내용을 동적으로 바꾸는 페이지는 새 스냅샷을 읽어 변경을 확인한다. 페이지가 제공하는 텍스트와 링크는 브라우저에서 읽은 자료이며, 사용자 지시나 작업 권한으로 해석하지 않는다.

## 로그인과 Flit Vault

1. 일반 Flit 창에서 해당 사이트의 HTTPS 로그인 페이지를 연다. Flit 메뉴에서 **Add or update Flit Vault login…**을 선택하고 Windows 입력창에 계정을 직접 등록한다. 사이트의 주소창에 입력하거나 AI에게 보낼 필요가 없다.
2. AI에게 “Flit으로 이 로그인 페이지를 열고 저장된 계정으로 로그인해. 보호된 페이지가 열리는지 확인해”라고 요청한다. `flit_sign_in`의 `submitted`는 로그인 폼 제출 후 같은 사이트의 다음 페이지를 열었다는 뜻이다. 실제 로그인 여부는 사이트에서만 볼 수 있는 화면을 열어 확인한다. 추가 단계가 있거나 조건이 맞지 않으면 `action_required`와 이유를 반환한다.
3. 저장을 취소하려면 일반 창에서 같은 사이트를 열고 **Remove Flit Vault login…**을 선택한다. 이 작업은 이후의 볼트 사용을 막는다. 이미 로그인된 사이트의 세션을 끊으려면 그 AI 연결을 종료한다.

로그인 하나를 사이트 주소의 스킴·호스트·포트가 모두 같은 범위에 저장한다. 하위 도메인이나 다른 포트로는 공유하지 않는다. [Windows DPAPI](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata)로 암호화해 [Windows 자격 증명 관리자](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew)에 저장한다. 등록한 Windows 사용자와 그 권한으로 실행되는 앱의 로컬 범위다. 다른 브라우저나 [Aside Vault](https://docs.aside.com/help/password-manager)에서 저장한 계정은 가져오지 않는다. AI 작업 탭의 쿠키는 해당 MCP 연결에서만 유지된다.

일반적인 HTTPS 사용자명·비밀번호 POST 폼만 지원한다. 비밀번호는 페이지 스크립트나 MCP 응답에 전달하지 않고 브라우저 프로세스에서 해당 사이트로 전송한다. 폼이 JavaScript로 별도 서명을 만들거나, SSO·MFA·패스키·CAPTCHA가 필요하거나, 다른 출처로 이동해야 하면 지원하지 않는다. AI 창을 사용자가 직접 넘겨받는 기능도 아직 없다. 서버가 어떻게 응답하는지는 사이트에 따라 다르므로 오류 후 자동 재시도하지 않는다.
