# ClipStash

<img src="res/icon-preview.png" width="64" align="right" alt="ClipStash 아이콘">

**Windows용 가볍고 빠른 클립보드 기록 관리자.** Rust 로 Win32 API 를 직접 써서 만들었고,
실행 파일 하나(약 700KB)에 설치 없이 돌아갑니다.

복사(Ctrl+C)한 텍스트, 이미지, 파일을 자동으로 모아 두었다가 단축키(기본 **Ctrl+Shift+V**)로
목록을 띄워 골라 붙여넣을 수 있습니다.

---

## 주요 기능

| | |
|---|---|
| 📋 **클립보드 기록** | 텍스트, 이미지, 탐색기에서 복사한 파일을 자동 기록. 같은 내용은 중복 없이 맨 위로 올라옴 |
| ⌨️ **단축키 팝업** | 아무 프로그램에서나 단축키 → 입력 커서(또는 마우스) 옆에 목록이 뜨고, 고르면 **원래 창에 바로 붙여넣기** |
| 🔍 **즉시 검색 + 초성 검색** | 입력하는 대로 걸러짐. 여러 단어 AND 검색, 대소문자 무시, **`ㅎㄱ` → `한국`** 같은 한글 초성 검색 |
| ⭐ **고정(즐겨찾기)** | 자주 쓰는 항목은 Ctrl+P 로 고정. 고정한 항목은 개수 제한에 걸려도 지워지지 않음 |
| 📝 **템플릿(상용구)** | 날짜·시간·요일, 지금 클립보드 내용 `{clipboard}`, 붙여넣은 뒤 커서 위치 `{cursor}` 같은 자리표시자 지원 |
| ✂️ **서식 없이 / 바꿔서 붙여넣기** | Shift+Enter = 텍스트만. 우클릭 → 대문자·소문자·공백 정리·한 줄로 합치기·빈 줄 제거·인용(`> `) |
| 🖼️ **미리보기** | 선택한 항목의 전체 텍스트, 이미지 썸네일, 파일 목록, 복사한 프로그램과 글자 수 표시 |
| 🔒 **민감한 앱 제외** | 비밀번호 관리자(KeePass, 1Password, Bitwarden 등)에서 복사한 내용은 기록 안 함. 앱이 "기록 금지" 표시를 붙인 복사도 자동으로 무시 |
| 🧭 **트레이 아이콘** | 일시정지, Windows 시작 시 자동 실행, 설정 열기, 기록 지우기 |
| 💾 **자동 저장** | 기록은 껐다 켜도 남음. 저장 도중 꺼져도 파일이 깨지지 않게 안전하게 저장 |
| 🧳 **휴대용 모드** | exe 옆에 `data` 폴더를 만들면 그 안에 모든 데이터를 저장(USB 에 넣고 다니기) |
| 🖥️ **고해상도 모니터** | 모니터별 DPI 대응(PerMonitorV2). 4K/노트북 배율에서도 선명 |

## 설치와 실행

### 바로 받기
GitHub 의 **Actions** 탭 → 가장 최근 성공한 **CI** 실행 → 아래 **Artifacts** 에서
`clipstash-windows-x64` 를 받아 압축을 풀고 `clipstash.exe` 를 실행하세요.
창은 뜨지 않고 작업 표시줄 오른쪽 아래(트레이)에 아이콘이 생깁니다.

> 처음 실행할 때 Windows SmartScreen 이 "알 수 없는 게시자" 경고를 띄울 수 있습니다.
> 서명하지 않은 프로그램이라 그렇습니다. **추가 정보 → 실행** 을 누르면 됩니다.

### 직접 빌드하기 (Windows)
1. [Rust 설치](https://rustup.rs) (설치 중 나오는 Visual Studio C++ Build Tools 도 함께 설치)
2. 이 저장소를 받고 폴더에서:
   ```powershell
   cargo build --release
   ```
3. `target\release\clipstash.exe` 가 만들어집니다.

### Linux/macOS 에서 Windows 용으로 크로스 빌드
```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install gcc-mingw-w64-x86-64      # macOS: brew install mingw-w64
cargo build --release --target x86_64-pc-windows-gnu
```

## 사용법

1. ClipStash 를 켜 두고 평소처럼 복사(Ctrl+C)합니다.
2. 붙여넣고 싶은 곳에서 **Ctrl+Shift+V** → 목록에서 고르고 **Enter**.

### 팝업 단축키

| 키 | 동작 |
|---|---|
| 글자 입력 | 바로 검색 (초성 검색 가능) |
| ↑ ↓ / PgUp PgDn / Ctrl+Home End | 항목 이동 |
| **Enter** / 더블클릭 | 붙여넣기 |
| **Shift+Enter** | 서식 없이 텍스트만 붙여넣기 |
| **Ctrl+1 … Ctrl+9, Ctrl+0** | 1~10번째 항목 바로 붙여넣기 |
| **Tab** / Shift+Tab | 기록 → 고정 → 템플릿 탭 전환 |
| Ctrl+P | 고정 / 고정 해제 |
| Del (검색어가 없을 때) · Shift+Del | 항목 삭제 |
| Ctrl+T | 선택한 항목을 템플릿으로 저장 |
| Ctrl+C (검색어 선택이 없을 때) | 붙여넣지 않고 클립보드에 복사만 |
| Ctrl+E | 설정 파일 열기 |
| 우클릭 / 메뉴 키 / Shift+F10 | 더 보기 메뉴 (바꿔서 붙여넣기 등) |
| Ctrl+Backspace | 검색어 지우기 |
| Esc | 검색어 지우기 → 한 번 더 누르면 닫기 |

트레이 아이콘을 **왼쪽 클릭**하면 팝업이 마우스 위치에 뜹니다(이때는 붙여넣지 않고 클립보드에 넣기만 합니다).
**오른쪽 클릭**하면 메뉴가 나옵니다.

## 설정

트레이 메뉴 → **설정 파일 열기** (또는 팝업에서 Ctrl+E). 메모장으로 `config.toml` 이 열립니다.
파일에 항목마다 한국어 설명이 달려 있고, 저장하면 다음에 팝업을 열 때 자동으로 적용됩니다.

주요 항목:

```toml
hotkey = "Ctrl+Shift+V"        # 예: "Alt+C", "Ctrl+`", "F9"
max_items = 300                # 보관 개수(고정 항목 제외)
capture_images = true          # 이미지도 기록
paste_after_select = true      # 고르면 바로 붙여넣기(false 면 복사만)
plain_text_by_default = false  # 항상 서식 없이 붙여넣기
popup_position = "caret"       # "caret" | "mouse" | "center"
excluded_apps = ["keepass.exe", "1password.exe", "bitwarden.exe"]

[[templates]]
name = "메일 맺음말"
group = "업무"
text = '''
{cursor}

감사합니다.
'''
```

### 템플릿 자리표시자

| 쓰기 | 결과 예 |
|---|---|
| `{date}` · `{time}` · `{datetime}` | 2026-11-24 · 15:35 · 2026-11-24 15:35 |
| `{date:yyyy년 M월 d일 (E)}` | 2026년 11월 24일 (화) |
| `{time:HH시 mm분}` | 15시 35분 |
| `{weekday}` | 화 |
| `{clipboard}` | 지금 클립보드에 있는 텍스트 |
| `{cursor}` | 붙여넣은 뒤 커서가 이 자리로 이동 |
| `{{` `}}` | 중괄호 문자 그대로 |

형식 문자: `yyyy` `yy` `MM` `M` `dd` `d` `HH` `H` `mm` `ss` `E`(요일)

## 데이터와 개인정보

- 데이터 위치: `%APPDATA%\ClipStash\` (휴대용 모드면 exe 옆 `data\`)
  - `config.toml` 설정 · `history.json` 기록 · `images\` 이미지
- **모든 데이터는 내 컴퓨터에만 저장됩니다.** 인터넷 연결을 전혀 쓰지 않습니다.
- `history.json` 은 암호화되지 않은 일반 파일입니다. 공용 컴퓨터라면 트레이 메뉴의
  **일시정지** 나 **기록 모두 지우기** 를 활용하세요.
- 다음 경우는 기록하지 않습니다:
  - `excluded_apps` 에 있는 프로그램에서 복사한 것
  - 복사한 프로그램이 "클립보드 기록 금지" 표시(`ExcludeClipboardContentFromMonitorProcessing`,
    `CanIncludeInClipboardHistory = 0` 등)를 붙인 것 — 대부분의 비밀번호 관리자가 이렇게 합니다
  - 일시정지 중일 때

## 테스트 상태 (중요)

이 프로그램은 Linux 클라우드 환경에서 만들어졌습니다. 확인된 것과 아직 확인하지 못한 것은 이렇습니다.

**확인됨**
- 핵심 로직(기록·중복 처리·고정·검색·초성 검색·템플릿·텍스트 변환·설정 파일·저장/복구·BMP 변환·단축키 해석)
  단위 테스트 30개 통과 (`cargo test --lib`)
- Windows 용 실행 파일 크로스 컴파일 성공, clippy 경고 없음
- GitHub Actions 의 실제 Windows 환경에서: 빌드, 테스트, 실행 후 텍스트를 복사하면 `history.json` 에
  기록되는지, 중복 실행 방지가 되는지 자동 확인 (`.github/workflows/ci.yml`)

**실제 Windows 화면에서 아직 확인하지 못함** — 직접 써 보시고 이상하면 알려주세요
- 팝업 창의 모양(글꼴, 간격, 색), 배율 125%/150% 모니터에서의 크기
- 단축키로 팝업 → Enter 로 원래 창에 붙여넣기가 모든 프로그램에서 잘 되는지
  (특히 관리자 권한으로 실행된 프로그램에는 Windows 보안 정책상 붙여넣기 키를 보낼 수 없습니다)
- 입력 커서 옆에 팝업 띄우기: 메모장 같은 기본 프로그램은 되지만, 크롬·VS Code 등은 커서 위치를
  알려주지 않아 마우스 위치에 뜹니다
- 이미지 미리보기와 이미지 붙여넣기, 파일 복사/붙여넣기
- 트레이 아이콘, 풍선 알림, Windows 시작 시 자동 실행

## 코드 구조

```
src/
  lib.rs          핵심 로직(운영체제와 무관, Linux 에서도 테스트 가능)
    history.rs    기록 목록: 추가·중복·고정·개수 제한·검색
    search.rs     검색 + 한글 초성 검색
    template.rs   템플릿 자리표시자 치환
    transform.rs  붙여넣기 전 텍스트 변환
    config.rs     설정 파일(기본값과 설명 포함)
    storage.rs    저장/불러오기(안전한 쓰기, 손상 복구)
    hotkey.rs     "Ctrl+Shift+V" → Windows 키 코드
    bmp.rs        클립보드 이미지(DIB) ↔ BMP 파일
  main.rs
  win/            Windows 전용
    mod.rs        메인: 클립보드 감시, 전역 단축키, 트레이, 붙여넣기 흐름
    popup.rs      팝업 창(탭·검색·직접 그린 목록·미리보기·우클릭 메뉴)
    clip.rs       클립보드 읽기/쓰기(텍스트·HTML·이미지·파일)
    paste.rs      원래 창으로 돌아가 Ctrl+V 보내기
    util.rs       자잘한 도우미(경로, 프로세스 이름, 자동 실행 등)
res/              아이콘, 매니페스트(DPI·공용 컨트롤), 버전 정보
tools/make_icon.py 아이콘 그리는 스크립트
```

## 앞으로 해 볼 만한 것
- 설정 창(GUI) — 지금은 설정 파일을 직접 고칩니다
- 다크 모드
- 기록 암호화 저장
- 항목별 단축키(템플릿을 단축키 하나로 바로 붙여넣기)

## 라이선스
MIT
