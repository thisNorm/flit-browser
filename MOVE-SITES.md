# 파일을 내보내지 않고 사이트와 북마크 옮기기

Flit의 일반 창에서 **⋯ → Move sites and bookmarks**를 연다. [Chrome](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md) 또는 [Edge](https://learn.microsoft.com/en-us/deployedge/edge-learnmore-create-user-directory-vars)의 표준 Windows 위치에 북마크를 둔 로컬 프로필을 선택할 수 있다. 프로필이 둘 이상이면 `Default`나 `Profile N`을 고른다.

- **북마크와 폴더 가져오기:** **Import Chrome bookmarks and folders…** 또는 **Import Edge bookmarks and folders…**에서 개수와 대상 폴더를 확인한다. 가져온 북마크는 **Other bookmarks → From Chrome - Default**와 같은 출처별 폴더에서 볼 수 있다. 기존 항목은 그대로 두고, 다시 가져올 때 같은 폴더에 같은 주소와 제목이 있으면 건너뛴다.
- **자주 쓰는 사이트 준비:** **From Chrome bookmarks…** 또는 **From Edge bookmarks…**를 고른다. 사이트 주소를 확인하고 승인하면 최대 12개의 새 탭이 세로 탭에 추가된다. 이미 열린 사이트는 건너뛰고, 추가한 탭은 하나씩 선택할 때 로드된다.
- **Aside 등에서 복사한 링크:** 사이트 링크들을 텍스트로 복사하고 **From copied site links…**를 고른다. 이 항목을 선택할 때만 클립보드 텍스트를 읽고, 확인창에서 사이트 주소를 검토할 수 있다.
- **다른 브라우저의 북마크:** 북마크를 HTML 파일로 내보낼 수 있다면 Flit의 **⋯ → Import bookmarks from HTML…**에서 파일을 고른다.

사이트 준비는 경로·검색어·앵커를 제외한 사이트 주소만 사용한다. 북마크 가져오기는 각 사이트의 웹 주소, 제목, 폴더를 옮긴다. 원본 브라우저의 북마크, 비밀번호, 쿠키, 로그인 세션을 변경하지 않는다. 로그인은 필요한 사이트에서 직접 진행한다.

0.22.0은 Windows에서 테스트용 Chrome·Edge 프로필로 검증했다. 북마크 검증은 취소, 중첩 폴더와 한글 제목, 재시작, 중복 없는 재가져오기를 기록한다. 사이트 탭 검증은 이미 열린 사이트 제외, 최대 12개 제한, 선택 후 첫 요청을 기록한다.
