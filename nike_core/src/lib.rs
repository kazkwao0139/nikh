//! 니케 코어 — search.py 1:1. 질의 → bge-m3(ONNX) 임베딩 → int8 인덱스 코사인 → 판례 단위 집계 → 태그·체인.
//! 화면에 나가는 문자열은 전부 레코드 원문·메타·태그. 생성 0.
mod engine;
mod pdf;
mod ocr;
pub use engine::{Engine, Rec, Hit, ChainNode, ZStore, url_of};
pub use pdf::{pdf_text, pdf_facts, pdf_section_by_heading, pdf_section_by_heading_en, en_body_fallback, is_english, split_paragraphs};
pub use ocr::ocr_pdf;
