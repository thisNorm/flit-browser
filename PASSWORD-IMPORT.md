# 다른 브라우저에 저장한 비밀번호 옮기기

1. 원래 브라우저에서 저장한 사용자 이름과 비밀번호를 CSV로 내보낸다. [Chrome](https://support.google.com/chrome/answer/13068232?hl=ko), [Edge](https://support.microsoft.com/en-us/edge/export-passwords-in-microsoft-edge), [Firefox](https://support.mozilla.org/en-US/kb/export-login-data-firefox)의 공식 안내를 참조한다. 다른 도구도 `url`, `username`, `password` 열이 있는 호환 CSV를 제공하면 사용할 수 있다.
2. Flit의 일반 창에서 **⋯ → Import passwords from CSV…**를 선택한다. 비밀번호 관리자의 **Import passwords → Select file**을 눌러 파일을 고른다.
3. 가져오기 결과를 확인한 뒤 **Passwords** 목록을 연다. 가져온 항목은 이 Flit 프로필에 저장되며 해당 사이트의 로그인 폼에서 자동완성 후보로 쓸 수 있다.

내보낸 CSV는 평문 파일이다. 업로드하거나 다른 사람에게 보내지 말고, 가져오기를 확인한 뒤 원본 브라우저에서는 계정을 그대로 두고 임시 CSV만 지운다.

활성 로그인 세션, 쿠키, 패스키, 일회용 인증 코드는 이전하지 않는다. 가져온 항목은 일반 비밀번호 관리자에 들어가고 Flit Vault나 AI 작업 탭에는 자동으로 전달되지 않는다. [Aside Vault의 공식 안내](https://docs.aside.com/help/password-manager)에서는 CSV 내보내기 경로를 확인하지 못했다. Aside에서 호환 파일을 내보낼 수 있는 경우에만 이 절차로 옮길 수 있다.

0.21.0에서 Chromium 계열 형식과 Firefox 추가 열이 있는 형식의 가짜 CSV를 가져왔다. CSV를 치우고 Flit을 다시 실행한 뒤에도 항목이 남았고, 로컬 로그인 폼의 두 필드가 자동완성됐다. 실제 계정이나 다른 브라우저의 비밀번호 파일은 읽지 않았다.
