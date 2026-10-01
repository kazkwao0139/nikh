//! PDF 사실관계 추출 — 텍스트 레이어 → 표제 규칙(한국어 청구원인·공소사실 / 영어 statement of facts) → 없으면 문단 임베딩 분류(한국어) 또는 위치 규칙(영어). 스캔본은 OCR(ocr 모듈).
use crate::engine::Engine;
use crate::ocr::ocr_pdf;

/// (구간 라벨, 본문, 문단별 (라벨, 앞 80자, 유사도) — 자동 분류일 때만)
pub type Facts = (String, String, Vec<(String, String, f32)>);

/// PACER 스탬프('Case 2:24-cv-00123 Document 1 Filed…')·'Page 3 of 20'·플리딩 용지 줄번호(1–28) 제거. bare_page_numbers: 숫자만 있는 줄(쪽번호)도 지움
fn strip_court_stamps(text: &str, bare_page_numbers: bool) -> String {
    let re_stamp = Regex::new(if bare_page_numbers {
        r"(?mi)^\s*Case\s+\d[:\d]*-[a-z]{2}-\d+[^\n]*$|^\s*(?:Page\s+\d+\s+of\s+\d+|-\s*\d+\s*-|\d{1,3})\s*$"
    } else {
        r"(?mi)^\s*Case\s+\d[:\d]*-[a-z]{2}-\d+[^\n]*$|^\s*(?:Page\s+\d+\s+of\s+\d+|-\s*\d+\s*-)\s*$"
    })
    .unwrap();
    let re_lineno = Regex::new(r"(?m)^\s{0,6}\d{1,2}\s{2,}").unwrap();
    re_lineno.replace_all(&re_stamp.replace_all(text, ""), "").to_string()
}

/// PDF 줄바꿈 복원: 문장 중간 줄바꿈은 붙이고 번호 항목(re_item) 앞은 유지. skip(줄) = 문단 경계로 취급(쪽번호 등), closes(줄) = 그 줄에서 문장이 끝남. 빈 줄 3개 이상은 2개로.
fn join_wrapped_lines(body: &str, re_item: &Regex, skip: impl Fn(&str) -> bool, closes: impl Fn(&str) -> bool) -> String {
    let mut out = String::new();
    let mut prev_open = false;
    for line in body.lines() {
        let l = line.trim();
        if l.is_empty() || skip(l) {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            prev_open = false;
            continue;
        }
        if prev_open && !re_item.is_match(l) {
            out.push(' ');
        } else if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(l);
        prev_open = !closes(l);
    }
    Regex::new(r"\n{3,}").unwrap().replace_all(&out, "\n\n").to_string()
}
use anyhow::Result;
use regex::Regex;
use serde::Deserialize;
use std::path::Path;

/// 소장·공소장 PDF(텍스트 레이어) → 사실관계 구간. 스캔본(텍스트 없음)이면 Err. 생성 0: 문서 문장을 그대로 돌려줌.
pub fn pdf_text(bytes: &[u8]) -> Result<String> {
    // pdf-extract 는 일부 PDF(cmap 손상 등)에서 패닉 → 잡아서 OCR 경로로 넘김(앱 크래시 방지)
    let raw = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdf_extract::extract_text_from_mem(bytes))) {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => anyhow::bail!("pdf: {e}"),
        Err(_) => anyhow::bail!("NO_TEXT_LAYER"),
    };
    let text: String = raw.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
    let vis: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if vis.len() < 40 {
        anyhow::bail!("NO_TEXT_LAYER");
    }
    // 텍스트 레이어는 있지만 글리프 매핑이 깨진 경우(˘ˇˆ…): 읽을 수 있는 문자 비율이 낮으면 스캔본으로 취급
    let ok = vis.iter().filter(|c| c.is_alphanumeric() || ('가'..='힣').contains(c) || ".,;:()[]'\"-–—§$%/&*".contains(**c)).count();
    if ok * 10 < vis.len() * 7 {
        anyhow::bail!("NO_TEXT_LAYER");
    }
    Ok(text)
}
/// 표제 규칙만으로 구간 절단 (표제가 있을 때만 Some)
/// 영어 문서 판정: 알파벳 대비 한글 비율(한글이 1% 미만이면 영어)
pub fn is_english(text: &str) -> bool {
    let (mut ko, mut en) = (0usize, 0usize);
    for c in text.chars().take(20000) {
        if ('가'..='힣').contains(&c) {
            ko += 1;
        } else if c.is_ascii_alphabetic() {
            en += 1;
        }
    }
    en > 200 && ko * 100 < en
}
/// 미국 소장·기소장·브리프: 표제(STATEMENT OF FACTS 등) 사이 구간. 대문자·공백 제거본에서 찾음. PACER 스탬프·플리딩 용지 줄번호·쪽 표기 제거.
pub fn pdf_section_by_heading_en(text: &str) -> Result<Option<(String, String)>> {
    let t = strip_court_stamps(text, true);
    let chars: Vec<char> = t.chars().collect();
    let mut map = Vec::with_capacity(chars.len());
    let mut sq: Vec<char> = Vec::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() {
        if c.is_ascii_alphanumeric() {
            sq.push(c.to_ascii_uppercase());
            map.push(i);
        }
    }
    let norm = |p: &str| -> Vec<char> { p.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect() };
    // 표제 판정: 원문에서 그 위치 앞이 줄 시작(번호 'IV.' 'A.' '1.' '(a)' 허용)이고, 그 줄이 짧아야(≤80자) 함 → 본문 문장 속 언급은 제외
    let is_heading = |i: usize, plen: usize| -> bool {
        let a = map[i];
        let mut k = a;
        while k > 0 && chars[k - 1] != '\n' {
            k -= 1;
        }
        let pre: String = chars[k..a].iter().collect();
        let pre = pre.trim();
        let pre_ok = pre.is_empty() || Regex::new(r"^(?:[IVX]{1,5}\.?|[A-Z]\.|\d{1,2}\.|\(\w{1,3}\)|[A-Z]\))\s*$").map(|r| r.is_match(pre)).unwrap_or(false);
        let end_i = map[i + plen - 1] + 1;
        // 길이·목차 판정은 '줄 시작 ~ 표제 끝'까지만 본다(줄 끝까지 넓히면 'The Grand Jury charges that on or about …' 같은 본문 첫 줄이 80자 규칙에 걸리고, 'CHARGES: COUNT 1' 의 끝 숫자가 쪽번호로 오인됨 — 560건 스윕에서 10건 악화 확인, 2026-10-01)
        let e = end_i;
        let line: String = chars[k..e].iter().collect();
        let ahead: String = chars[e..(e + 300).min(chars.len())].iter().collect();
        let toc = line.contains("....") || ahead.contains("....") || Regex::new(r"\s\d{1,3}\s*$").map(|r| r.is_match(line.trim_end())).unwrap_or(false); // 목차 줄('Facts.' 다음 줄에 '...... 6') 제외
        pre_ok && (e - k) <= 80 && !toc
    };
    let find = |pat: &str, from: usize| -> Option<usize> {
        let p = norm(pat);
        if p.is_empty() || sq.len() < p.len() {
            return None;
        }
        (from..=sq.len() - p.len()).find(|&i| sq[i..i + p.len()] == p[..] && is_heading(i, p.len()))
    };
    let cut = |start_pat: &[&str], end_pat: &[&str], label: &str| -> Option<(String, String)> {
        let st = start_pat.iter().filter_map(|p| find(p, 0).map(|i| i + norm(p).len())).min()?;
        let en = end_pat.iter().filter_map(|p| find(p, st + 40)).min().unwrap_or(sq.len());
        if en <= st {
            return None;
        }
        let a = map[st];
        let b = if en >= map.len() { chars.len() } else { map[en] };
        Some((label.to_string(), chars[a..b].iter().collect::<String>().trim().to_string()))
    };
    let counts = [
        "COUNT I",
        "COUNT ONE",
        "COUNT 1",
        "FIRST CAUSE OF ACTION",
        "FIRST CLAIM",
        "CLAIMS FOR RELIEF",
        "CAUSES OF ACTION",
        "CLAIM FOR RELIEF",
        "PRAYER FOR RELIEF",
        "WHEREFORE",
        "DEMAND FOR JURY TRIAL",
        "JURY DEMAND",
        "REQUEST FOR RELIEF",
    ];
    let r = cut(
        &[
            "STATEMENT OF FACTS",
            "FACTUAL ALLEGATIONS",
            "FACTUAL BACKGROUND",
            "GENERAL ALLEGATIONS",
            "ALLEGATIONS COMMON TO ALL COUNTS",
            "STATEMENT OF THE FACTS",
            "FACTS COMMON TO ALL",
            "FACTS",
            "BACKGROUND FACTS",
            "RELEVANT FACTS",
            "SUBSTANTIVE ALLEGATIONS",
        ],
        &counts,
        "Statement of Facts",
    ) // 소장
    .or_else(|| {
        cut(
            &["THE GRAND JURY CHARGES", "THE GRAND JURY FURTHER CHARGES", "OVERT ACTS", "MANNER AND MEANS"],
            &["FORFEITURE ALLEGATION", "FORFEITURE", "A TRUE BILL", "FOREPERSON"],
            "Indictment",
        )
    }) // 기소장
    .or_else(|| {
        cut(
            &["STATEMENT OF THE CASE", "STATEMENT OF FACTS", "FACTUAL BACKGROUND", "BACKGROUND"],
            &["ARGUMENT", "LEGAL STANDARD", "STANDARD OF REVIEW", "DISCUSSION", "SUMMARY OF ARGUMENT", "LEGAL ANALYSIS"],
            "Statement of the Case",
        )
    }) // 브리프·모션
    .or_else(|| cut(&["NATURE OF THE ACTION", "NATURE OF THE CASE", "PRELIMINARY STATEMENT"], &counts, "Nature of the Action")); // 'ALLEGATIONS'·'COMPLAINT' 단독은 본문 언급과 구분 불가 → 위치 규칙(en_body_fallback)에 맡김
    let r = match r {
        Some(r) if r.1.chars().filter(|c| c.is_alphanumeric()).count() >= 200 => r,
        _ => return Ok(None),
    };
    let re_item = Regex::new(r"^\s*(\d{1,3}\.|[a-z]\.|\(\d{1,3}\)|\([a-z]\)|[IVX]+\.)")?;
    let body = join_wrapped_lines(&r.1, &re_item, |_| false, |l| l.ends_with('.') || l.ends_with(':') || l.ends_with(';'));
    Ok(Some((r.0, body.chars().take(6000).collect())))
}
/// 영어 문서 표제 없음: 첫 번호 문단('1.')·'COMES NOW'·'Plaintiff … alleges' 부터 WHEREFORE·PRAYER·COUNT·서명·송달증명 전까지(캡션·서명 제거). 그것도 없으면 본문 앞 6,000자.
pub fn en_body_fallback(text: &str) -> (String, String) {
    let t = strip_court_stamps(text, true);
    let re_start = Regex::new(
        r"(?mi)^\s*1\.\s+\S|^\s*COMES?\s+NOW\b|^\s*(?:Plaintiffs?|Petitioners?|Defendants?)\b[^\n]{0,80}\b(?:alleges?|states?|avers?|complains?|petitions?)\b",
    )
    .unwrap();
    let re_end = Regex::new(r"(?mi)^\s*(?:WHEREFORE|PRAYER\s+FOR\s+RELIEF|REQUEST\s+FOR\s+RELIEF|COUNT\s+(?:I|ONE|1)\b|FIRST\s+(?:CAUSE|CLAIM)|CAUSES?\s+OF\s+ACTION|CLAIMS?\s+FOR\s+RELIEF|Respectfully\s+submitted|CERTIFICATE\s+OF\s+SERVICE|DEMAND\s+FOR\s+JURY|JURY\s+DEMAND|A\s+TRUE\s+BILL)").unwrap();
    let st = re_start.find(&t).map(|m| m.start()).unwrap_or(0);
    let en = re_end.find_at(&t, (st + 200).min(t.len())).map(|m| m.start()).unwrap_or(t.len());
    let body = t[st..en.max(st)].trim();
    let re_item = Regex::new(r"^\s*(\d{1,3}\.|[a-z]\.|\(\d{1,3}\)|\([a-z]\)|[IVX]+\.)").unwrap();
    let out = join_wrapped_lines(body, &re_item, |_| false, |l| l.ends_with('.') || l.ends_with(':') || l.ends_with(';'));
    ((if st > 0 { "Body (caption and prayer removed)" } else { "Full text" }).to_string(), out.chars().take(6000).collect())
}
pub fn pdf_section_by_heading(text: &str) -> Result<Option<(String, String)>> {
    if is_english(text) {
        return pdf_section_by_heading_en(text);
    }
    // 표제는 글자 사이를 띄우는 관행("청 구 원 인") → 공백 제거본에서 위치를 찾고 원문 인덱스로 되돌림
    let chars: Vec<char> = text.chars().collect();
    let mut map = Vec::with_capacity(chars.len());
    let mut sq: Vec<char> = Vec::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() {
        if !c.is_whitespace() {
            sq.push(*c);
            map.push(i);
        }
    }
    let find = |pat: &str, from: usize| -> Option<usize> {
        let p: Vec<char> = pat.chars().collect();
        if p.is_empty() || sq.len() < p.len() {
            return None;
        }
        (from..=sq.len() - p.len()).find(|&i| sq[i..i + p.len()] == p[..])
    }; // 문자 단위 검색(바이트 경계 문제 없음)
    let cut = |start_pat: &[&str], end_pat: &[&str], label: &str| -> Option<(String, String)> {
        let st = start_pat.iter().filter_map(|p| find(p, 0).map(|i| i + p.chars().count())).min()?;
        let en = end_pat.iter().filter_map(|p| find(p, st)).min().unwrap_or(sq.len());
        if en <= st {
            return None;
        }
        let a = map[st];
        let b = if en >= map.len() { chars.len() } else { map[en] };
        let body: String = chars[a..b].iter().collect();
        Some((label.to_string(), body.trim().to_string()))
    };
    let r = cut(&["청구원인에대한답변", "청구원인에관한답변"], &["입증방법", "첨부서류", "증거방법"], "청구원인에 대한 답변") // 답변서(더 긴 표제를 먼저)
        .or_else(|| cut(&["청구원인"], &["입증방법", "첨부서류", "증거방법", "관할법원"], "청구원인"))
        .or_else(|| cut(&["공소사실", "범죄사실"], &["첨부서류", "증거", "적용법조"], "공소사실"))
        .or_else(|| cut(&["고소사실", "고소이유", "고소원인"], &["입증방법", "첨부서류", "증거자료", "관련사건"], "고소사실")) // 고소장
        .or_else(|| cut(&["신청이유", "신청원인"], &["소명방법", "입증방법", "첨부서류"], "신청이유")) // 가압류·가처분·지급명령
        .or_else(|| cut(&["항소이유", "상고이유", "항고이유", "재항고이유"], &["입증방법", "첨부서류", "증거방법"], "항소이유"))
        .or_else(|| cut(&["심판청구이유", "청구이유", "심판청구의이유"], &["입증방법", "첨부서류", "증거서류"], "심판청구이유")) // 행정심판·조세심판 청구서
        .or_else(|| cut(&["기초사실", "사실관계"], &["입증방법", "첨부서류", "판단"], "사실관계"))
        .or_else(|| cut(&["특허청구범위", "청구범위"], &["발명의설명", "발명의상세한설명", "도면의간단한설명", "요약서", "요약"], "청구범위")) // 특허 명세서·공보
        .or_else(|| cut(&["요약"], &["대표도", "청구범위", "기술분야"], "요약"));
    let r = match r {
        Some(r) if r.1.chars().filter(|c| !c.is_whitespace()).count() >= 60 => r,
        _ => return Ok(None),
    };
    // 증거 참조·페이지 번호 등 잡음 제거 + PDF 줄바꿈 복원(문장 중간 줄바꿈은 붙이고, 번호 항목 앞은 유지)
    let re_junk = Regex::new(r"(?s)[{｛][^}｝]*호증[^}｝]*[}｝]|\(갑\s*제?\s*\d+호증[^)]*\)|\[갑\s*제?\s*\d+호증[^\]]*\]")?;
    let body = re_junk.replace_all(&r.1, "").to_string();
    let re_item = Regex::new(r"^\s*(\d{1,2}\.|[가-하]\.|\(\d{1,2}\)|[①-⑳])")?;
    let re_page = Regex::new(r"^\s*-?\s*\d{1,3}\s*-?\s*$")?;
    let body = join_wrapped_lines(
        &body,
        &re_item,
        |l| re_page.is_match(l),
        |l| l.ends_with('.') || l.ends_with('。') || l.ends_with(':') || l.ends_with("다") && l.len() < 12,
    );
    Ok(Some((r.0, body.chars().take(6000).collect())))
}
pub fn pdf_facts(bytes: &[u8]) -> Result<(String, String)> {
    // 규칙만(엔진 없이): 표제 없으면 전문
    let text = pdf_text(bytes)?;
    Ok(pdf_section_by_heading(&text)?.unwrap_or_else(|| ("전문".to_string(), text.trim().chars().take(6000).collect())))
}
const PARA_PROTO: &str = include_str!("../assets/para_proto.json"); // 공단 작성례 1,137건에서 구운 문단 유형 프로토타입(FACT/PRAYER/EVID/HEAD/TAIL)
#[derive(Deserialize)]
struct Proto {
    labels: Vec<String>,
    vectors: Vec<Vec<f32>>,
}
/// 문단 나누기(빈 줄 또는 번호 항목 앞), 15자 미만·쪽번호 제외, '귀중' 이후(해설·서명) 제외
pub fn split_paragraphs(text: &str) -> Vec<String> {
    let mut t = text.to_string();
    if is_english(&t) {
        t = strip_court_stamps(&t, false);
    }
    if let Some(m) = Regex::new(r"\n[^\n]*귀\s*중[^\n]*\n").ok().and_then(|re| re.find(&t)) {
        t.truncate(m.end());
    }
    let re_item = Regex::new(r"^\s*(?:\d{1,3}\.|[가-하]\.|[a-z]\.|\(\d{1,3}\)|\([a-z]\)|[①-⑳]|[IVX]+\.)").unwrap();
    let re_page = Regex::new(r"^\s*-?\s*\d{1,3}\s*-?\s*$").unwrap();
    let mut paras: Vec<String> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let flush = |cur: &mut Vec<&str>, paras: &mut Vec<String>| {
        if !cur.is_empty() {
            let p = cur.join(" ");
            if p.chars().count() >= 15 && !re_page.is_match(&p) {
                paras.push(p);
            }
            cur.clear();
        }
    };
    for line in t.lines() {
        let l = line.trim();
        if l.is_empty() {
            flush(&mut cur, &mut paras);
            continue;
        } // 빈 줄 = 문단 경계
        if re_item.is_match(l) && !cur.is_empty() {
            flush(&mut cur, &mut paras);
        } // 번호 항목 시작 = 새 문단
        cur.push(l);
    }
    flush(&mut cur, &mut paras);
    paras
}
impl Engine {
    /// 소장·공소장·답변서·준비서면 PDF → 사실 문단. 1) 표제 규칙이 맞으면 그 구간, 2) 아니면 문단마다 임베딩→프로토타입 코사인으로 FACT 문단만 선별(생성 0, 문단별 유사도 반환).
    /// ocr_helper 가 있으면 텍스트 레이어 없는 PDF 를 OCR 로 읽음
    pub fn pdf_facts_smart_ocr(&mut self, bytes: &[u8], ocr_helper: Option<&Path>) -> Result<Facts> {
        // 텍스트 레이어 추출 실패(스캔본·깨진 글리프·폰트 오류·파서 패닉) → 전부 OCR 로. 텍스트는 뽑혔는데 사실 구간을 못 찾으면 OCR 로 한 번 더(텍스트 레이어가 부분적인 스캔본).
        let helper = ocr_helper.unwrap_or(Path::new("ocr/nike_ocr"));
        let (text, mut ocr_used) = match pdf_text(bytes) {
            Ok(t) => (t, false),
            Err(e) => {
                let msg = e.to_string();
                (ocr_pdf(bytes, helper).map_err(|oe| if msg == "NO_TEXT_LAYER" { oe } else { anyhow::anyhow!("{msg}; {oe}") })?, true)
            }
        };
        let (lab, body, detail) = match self.facts_from_text(&text) {
            Ok(r) => r,
            Err(e) if !ocr_used => {
                let t2 = ocr_pdf(bytes, helper).map_err(|_| e)?;
                ocr_used = true;
                self.facts_from_text(&t2)?
            }
            Err(e) => return Err(e),
        };
        Ok((if ocr_used { format!("{lab} · OCR") } else { lab }, body, detail))
    }
    pub fn facts_from_text(&mut self, text: &str) -> Result<Facts> {
        let text = text.replace('\u{0C}', "\n");
        if let Some((lab, body)) = pdf_section_by_heading(&text)? {
            return Ok((lab, body, vec![]));
        }
        let en = is_english(&text);
        if en {
            // 영어: 표제가 없으면 위치 규칙(캡션·서명 제거)만 — 문단 임베딩 분류는 사실/청구 구분력이 낮아(홀드아웃 48%) 쓰지 않음
            let (lab, body) = en_body_fallback(&text);
            if body.chars().count() < 200 {
                anyhow::bail!("No factual section found. Paste the facts directly.");
            }
            return Ok((lab, body, vec![]));
        }
        let proto: Proto = serde_json::from_str(PARA_PROTO)?;
        let paras = split_paragraphs(&text); // 한국어 전용(영어는 위에서 규칙으로 끝남)
        let mut picked = Vec::new();
        let mut detail = Vec::new();
        for p in paras.iter().take(400) {
            let v = self.embed(p)?;
            let mut best = (0usize, -1.0f32);
            for (i, pv) in proto.vectors.iter().enumerate() {
                let c: f32 = pv.iter().zip(&v).map(|(a, b)| a * b).sum();
                if c > best.1 {
                    best = (i, c);
                }
            }
            let lab = proto.labels[best.0].clone();
            detail.push((lab.clone(), p.chars().take(80).collect(), best.1));
            if lab == "FACT" {
                picked.push(p.clone());
            }
        }
        if picked.is_empty() {
            anyhow::bail!("사실 서술 문단을 찾지 못했습니다(문단 {}개). 내용을 직접 붙여 넣어 주세요.", paras.len());
        }
        let body: String = picked.join("\n");
        Ok((format!("사실 문단 {}개(자동 분류)", picked.len()), body.chars().take(6000).collect(), detail))
    }
}
