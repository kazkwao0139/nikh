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

## 라이선스
AGPL-3.0
