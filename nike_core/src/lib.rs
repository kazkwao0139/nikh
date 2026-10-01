//! 니케 코어 — search.py 1:1. 질의 → bge-m3(ONNX) 임베딩 → int8 인덱스 코사인 → 판례 단위 집계 → 태그·체인.
//! 화면에 나가는 문자열은 전부 레코드 원문·메타·태그. 생성 0.
mod engine;
mod ocr;
mod pdf;
pub use engine::{url_of, ChainNode, Engine, Hit, Rec, ZStore};
pub use ocr::ocr_pdf;
pub use pdf::{en_body_fallback, is_english, pdf_facts, pdf_section_by_heading, pdf_section_by_heading_en, pdf_text, split_paragraphs, Facts};
