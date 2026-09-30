# NIKH

판례랑 특허 검색하다 빡쳐서 주말동안 만듦. 별 대단한건 아닐거임.

의뢰인 상황을 문장으로 적으면 가장 비슷한 판례·해석례·심판례·특허 공보가 유사도 순으로 나옵니다.
오프라인 · 생성 0 · 기록 0. 다운로드와 설명은 https://nikh.hrmk.studio

## 구조
- `nike_core/` — Rust 엔진. bge-m3(ONNX, int8) 쿼리 임베딩, int8 mmap 코사인 검색, 팩 v2(zstd 블록 지연 로드), 규칙 태그(심급·결과·적용 법률), PDF 사실관계 추출·OCR.
- `nike_app/` — Tauri 2 앱(macOS · Windows · iPadOS). 팩·업데이트 관리, 첫 실행 다운로드.
- `ui/` — 단일 HTML 프론트(한국어/English).
- `vendor/` — iOS 27(UIScene) 대응용 tao 0.37 패치와 tauri-runtime-wry 패치.

데이터 팩과 수집·정규화·임베딩 파이프라인은 이 저장소에 없습니다.

## 빌드
```
cargo tauri build --bundles dmg                       # macOS (nike_app 에서)
./build_windows_on_mac.sh                              # Windows 설치본, 맥에서 크로스 컴파일
PATH=$HOME/.cargo/bin:$PATH cargo tauri ios build      # iPadOS
```
검색 모델(`data/onnx/bge-m3/model_int8.onnx`, `tokenizer.json`)은 빌드 전에 https://nike-data.hrmk.studio/onnx/bge-m3/ 에서 받아 두면 앱에 내장됩니다.

iPadOS 빌드는 추가로 공식 onnxruntime-c 1.23 xcframework(양자화 연산자 포함 전체 빌드)를 `vendor/ort-ios/onnxruntime.xcframework`에 두어야 합니다(`gen/apple/project.yml`의 OTHER_LDFLAGS 참조). 서명 팀·번들 ID는 `tauri.conf.json`과 `gen/apple/project.yml`에서 바꾸면 됩니다.

## 지원 기기
- macOS 13+ (Apple silicon), Windows 10+ (x64)
- iPadOS 17+, M1 이상 아이패드

변경 이력은 [CHANGELOG.md](CHANGELOG.md).

## CLI · MCP (에이전트에서 쓰기)
`nike_core`는 앱 없이도 도는 명령줄 도구를 포함합니다. NIKH 앱을 설치하고 팩을 내려받았다면 설정 없이 그 팩을 그대로 씁니다(`NIKE_DATA`로 다른 폴더 지정 가능).
```
cargo build --release -p nike_core            # → nike_core/target/release/nike
nike packs                                    # 설치된 팩 목록(JSON)
nike search "임차인이 보증금을 돌려받기 전에 집을 비웠는데 임대인이 원상복구 비용을 공제했다" --k 10 --json
nike search "Employee fired two weeks after filing a workers' compensation claim; retaliation, pretext" --mode us --json
nike pdf 소장.pdf --json                       # 청구원인·공소사실(영문: statement of facts)만 추출, 스캔본은 OCR
nike serve 8791                               # http://localhost:8791 웹 UI
nike mcp                                      # MCP 서버(stdio)
```
MCP 서버는 `search`·`pdf_facts`·`packs` 세 도구를 냅니다. Claude Code:
```
claude mcp add nikh -- /절대경로/nike mcp
```
Claude Desktop·Cursor 등은 설정에 `{"mcpServers": {"nikh": {"command": "/절대경로/nike", "args": ["mcp"]}}}`.

NIKH 자체는 여전히 생성 0·기록 0이고 모든 검색은 이 기기 안에서 끝납니다. 다만 클라우드 모델(Claude, GPT 등)에 물리면 **검색어와 결과가 그 모델 제공사로 전송되는 건 사용자 선택**입니다. 의뢰인 정보를 다루면 로컬 모델(Ollama 등)과 함께 쓰거나 앱을 쓰세요.

## 라이선스
AGPL-3.0
