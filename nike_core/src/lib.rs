//! 니케 코어 — search.py 1:1. 질의 → bge-m3(ONNX) 임베딩 → int8 인덱스 코사인 → 판례 단위 집계 → 태그·체인.
//! 화면에 나가는 문자열은 전부 레코드 원문·메타·태그. 생성 0.
use anyhow::Result;
use memmap2::Mmap;
use ort::{session::{builder::GraphOptimizationLevel, Session}, value::Tensor};
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs::File, io::{BufRead, BufReader}, path::{Path, PathBuf}};
use tokenizers::Tokenizer;

#[derive(Deserialize)]
struct Chunk { i: String, s: String, t: String }

#[derive(Deserialize, Serialize, Clone)]
pub struct Rec {
    pub id: String,
    #[serde(rename = "사건명", default)] pub title: String,
    #[serde(rename = "사건번호", default)] pub caseno: String,
    #[serde(rename = "선고일자", default)] pub date: String,
    #[serde(rename = "법원", default)] pub court: Option<String>,
    #[serde(rename = "종류", default)] pub kind: Option<String>,
    #[serde(rename = "심급", default)] pub level: Option<String>,
    #[serde(rename = "적용법률", default)] pub laws: Vec<String>,
    #[serde(rename = "결과", default)] pub result: Vec<String>,
    #[serde(rename = "인정금액", default)] pub amount: Option<i64>,
    #[serde(rename = "참조판례", default)] pub refs: String,
    #[serde(rename = "판시사항", default)] pub issue: String,
    #[serde(rename = "판결요지", default)] pub summary: String,
    #[serde(rename = "전문", default)] pub full: String,
    #[serde(default)] pub prev_no: Option<String>, #[serde(default)] pub prev_txt: Option<String>, #[serde(default)] pub first_txt: Option<String>,   // v2 meta 사전계산
    #[serde(rename = "원문", default)] pub url: Option<String>,        // 특허공보: Google Patents 링크
    #[serde(rename = "키프리스", default)] pub kipris: Option<String>,  // 특허공보: KIPRIS(DOI) 링크
}

/// 레코드 id → 원문 링크 (판례는 순수 번호, 나머지는 '접두:번호')
pub fn url_of(id: &str, r: &Rec) -> String {
    if let Some(u) = &r.url { return u.clone(); }
    match id.split_once(':') {
        None => format!("https://www.law.go.kr/LSW/precInfoP.do?precSeq={}", id),
        Some(("expc", n)) => format!("https://www.law.go.kr/LSW/expcInfoP.do?expcSeq={}", n),
        Some(("decc", n)) => format!("https://www.law.go.kr/LSW/deccInfoP.do?deccSeq={}", n),
        Some(("detc", n)) => format!("https://www.law.go.kr/LSW/detcInfoP.do?detcSeq={}", n),
        Some(("trty", n)) => format!("https://www.law.go.kr/LSW/trtyInfoP.do?trtySeq={}", n),
        Some(("tax", n)) => format!("https://www.law.go.kr/LSW/precInfoR.do?precSeq={}", n),
        Some(("tt", n)) => format!("https://www.law.go.kr/LSW/specialDeccInfoP.do?ttSpecialDeccSeq={}", n),   // 공개 페이지(API 키 불필요)
        Some((t @ ("ftc" | "kcc" | "iaciac" | "ecc" | "eiac" | "oclt"), n)) => format!("https://www.law.go.kr/LSW/{}InfoR.do?{}Seq={}", t, t, n),   // 공개 페이지(본문 직접 렌더)
        Some((t, n)) => format!("https://www.law.go.kr/LSW/{}InfoP.do?{}Seq={}", t, t, n),   // ppc·acr·nlrc 결정문 공개 페이지
    }
}

#[derive(Serialize, Clone)]
pub struct ChainNode {
    pub id: String, pub level: Option<String>, pub court: Option<String>, pub caseno: String, pub date: String,
    pub result: Vec<String>, pub issue: String, pub summary: String, pub refs: String, pub laws: String, pub full: String,
    pub prev_text: Option<String>, pub prev_missing: bool, pub first_text: Option<String>, pub url: String,
}

#[derive(Serialize)]
pub struct Hit {
    pub score: f32, pub title: String, pub caseno: String, pub court: Option<String>, pub date: String,
    pub level: Option<String>, pub kind: Option<String>, pub result: Vec<String>, pub laws: Vec<String>,
    pub sec: String, pub snippet: String, pub issue: String, pub url: String, pub chain: Vec<ChainNode>, pub kipris: Option<String>,
}

struct PackMem { emb: Mmap, scale: Vec<f32>, n: usize, key: String, recs: Option<ZStore>, chunks: Option<ZStore>, texts: Vec<String> }   // texts: v1 팩만(팩 내부 인덱스)

/// v2 텍스트 저장소: zstd 블록 mmap + 블록 표. 마지막 블록 1개 캐시. 검색엔 안 쓰이고 표시할 때만 해제.
pub struct ZStore { mmap: Mmap, blocks: Vec<(u64, u64)>, cache: std::sync::Mutex<Option<(u32, Vec<u8>)>> }
impl ZStore {
    fn open(zst: &Path, blocks: Vec<(u64, u64)>) -> Result<Self> { Ok(ZStore { mmap: unsafe { Mmap::map(&File::open(zst)?)? }, blocks, cache: std::sync::Mutex::new(None) }) }
    fn get(&self, block: u32, off: u32, len: u32) -> String {
        let mut c = self.cache.lock().unwrap();
        if c.as_ref().map(|(b, _)| *b != block).unwrap_or(true) {
            let (o, l) = self.blocks[block as usize]; let raw = &self.mmap[o as usize..(o + l) as usize];
            let dec = zstd::stream::decode_all(raw).unwrap_or_default(); *c = Some((block, dec));
        }
        let d = &c.as_ref().unwrap().1; String::from_utf8_lossy(&d[off as usize..(off + len) as usize]).to_string()
    }
}
#[derive(Deserialize)] struct RecIdx { blocks: Vec<(u64, u64)>, items: Vec<(String, u32, u32, u32)> }
#[derive(Deserialize)] struct ChunkIdx { blocks: Vec<(u64, u64)>, items: Vec<(String, String, u32, u32, u32)> }
#[derive(Clone, Copy)] struct Pos { pack: u16, block: u32, off: u32, len: u32 }
/// 모드: 특허 팩(pat_*)만 / 특허 제외 / 전부
fn pack_in_mode(key: &str, mode: Option<&str>) -> bool {
    match mode { Some("patent") => key.starts_with("pat_"), Some("us") => key.starts_with("us_"), Some("case") => !key.starts_with("pat_") && !key.starts_with("us_"),
        Some(m) if m.starts_with("packs:") => m[6..].split(',').any(|k| k.trim() == key),   // "packs:civil,criminal" → 그 팩만 검색
        _ => true }
}
pub struct Engine {
    n: usize, dim: usize,
    packs: Vec<PackMem>,       // 도메인별 팩 (int8 n_k×dim), 청크 배열은 팩 순서로 이어 붙임
    pub loaded: Vec<String>,
    chunk_id: Vec<String>, chunk_sec: Vec<String>, chunk_text: Vec<String>,   // chunk_text 는 v1 팩만 채움
    chunk_pos: Vec<Option<Pos>>,                 // v2 팩: 청크 텍스트 위치
    rec_pos: HashMap<String, Pos>,               // v2 팩: 원본 레코드 위치 (전문·요지는 여기서 지연 로드)
    recs: HashMap<String, Rec>,                  // v1: 전체 / v2: meta (전문 없음)
    by_no: HashMap<String, String>,            // 사건번호(공백 제거) → id
    prev_of: HashMap<String, (Option<String>, Option<String>)>,  // id → (원심 사건번호, 원심 표기)
    next_of: HashMap<String, String>,
    tok: Tokenizer, sess: Session,
    re_prev: Regex, re_no: Regex, re_first: Regex,
}

fn caseno_key(s: &str) -> String { s.split(',').next().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect() }

impl Engine {
    pub fn load(pack: &Path, model_dir: &Path) -> Result<Self> { Self::load_packs(&[pack.to_path_buf()], model_dir) }

    /// 여러 팩 디렉터리를 이어 붙여 로드 (체크한 팩만). 각 팩: meta.json(n, dim), emb_i8.bin, emb_scale.bin, chunks.json, recs.jsonl
    pub fn load_packs(packs: &[PathBuf], model_dir: &Path) -> Result<Self> { Self::load_packs_with(packs, model_dir, &|_, _, _| {}) }

    /// progress(done, total, 현재 팩 키): 팩 하나 읽기 전에 호출 (게이지용)
    pub fn load_packs_with(packs: &[PathBuf], model_dir: &Path, progress: &dyn Fn(usize, usize, &str)) -> Result<Self> {
        let mut dim = 0usize; let mut mems = Vec::new(); let mut chunk_id = Vec::new(); let mut chunk_sec = Vec::new(); let mut chunk_text = Vec::new(); let mut v1_texts: Vec<String> = Vec::new();
        let mut recs: HashMap<String, Rec> = HashMap::new(); let mut loaded = Vec::new(); let mut chunk_pos: Vec<Option<Pos>> = Vec::new(); let mut rec_pos: HashMap<String, Pos> = HashMap::new();
        for (pi, pack) in packs.iter().enumerate() {
            progress(pi, packs.len(), &pack.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            let meta: serde_json::Value = serde_json::from_reader(File::open(pack.join("meta.json"))?)?;
            let n = meta["n"].as_u64().unwrap() as usize; let d = meta["dim"].as_u64().unwrap() as usize;
            if dim == 0 { dim = d; } anyhow::ensure!(dim == d, "dim mismatch in {:?}", pack);
            let emb = unsafe { Mmap::map(&File::open(pack.join("emb_i8.bin"))?)? };
            anyhow::ensure!(emb.len() == n * dim, "emb size mismatch in {:?}", pack);
            let sb = std::fs::read(pack.join("emb_scale.bin"))?;
            let scale: Vec<f32> = sb.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            let key = meta["key"].as_str().unwrap_or("pack").to_string();
            let (rstore, cstore) = if pack.join("meta.jsonl").exists() {   // v2: 텍스트는 zstd 블록, 메타만 상주
                let ci: ChunkIdx = serde_json::from_reader(BufReader::new(File::open(pack.join("chunks.idx"))?))?;
                anyhow::ensure!(ci.items.len() == n, "chunks.idx size mismatch in {:?}", pack);
                for (i, s_, b, o, l) in ci.items { chunk_id.push(i); chunk_sec.push(s_); chunk_pos.push(Some(Pos { pack: pi as u16, block: b, off: o, len: l })); }
                let ri: RecIdx = serde_json::from_reader(BufReader::new(File::open(pack.join("recs.idx"))?))?;
                for (i, b, o, l) in ri.items { rec_pos.insert(i, Pos { pack: pi as u16, block: b, off: o, len: l }); }
                for line in BufReader::new(File::open(pack.join("meta.jsonl"))?).lines() { let r: Rec = serde_json::from_str(&line?)?; recs.insert(r.id.clone(), r); }
                (Some(ZStore::open(&pack.join("recs.zst"), ri.blocks)?), Some(ZStore::open(&pack.join("chunks.zst"), ci.blocks)?))
            } else {
                let chunks: Vec<Chunk> = serde_json::from_reader(BufReader::new(File::open(pack.join("chunks.json"))?))?;
                for (li, c) in chunks.into_iter().enumerate() { chunk_id.push(c.i); chunk_sec.push(c.s); v1_texts.push(c.t); chunk_pos.push(Some(Pos { pack: pi as u16, block: li as u32, off: 0, len: 0 })); }   // v1: block = 팩 내부 인덱스
                for line in BufReader::new(File::open(pack.join("recs.jsonl"))?).lines() { let r: Rec = serde_json::from_str(&line?)?; recs.insert(r.id.clone(), r); }
                (None, None)
            };
            mems.push(PackMem { emb, scale, n, key: key.clone(), recs: rstore, chunks: cstore, texts: std::mem::take(&mut v1_texts) }); loaded.push(key);
        }
        let n: usize = mems.iter().map(|p| p.n).sum();
        let re_prev = Regex::new(r"【\s*원심판결\s*】\s*([^\n]*)")?; let re_no = Regex::new(r"(\d{4}[가-힣]{1,2}\d+)")?;
        let re_first = Regex::new(r"【\s*제\s*1\s*심\s*판\s*결\s*】\s*([^\n]*)")?;
        let mut by_no = HashMap::new(); let mut prev_of = HashMap::new();
        for (id, r) in &recs {
            by_no.insert(caseno_key(&r.caseno), id.clone());
            let (pno, ptxt) = if r.prev_txt.is_some() || r.prev_no.is_some() { (r.prev_no.clone(), r.prev_txt.clone()) } else { match re_prev.captures(&r.full) {
                Some(c) => { let t = c[1].trim().chars().take(80).collect::<String>(); (re_no.captures(&t).map(|m| m[1].to_string()), Some(t)) }
                None => (None, None) } };
            prev_of.insert(id.clone(), (pno, ptxt));
        }
        let mut next_of = HashMap::new();
        for (id, (pno, _)) in &prev_of { if let Some(p) = pno { if let Some(pid) = by_no.get(p) { next_of.insert(pid.clone(), id.clone()); } } }
        progress(packs.len(), packs.len() + 2, "tokenizer");
        let tok = Tokenizer::from_file(model_dir.join("tokenizer.json")).map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;
        progress(packs.len() + 1, packs.len() + 2, "onnx");
        // int8(양자화) 우선, 로드 실패(축소 빌드 onnxruntime 등)면 fp32 model.onnx 로 폴백
        let candidates: Vec<std::path::PathBuf> = ["model_int8.onnx", "model.onnx"].iter().map(|f| model_dir.join(f)).filter(|p| p.exists()).collect();
        if candidates.is_empty() { anyhow::bail!("onnx 모델 없음: {}", model_dir.display()); }
        let mut sess_res = None; let mut last_err = String::new();
        for model_path in &candidates { match (|| -> Result<Session> { Ok(Session::builder().map_err(|e| anyhow::anyhow!("ort: {e}"))?.with_optimization_level(GraphOptimizationLevel::Level3).map_err(|e| anyhow::anyhow!("ort: {e}"))?.with_intra_threads(4).map_err(|e| anyhow::anyhow!("ort: {e}"))?.commit_from_file(model_path).map_err(|e| anyhow::anyhow!("ort: {e}"))?) })() { Ok(s) => { sess_res = Some(s); break; } Err(e) => { last_err = format!("{}: {e}", model_path.display()); } } }
        let sess = match sess_res { Some(s) => s, None => anyhow::bail!("ort: {last_err}") };
        Ok(Engine { n, dim, packs: mems, loaded, chunk_id, chunk_sec, chunk_text, chunk_pos, rec_pos, recs, by_no, prev_of, next_of, tok, sess, re_prev, re_no, re_first })
    }

    /// bge-m3: CLS 풀링 + L2 정규화 (sentence-transformers 설정과 동일)
    pub fn embed(&mut self, q: &str) -> Result<Vec<f32>> {
        let enc = self.tok.encode(q, true).map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
        let mask: Vec<i64> = enc.get_attention_mask().iter().map(|&x| x as i64).collect();
        let len = ids.len();
        let ids_t = Tensor::from_array(([1usize, len], ids)).map_err(|e| anyhow::anyhow!("ort: {e}"))?; let mask_t = Tensor::from_array(([1usize, len], mask)).map_err(|e| anyhow::anyhow!("ort: {e}"))?;
        let out = self.sess.run(ort::inputs!["input_ids" => ids_t, "attention_mask" => mask_t]).map_err(|e| anyhow::anyhow!("ort: {e}"))?;
        let (shape, data) = out[0].try_extract_tensor::<f32>().map_err(|e| anyhow::anyhow!("ort: {e}"))?;   // [1, len, dim] last_hidden_state
        let dim = shape[2] as usize;
        let mut v: Vec<f32> = data[..dim].to_vec();               // CLS = 위치 0
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9); v.iter_mut().for_each(|x| *x /= norm);
        Ok(v)
    }

    fn scores(&self, v: &[f32], mode: Option<&str>) -> Vec<f32> {
        let dim = self.dim; let mut out = Vec::with_capacity(self.n);
        for p in &self.packs {
            if !pack_in_mode(&p.key, mode) { out.extend(std::iter::repeat(f32::NEG_INFINITY).take(p.n)); continue; }   // 모드 밖 팩은 계산 생략
            let emb = &p.emb[..];
            let part: Vec<f32> = (0..p.n).into_par_iter().map(|i| {
                let row = &emb[i * dim..(i + 1) * dim];
                let dot: f32 = row.iter().zip(v).map(|(&a, &b)| (a as i8) as f32 * b).sum();
                dot * p.scale[i]
            }).collect();
            out.extend(part);
        }
        out
    }

    pub fn search(&mut self, q: &str, k: usize, filter: Option<(&str, &str)>) -> Result<Vec<Hit>> { self.search_mode(q, k, filter, None) }

    /// mode: Some("patent") = 특허 팩만, Some("case") = 특허 제외, None = 전부
    pub fn search_mode(&mut self, q: &str, k: usize, filter: Option<(&str, &str)>, mode: Option<&str>) -> Result<Vec<Hit>> {
        let v = self.embed(q)?; let s = self.scores(&v, mode);
        let mut order: Vec<usize> = (0..self.n).collect();
        // 상위 k*6 판례만 필요 → 부분 정렬
        let take = (k * 8).min(self.n);
        order.select_nth_unstable_by(take.saturating_sub(1), |&a, &b| s[b].partial_cmp(&s[a]).unwrap());
        order.truncate(take); order.sort_by(|&a, &b| s[b].partial_cmp(&s[a]).unwrap());
        let mut best: Vec<(String, f32, usize)> = Vec::new(); let mut seen = std::collections::HashSet::new();
        for i in order { if s[i] == f32::NEG_INFINITY { break; } let id = &self.chunk_id[i]; if seen.insert(id.clone()) { best.push((id.clone(), s[i], i)); } if best.len() >= k * 6 { break; } }
        let mut hits = Vec::new();
        for (id, sc, i) in best {
            let r = match self.recs.get(&id) { Some(r) => r, None => continue };
            if let Some((key, val)) = filter {
                let hay: Vec<String> = match key { "적용법률" => r.laws.clone(), "결과" => r.result.clone(), "종류" => vec![r.kind.clone().unwrap_or_default()], "심급" => vec![r.level.clone().unwrap_or_default()], _ => vec![] };
                let v2: String = val.chars().filter(|c| !c.is_whitespace()).collect();
                if !hay.iter().any(|h| h.chars().filter(|c| !c.is_whitespace()).collect::<String>().contains(&v2)) { continue; }
            }
            hits.push(Hit { score: (sc * 1000.0).round() / 1000.0, title: r.title.clone(), caseno: r.caseno.clone(), court: r.court.clone(), date: r.date.clone(),
                level: r.level.clone(), kind: r.kind.clone(), result: r.result.clone(), laws: r.laws.iter().take(6).cloned().collect(),
                sec: self.chunk_sec[i].clone(), snippet: self.chunk_text(i).chars().take(400).collect(), issue: r.issue.clone(),
                url: url_of(&id, r), kipris: r.kipris.clone(), chain: if id.contains(':') && !id.starts_with("cap:") { vec![] } else { self.chain_of(&id) } });
            if hits.len() >= k { break; }
        }
        Ok(hits)
    }

    /// 청크 텍스트(발췌용): v2 는 zstd 블록에서 지연 해제
    pub fn chunk_text(&self, i: usize) -> String {
        match self.chunk_pos.get(i).copied().flatten() { Some(p) => { let pk = &self.packs[p.pack as usize]; match pk.chunks.as_ref() { Some(s) => s.get(p.block, p.off, p.len), None => pk.texts.get(p.block as usize).cloned().unwrap_or_default() } } None => String::new() }
    }
    /// 원본 레코드 전체(전문·요지 포함): v2 는 recs.zst 에서 지연 해제
    pub fn full_rec(&self, id: &str) -> Option<Rec> {
        if let Some(p) = self.rec_pos.get(id) { let s = self.packs[p.pack as usize].recs.as_ref()?.get(p.block, p.off, p.len); return serde_json::from_str(&s).ok(); }
        self.recs.get(id).cloned()
    }

    pub fn chain_of(&self, pid: &str) -> Vec<ChainNode> {
        let mut first = pid.to_string();
        loop { match self.prev_of.get(&first) { Some((Some(pno), _)) => match self.by_no.get(pno) { Some(p) if p != &first => first = p.clone(), _ => break }, _ => break } }
        let mut out = Vec::new(); let mut cur = Some(first);
        while let Some(c) = cur { if out.len() >= 4 || out.iter().any(|o: &ChainNode| o.id == c) { break; }
            let r = match self.full_rec(&c) { Some(r) => r, None => break };
            let (pno, ptxt) = self.prev_of.get(&c).cloned().unwrap_or((None, None));
            let first_txt = self.recs.get(&c).and_then(|m| m.first_txt.clone()).or_else(|| self.re_first.captures(&r.full).map(|m| m[1].trim().chars().take(80).collect::<String>()));
            out.push(ChainNode { id: c.clone(), level: r.level.clone(), court: r.court.clone(), caseno: r.caseno.clone(), date: r.date.clone(), result: r.result.clone(),
                issue: r.issue.clone(), summary: r.summary.clone(), refs: r.refs.clone(), laws: r.laws.iter().take(12).cloned().collect::<Vec<_>>().join(" / "), full: r.full.clone(),
                prev_missing: pno.as_ref().map(|p| !self.by_no.contains_key(p)).unwrap_or(false), prev_text: ptxt, first_text: first_txt,
                url: url_of(&c, &r) });
            cur = self.next_of.get(&c).cloned();
        }
        out
    }

    pub fn stats(&self) -> (usize, usize) { (self.recs.len(), self.n) }

    /// 콜드스타트 제거: 임베딩 파일을 한 번 훑어 OS 페이지 캐시에 올리고(첫 검색의 디스크 읽기 선불), 모델 첫 추론(커널 초기화)도 미리.
    pub fn warm(&mut self) -> Result<()> { let _ = self.embed("준비")?; Ok(()) }   // ONNX 커널 초기화(수백 ms)

    /// 임베딩 파일 페이지-인 (엔진 락 없이 파일만 읽음). progress(읽은 바이트, 전체). 검색은 이와 무관하게 가능 — 캐시가 덜 된 동안만 느림.
    pub fn warm_files(packs: &[PathBuf], progress: &dyn Fn(u64, u64)) {
        use std::io::Read;
        let files: Vec<PathBuf> = packs.iter().map(|p| p.join("emb_i8.bin")).filter(|p| p.exists()).collect();
        let total: u64 = files.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum(); let mut done = 0u64; let mut buf = vec![0u8; 8 << 20];
        for f in files { if let Ok(mut fh) = File::open(&f) { while let Ok(n) = fh.read(&mut buf) { if n == 0 { break; } done += n as u64; std::hint::black_box(&buf[..n]); progress(done, total); } } }
    }
}

pub fn default_paths() -> (PathBuf, PathBuf) {
    let base = std::env::var("NIKE_DATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("data"));
    (base.join("pack"), base.join("../onnx/bge-m3"))
}

/// 소장·공소장 PDF(텍스트 레이어) → 사실관계 구간. 스캔본(텍스트 없음)이면 Err. 생성 0: 문서 문장을 그대로 돌려줌.
pub fn pdf_text(bytes: &[u8]) -> Result<String> {
    // pdf-extract 는 일부 PDF(cmap 손상 등)에서 패닉 → 잡아서 OCR 경로로 넘김(앱 크래시 방지)
    let raw = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdf_extract::extract_text_from_mem(bytes))) {
        Ok(Ok(t)) => t, Ok(Err(e)) => anyhow::bail!("pdf: {e}"), Err(_) => anyhow::bail!("NO_TEXT_LAYER") };
    let text: String = raw.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
    let vis: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if vis.len() < 40 { anyhow::bail!("NO_TEXT_LAYER"); }
    // 텍스트 레이어는 있지만 글리프 매핑이 깨진 경우(˘ˇˆ…): 읽을 수 있는 문자 비율이 낮으면 스캔본으로 취급
    let ok = vis.iter().filter(|c| c.is_alphanumeric() || ('가'..='힣').contains(c) || ".,;:()[]'\"-–—§$%/&*".contains(**c)).count();
    if ok * 10 < vis.len() * 7 { anyhow::bail!("NO_TEXT_LAYER"); }
    Ok(text)
}
/// 스캔본 OCR (macOS: 동봉 Vision 헬퍼 nike_ocr). 임시 파일은 즉시 삭제.
pub fn ocr_pdf(bytes: &[u8], helper: &Path) -> Result<String> {
    #[cfg(windows)] { let _ = helper; return ocr_pdf_windows(bytes); }   // Windows: WinRT(Windows.Media.Ocr + Windows.Data.Pdf), 별도 헬퍼 없음
    #[cfg(target_os = "ios")] { let _ = helper; return ocr_pdf_ios(bytes); }   // iPadOS: 앱 안에서 PDFKit 렌더 + Vision 인식(헬퍼 프로세스 불가)
    #[allow(unreachable_code)]
    if !helper.exists() { anyhow::bail!("스캔본(텍스트 레이어 없음)입니다. 이 플랫폼엔 OCR 헬퍼가 없습니다."); }
    let tmp = std::env::temp_dir().join(format!("nike_ocr_{}.pdf", std::process::id())); std::fs::write(&tmp, bytes)?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; let _ = std::fs::set_permissions(helper, std::fs::Permissions::from_mode(0o755)); }
    let out = std::process::Command::new(helper).arg(&tmp).output(); let _ = std::fs::remove_file(&tmp);
    let out = out.map_err(|e| anyhow::anyhow!("ocr 실행 실패: {e}"))?;
    if !out.status.success() { anyhow::bail!("ocr 실패: {}", String::from_utf8_lossy(&out.stderr)); }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if text.chars().filter(|c| !c.is_whitespace()).count() < 40 { anyhow::bail!("OCR로도 글자를 찾지 못했습니다."); }
    Ok(text)
}
/// 표제 규칙만으로 구간 절단 (표제가 있을 때만 Some)
/// 영어 문서 판정: 알파벳 대비 한글 비율(한글이 1% 미만이면 영어)
pub fn is_english(text: &str) -> bool { let (mut ko, mut en) = (0usize, 0usize); for c in text.chars().take(20000) { if ('가'..='힣').contains(&c) { ko += 1; } else if c.is_ascii_alphabetic() { en += 1; } } en > 200 && ko * 100 < en }
/// 미국 소장·기소장·브리프: 표제(STATEMENT OF FACTS 등) 사이 구간. 대문자·공백 제거본에서 찾음. PACER 스탬프·플리딩 용지 줄번호·쪽 표기 제거.
pub fn pdf_section_by_heading_en(text: &str) -> Result<Option<(String, String)>> {
    let re_stamp = Regex::new(r"(?mi)^\s*Case\s+\d[:\d]*-[a-z]{2}-\d+[^\n]*$|^\s*(?:Page\s+\d+\s+of\s+\d+|-\s*\d+\s*-|\d{1,3})\s*$")?;   // 'Case 2:24-cv-00123 Document 1 Filed…', 'Page 3 of 20', 쪽번호
    let re_lineno = Regex::new(r"(?m)^\s{0,6}\d{1,2}\s{2,}")?;   // 캘리포니아 플리딩 용지 줄번호(1–28)
    let t = re_lineno.replace_all(&re_stamp.replace_all(text, ""), "").to_string();
    let chars: Vec<char> = t.chars().collect(); let mut map = Vec::with_capacity(chars.len()); let mut sq: Vec<char> = Vec::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() { if c.is_ascii_alphanumeric() { sq.push(c.to_ascii_uppercase()); map.push(i); } }
    let norm = |p: &str| -> Vec<char> { p.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect() };
    // 표제 판정: 원문에서 그 위치 앞이 줄 시작(번호 'IV.' 'A.' '1.' '(a)' 허용)이고, 그 줄이 짧아야(≤80자) 함 → 본문 문장 속 언급은 제외
    let is_heading = |i: usize, plen: usize| -> bool {
        let a = map[i]; let mut k = a; while k > 0 && chars[k - 1] != '\n' { k -= 1; }
        let pre: String = chars[k..a].iter().collect(); let pre = pre.trim();
        let pre_ok = pre.is_empty() || Regex::new(r"^(?:[IVX]{1,5}\.?|[A-Z]\.|\d{1,2}\.|\(\w{1,3}\)|[A-Z]\))\s*$").map(|r| r.is_match(pre)).unwrap_or(false);
        let end_i = map[i + plen - 1] + 1; let mut e = end_i;   // 표제 마지막 글자 바로 다음부터 줄 끝까지 while e < chars.len() && chars[e] != '\n' { e += 1; }
        let line: String = chars[k..e].iter().collect(); let ahead: String = chars[e..(e + 300).min(chars.len())].iter().collect();
        let toc = line.contains("....") || ahead.contains("....") || Regex::new(r"\s\d{1,3}\s*$").map(|r| r.is_match(line.trim_end())).unwrap_or(false);   // 목차 줄('Facts.' 다음 줄에 '...... 6') 제외
        pre_ok && (e - k) <= 80 && !toc
    };
    let find = |pat: &str, from: usize| -> Option<usize> { let p = norm(pat); if p.is_empty() || sq.len() < p.len() { return None; } (from..=sq.len() - p.len()).find(|&i| sq[i..i + p.len()] == p[..] && is_heading(i, p.len())) };
    let cut = |start_pat: &[&str], end_pat: &[&str], label: &str| -> Option<(String, String)> {
        let st = start_pat.iter().filter_map(|p| find(p, 0).map(|i| i + norm(p).len())).min()?;
        let en = end_pat.iter().filter_map(|p| find(p, st + 40)).min().unwrap_or(sq.len());
        if en <= st { return None; }
        let a = map[st]; let b = if en >= map.len() { chars.len() } else { map[en] };
        Some((label.to_string(), chars[a..b].iter().collect::<String>().trim().to_string()))
    };
    let counts = ["COUNT I", "COUNT ONE", "COUNT 1", "FIRST CAUSE OF ACTION", "FIRST CLAIM", "CLAIMS FOR RELIEF", "CAUSES OF ACTION", "CLAIM FOR RELIEF", "PRAYER FOR RELIEF", "WHEREFORE", "DEMAND FOR JURY TRIAL", "JURY DEMAND", "REQUEST FOR RELIEF"];
    let r = cut(&["STATEMENT OF FACTS", "FACTUAL ALLEGATIONS", "FACTUAL BACKGROUND", "GENERAL ALLEGATIONS", "ALLEGATIONS COMMON TO ALL COUNTS", "STATEMENT OF THE FACTS", "FACTS COMMON TO ALL", "FACTS", "BACKGROUND FACTS", "RELEVANT FACTS", "SUBSTANTIVE ALLEGATIONS"], &counts, "Statement of Facts")   // 소장
        .or_else(|| cut(&["THE GRAND JURY CHARGES", "THE GRAND JURY FURTHER CHARGES", "OVERT ACTS", "MANNER AND MEANS"], &["FORFEITURE ALLEGATION", "FORFEITURE", "A TRUE BILL", "FOREPERSON"], "Indictment"))   // 기소장
        .or_else(|| cut(&["STATEMENT OF THE CASE", "STATEMENT OF FACTS", "FACTUAL BACKGROUND", "BACKGROUND"], &["ARGUMENT", "LEGAL STANDARD", "STANDARD OF REVIEW", "DISCUSSION", "SUMMARY OF ARGUMENT", "LEGAL ANALYSIS"], "Statement of the Case"))   // 브리프·모션
        .or_else(|| cut(&["NATURE OF THE ACTION", "NATURE OF THE CASE", "PRELIMINARY STATEMENT"], &counts, "Nature of the Action"));   // 'ALLEGATIONS'·'COMPLAINT' 단독은 본문 언급과 구분 불가 → 위치 규칙(en_body_fallback)에 맡김
    let r = match r { Some(r) if r.1.chars().filter(|c| c.is_alphanumeric()).count() >= 200 => r, _ => return Ok(None) };
    let re_item = Regex::new(r"^\s*(\d{1,3}\.|[a-z]\.|\(\d{1,3}\)|\([a-z]\)|[IVX]+\.)")?;
    let mut out = String::new(); let mut prev_open = false;
    for line in r.1.lines() { let l = line.trim(); if l.is_empty() { if !out.is_empty() && !out.ends_with("\n\n") { out.push('\n'); } prev_open = false; continue; }
        if prev_open && !re_item.is_match(l) { out.push(' '); } else if !out.is_empty() && !out.ends_with('\n') { out.push('\n'); }
        out.push_str(l); prev_open = !(l.ends_with('.') || l.ends_with(':') || l.ends_with(';')); }
    let body = Regex::new(r"\n{3,}")?.replace_all(&out, "\n\n").to_string();
    Ok(Some((r.0, body.chars().take(6000).collect())))
}
/// 영어 문서 표제 없음: 첫 번호 문단('1.')·'COMES NOW'·'Plaintiff … alleges' 부터 WHEREFORE·PRAYER·COUNT·서명·송달증명 전까지(캡션·서명 제거). 그것도 없으면 본문 앞 6,000자.
pub fn en_body_fallback(text: &str) -> (String, String) {
    let re_stamp = Regex::new(r"(?mi)^\s*Case\s+\d[:\d]*-[a-z]{2}-\d+[^\n]*$|^\s*(?:Page\s+\d+\s+of\s+\d+|-\s*\d+\s*-|\d{1,3})\s*$").unwrap(); let re_lineno = Regex::new(r"(?m)^\s{0,6}\d{1,2}\s{2,}").unwrap();
    let t = re_lineno.replace_all(&re_stamp.replace_all(text, ""), "").to_string();
    let re_start = Regex::new(r"(?mi)^\s*1\.\s+\S|^\s*COMES?\s+NOW\b|^\s*(?:Plaintiffs?|Petitioners?|Defendants?)\b[^\n]{0,80}\b(?:alleges?|states?|avers?|complains?|petitions?)\b").unwrap();
    let re_end = Regex::new(r"(?mi)^\s*(?:WHEREFORE|PRAYER\s+FOR\s+RELIEF|REQUEST\s+FOR\s+RELIEF|COUNT\s+(?:I|ONE|1)\b|FIRST\s+(?:CAUSE|CLAIM)|CAUSES?\s+OF\s+ACTION|CLAIMS?\s+FOR\s+RELIEF|Respectfully\s+submitted|CERTIFICATE\s+OF\s+SERVICE|DEMAND\s+FOR\s+JURY|JURY\s+DEMAND|A\s+TRUE\s+BILL)").unwrap();
    let st = re_start.find(&t).map(|m| m.start()).unwrap_or(0); let en = re_end.find_at(&t, (st + 200).min(t.len())).map(|m| m.start()).unwrap_or(t.len());
    let body = t[st..en.max(st)].trim();
    let re_item = Regex::new(r"^\s*(\d{1,3}\.|[a-z]\.|\(\d{1,3}\)|\([a-z]\)|[IVX]+\.)").unwrap(); let mut out = String::new(); let mut prev_open = false;
    for line in body.lines() { let l = line.trim(); if l.is_empty() { if !out.is_empty() && !out.ends_with("\n\n") { out.push('\n'); } prev_open = false; continue; }
        if prev_open && !re_item.is_match(l) { out.push(' '); } else if !out.is_empty() && !out.ends_with('\n') { out.push('\n'); }
        out.push_str(l); prev_open = !(l.ends_with('.') || l.ends_with(':') || l.ends_with(';')); }
    let out = Regex::new(r"\n{3,}").unwrap().replace_all(&out, "\n\n").to_string();
    ((if st > 0 { "Body (caption and prayer removed)" } else { "Full text" }).to_string(), out.chars().take(6000).collect())
}
pub fn pdf_section_by_heading(text: &str) -> Result<Option<(String, String)>> {
    if is_english(text) { return pdf_section_by_heading_en(text); }
    // 표제는 글자 사이를 띄우는 관행("청 구 원 인") → 공백 제거본에서 위치를 찾고 원문 인덱스로 되돌림
    let chars: Vec<char> = text.chars().collect(); let mut map = Vec::with_capacity(chars.len()); let mut sq: Vec<char> = Vec::with_capacity(chars.len());
    for (i, c) in chars.iter().enumerate() { if !c.is_whitespace() { sq.push(*c); map.push(i); } }
    let find = |pat: &str, from: usize| -> Option<usize> { let p: Vec<char> = pat.chars().collect(); if p.is_empty() || sq.len() < p.len() { return None; } (from..=sq.len() - p.len()).find(|&i| sq[i..i + p.len()] == p[..]) };   // 문자 단위 검색(바이트 경계 문제 없음)
    let cut = |start_pat: &[&str], end_pat: &[&str], label: &str| -> Option<(String, String)> {
        let st = start_pat.iter().filter_map(|p| find(p, 0).map(|i| i + p.chars().count())).min()?;
        let en = end_pat.iter().filter_map(|p| find(p, st)).min().unwrap_or(sq.len());
        if en <= st { return None; }
        let a = map[st]; let b = if en >= map.len() { chars.len() } else { map[en] };
        let body: String = chars[a..b].iter().collect();
        Some((label.to_string(), body.trim().to_string()))
    };
    let r = cut(&["청구원인에대한답변", "청구원인에관한답변"], &["입증방법", "첨부서류", "증거방법"], "청구원인에 대한 답변")   // 답변서(더 긴 표제를 먼저)
        .or_else(|| cut(&["청구원인"], &["입증방법", "첨부서류", "증거방법", "관할법원"], "청구원인"))
        .or_else(|| cut(&["공소사실", "범죄사실"], &["첨부서류", "증거", "적용법조"], "공소사실"))
        .or_else(|| cut(&["고소사실", "고소이유", "고소원인"], &["입증방법", "첨부서류", "증거자료", "관련사건"], "고소사실"))   // 고소장
        .or_else(|| cut(&["신청이유", "신청원인"], &["소명방법", "입증방법", "첨부서류"], "신청이유"))   // 가압류·가처분·지급명령
        .or_else(|| cut(&["항소이유", "상고이유", "항고이유", "재항고이유"], &["입증방법", "첨부서류", "증거방법"], "항소이유"))
        .or_else(|| cut(&["심판청구이유", "청구이유", "심판청구의이유"], &["입증방법", "첨부서류", "증거서류"], "심판청구이유"))   // 행정심판·조세심판 청구서
        .or_else(|| cut(&["기초사실", "사실관계"], &["입증방법", "첨부서류", "판단"], "사실관계"))
        .or_else(|| cut(&["특허청구범위", "청구범위"], &["발명의설명", "발명의상세한설명", "도면의간단한설명", "요약서", "요약"], "청구범위"))   // 특허 명세서·공보
        .or_else(|| cut(&["요약"], &["대표도", "청구범위", "기술분야"], "요약"));
    let r = match r { Some(r) if r.1.chars().filter(|c| !c.is_whitespace()).count() >= 60 => r, _ => return Ok(None) };
    // 증거 참조·페이지 번호 등 잡음 제거 + PDF 줄바꿈 복원(문장 중간 줄바꿈은 붙이고, 번호 항목 앞은 유지)
    let re_junk = Regex::new(r"(?s)[{｛][^}｝]*호증[^}｝]*[}｝]|\(갑\s*제?\s*\d+호증[^)]*\)|\[갑\s*제?\s*\d+호증[^\]]*\]")?;
    let body = re_junk.replace_all(&r.1, "").to_string();
    let re_item = Regex::new(r"^\s*(\d{1,2}\.|[가-하]\.|\(\d{1,2}\)|[①-⑳])")?; let re_page = Regex::new(r"^\s*-?\s*\d{1,3}\s*-?\s*$")?;
    let mut out = String::new(); let mut prev_open = false;
    for line in body.lines() { let l = line.trim(); if l.is_empty() || re_page.is_match(l) { if !out.is_empty() && !out.ends_with("\n\n") { out.push('\n'); } prev_open = false; continue; }
        if prev_open && !re_item.is_match(l) { out.push(' '); } else if !out.is_empty() && !out.ends_with('\n') { out.push('\n'); }
        out.push_str(l); prev_open = !(l.ends_with('.') || l.ends_with('。') || l.ends_with(':') || l.ends_with("다") && l.len() < 12); }
    let body = Regex::new(r"\n{3,}")?.replace_all(&out, "\n\n").to_string();
    Ok(Some((r.0, body.chars().take(6000).collect())))
}
pub fn pdf_facts(bytes: &[u8]) -> Result<(String, String)> {   // 규칙만(엔진 없이): 표제 없으면 전문
    let text = pdf_text(bytes)?;
    Ok(pdf_section_by_heading(&text)?.unwrap_or_else(|| ("전문".to_string(), text.trim().chars().take(6000).collect())))
}
const PARA_PROTO: &str = include_str!("../assets/para_proto.json");   // 공단 작성례 1,137건에서 구운 문단 유형 프로토타입(FACT/PRAYER/EVID/HEAD/TAIL)
#[derive(Deserialize)] struct Proto { labels: Vec<String>, vectors: Vec<Vec<f32>> }
/// 문단 나누기(빈 줄 또는 번호 항목 앞), 15자 미만·쪽번호 제외, '귀중' 이후(해설·서명) 제외
pub fn split_paragraphs(text: &str) -> Vec<String> {
    let mut t = text.to_string();
    if is_english(&t) { let re_stamp = Regex::new(r"(?mi)^\s*Case\s+\d[:\d]*-[a-z]{2}-\d+[^\n]*$|^\s*(?:Page\s+\d+\s+of\s+\d+|-\s*\d+\s*-)\s*$").unwrap(); let re_lineno = Regex::new(r"(?m)^\s{0,6}\d{1,2}\s{2,}").unwrap(); t = re_lineno.replace_all(&re_stamp.replace_all(&t, ""), "").to_string(); }
    if let Some(m) = Regex::new(r"\n[^\n]*귀\s*중[^\n]*\n").ok().and_then(|re| re.find(&t)) { t.truncate(m.end()); }
    let re_item = Regex::new(r"^\s*(?:\d{1,3}\.|[가-하]\.|[a-z]\.|\(\d{1,3}\)|\([a-z]\)|[①-⑳]|[IVX]+\.)").unwrap(); let re_page = Regex::new(r"^\s*-?\s*\d{1,3}\s*-?\s*$").unwrap();
    let mut paras: Vec<String> = Vec::new(); let mut cur: Vec<&str> = Vec::new();
    let flush = |cur: &mut Vec<&str>, paras: &mut Vec<String>| { if !cur.is_empty() { let p = cur.join(" "); if p.chars().count() >= 15 && !re_page.is_match(&p) { paras.push(p); } cur.clear(); } };
    for line in t.lines() { let l = line.trim();
        if l.is_empty() { flush(&mut cur, &mut paras); continue; }                       // 빈 줄 = 문단 경계
        if re_item.is_match(l) && !cur.is_empty() { flush(&mut cur, &mut paras); }       // 번호 항목 시작 = 새 문단
        cur.push(l); }
    flush(&mut cur, &mut paras); paras
}
impl Engine {
    /// 소장·공소장·답변서·준비서면 PDF → 사실 문단. 1) 표제 규칙이 맞으면 그 구간, 2) 아니면 문단마다 임베딩→프로토타입 코사인으로 FACT 문단만 선별(생성 0, 문단별 유사도 반환).
    pub fn pdf_facts_smart(&mut self, bytes: &[u8]) -> Result<(String, String, Vec<(String, String, f32)>)> { self.pdf_facts_smart_ocr(bytes, None) }
    /// ocr_helper 가 있으면 텍스트 레이어 없는 PDF 를 OCR 로 읽음
    pub fn pdf_facts_smart_ocr(&mut self, bytes: &[u8], ocr_helper: Option<&Path>) -> Result<(String, String, Vec<(String, String, f32)>)> {
        // 텍스트 레이어 추출 실패(스캔본·깨진 글리프·폰트 오류·파서 패닉) → 전부 OCR 로. 텍스트는 뽑혔는데 사실 구간을 못 찾으면 OCR 로 한 번 더(텍스트 레이어가 부분적인 스캔본).
        let helper = ocr_helper.unwrap_or(Path::new("ocr/nike_ocr"));
        let (text, mut ocr_used) = match pdf_text(bytes) { Ok(t) => (t, false), Err(e) => { let msg = e.to_string(); (ocr_pdf(bytes, helper).map_err(|oe| if msg == "NO_TEXT_LAYER" { oe } else { anyhow::anyhow!("{msg}; {oe}") })?, true) } };
        let (lab, body, detail) = match self.facts_from_text(&text) { Ok(r) => r, Err(e) if !ocr_used => { let t2 = ocr_pdf(bytes, helper).map_err(|_| e)?; ocr_used = true; self.facts_from_text(&t2)? } Err(e) => return Err(e) };
        Ok((if ocr_used { format!("{lab} · OCR") } else { lab }, body, detail))
    }
    pub fn facts_from_text(&mut self, text: &str) -> Result<(String, String, Vec<(String, String, f32)>)> {
        let text = text.replace('\u{0C}', "\n");
        if let Some((lab, body)) = pdf_section_by_heading(&text)? { return Ok((lab, body, vec![])); }
        let en = is_english(&text);
        if en {   // 영어: 표제가 없으면 위치 규칙(캡션·서명 제거)만 — 문단 임베딩 분류는 사실/청구 구분력이 낮아(홀드아웃 48%) 쓰지 않음
            let (lab, body) = en_body_fallback(&text); if body.chars().count() < 200 { anyhow::bail!("No factual section found. Paste the facts directly."); } return Ok((lab, body, vec![]));
        }
        let proto: Proto = serde_json::from_str(PARA_PROTO)?; let paras = split_paragraphs(&text);   // 한국어 전용(영어는 위에서 규칙으로 끝남)
        let mut picked = Vec::new(); let mut detail = Vec::new();
        for p in paras.iter().take(400) {
            let v = self.embed(p)?;
            let mut best = (0usize, -1.0f32);
            for (i, pv) in proto.vectors.iter().enumerate() { let c: f32 = pv.iter().zip(&v).map(|(a, b)| a * b).sum(); if c > best.1 { best = (i, c); } }
            let lab = proto.labels[best.0].clone(); detail.push((lab.clone(), p.chars().take(80).collect(), best.1));
            if lab == "FACT" { picked.push(p.clone()); }
        }
        if picked.is_empty() { anyhow::bail!("사실 서술 문단을 찾지 못했습니다(문단 {}개). 내용을 직접 붙여 넣어 주세요.", paras.len()); }
        let body: String = picked.join("\n"); Ok((format!("사실 문단 {}개(자동 분류)", picked.len()), body.chars().take(6000).collect(), detail))
    }
}


/// iPadOS 내장 OCR: PDFKit 으로 페이지 렌더(2.5x) → Vision VNRecognizeTextRequest(한국어+영어, accurate) → 위→아래·좌→우 줄 정렬. 맥 헬퍼(nike_ocr.swift)와 동일 규칙, 전부 기기 안.
#[cfg(target_os = "ios")]
pub fn ocr_pdf_ios(bytes: &[u8]) -> Result<String> {
    use objc2::{class, msg_send, runtime::{AnyObject, Bool}};
    use objc2_foundation::{NSRect as CGRect, NSSize as CGSize, NSArray, NSData, NSString};
    #[link(name = "PDFKit", kind = "framework")] unsafe extern "C" {}
    #[link(name = "Vision", kind = "framework")] unsafe extern "C" {}
    unsafe {
        let data = NSData::with_bytes(bytes);
        let doc: *mut AnyObject = msg_send![class!(PDFDocument), alloc]; let doc: *mut AnyObject = msg_send![doc, initWithData: &*data];
        if doc.is_null() { anyhow::bail!("PDF 를 열 수 없습니다."); }
        let n: usize = msg_send![doc, pageCount]; let mut out = String::new();
        let langs = NSArray::from_retained_slice(&[NSString::from_str("ko-KR"), NSString::from_str("en-US")]);
        for i in 0..n { objc2::rc::autoreleasepool(|_| {   // 페이지마다 풀: thumbnail/results 등 autorelease 객체 즉시 해제
            let page: *mut AnyObject = msg_send![doc, pageAtIndex: i]; if page.is_null() { return; }
            let b: CGRect = msg_send![page, boundsForBox: 0isize];   // kPDFDisplayBoxMediaBox
            let size = CGSize { width: b.size.width * 2.5, height: b.size.height * 2.5 };
            let img: *mut AnyObject = msg_send![page, thumbnailOfSize: size, forBox: 0isize];   // UIImage(흰 배경 렌더)
            if img.is_null() { return; }
            let cg: *mut std::ffi::c_void = msg_send![img, CGImage]; if cg.is_null() { return; }
            let req: *mut AnyObject = msg_send![class!(VNRecognizeTextRequest), new];
            let _: () = msg_send![req, setRecognitionLevel: 0isize];   // accurate
            let _: () = msg_send![req, setRecognitionLanguages: &*langs];
            let _: () = msg_send![req, setUsesLanguageCorrection: Bool::YES];
            let opts: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let handler: *mut AnyObject = msg_send![class!(VNImageRequestHandler), alloc]; let handler: *mut AnyObject = msg_send![handler, initWithCGImage: cg, options: opts];
            let reqs: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: req];
            let mut err: *mut AnyObject = std::ptr::null_mut(); let ok: Bool = msg_send![handler, performRequests: reqs, error: &mut err];
            let mut items: Vec<(f64, f64, String)> = Vec::new();
            if ok.as_bool() {
                let results: *mut AnyObject = msg_send![req, results];
                let cnt: usize = if results.is_null() { 0 } else { msg_send![results, count] };
                for j in 0..cnt {
                    let ob: *mut AnyObject = msg_send![results, objectAtIndex: j]; let bb: CGRect = msg_send![ob, boundingBox];
                    let cands: *mut AnyObject = msg_send![ob, topCandidates: 1usize]; let c: *mut AnyObject = msg_send![cands, firstObject]; if c.is_null() { continue; }
                    let s: *mut AnyObject = msg_send![c, string]; if s.is_null() { continue; }
                    let text = (&*(s as *const NSString)).to_string();
                    items.push((bb.origin.y + bb.size.height / 2.0, bb.origin.x, text));
                }
            }
            let _: () = msg_send![handler, release]; let _: () = msg_send![req, release];
            items.sort_by(|a, b| { if (a.0 - b.0).abs() > 0.008 { b.0.partial_cmp(&a.0).unwrap() } else { a.1.partial_cmp(&b.1).unwrap() } });   // 위→아래, 좌→우
            let mut lines: Vec<String> = Vec::new(); let mut cur: Vec<String> = Vec::new(); let mut last_y = 2.0f64;
            for (y, _x, t) in items { if (y - last_y).abs() > 0.008 { if !cur.is_empty() { lines.push(cur.join(" ")); } cur = Vec::new(); last_y = y; } cur.push(t); }
            if !cur.is_empty() { lines.push(cur.join(" ")); }
            out.push_str(&lines.join("\n")); out.push_str("\n\u{0C}\n");
        }); }
        let _: () = msg_send![doc, release];
        if out.chars().filter(|c| !c.is_whitespace()).count() < 40 { anyhow::bail!("OCR로도 글자를 찾지 못했습니다."); }
        Ok(out)
    }
}

/// Windows 내장 OCR: PDF → WinRT PdfDocument 로 페이지 렌더(2.5x) → Windows.Media.Ocr(한국어 언어팩; 없으면 사용자 언어) → 줄 텍스트. 전부 로컬.
#[cfg(windows)]
pub fn ocr_pdf_windows(bytes: &[u8]) -> Result<String> {
    use windows::{core::HSTRING, Data::Pdf::{PdfDocument, PdfPageRenderOptions}, Globalization::Language, Graphics::Imaging::BitmapDecoder, Media::Ocr::OcrEngine, Storage::Streams::{DataWriter, InMemoryRandomAccessStream}};
    let stream = InMemoryRandomAccessStream::new()?;
    { let w = DataWriter::CreateDataWriter(&stream)?; w.WriteBytes(bytes)?; w.StoreAsync()?.get()?; w.FlushAsync()?.get()?; w.DetachStream()?; }
    stream.Seek(0)?;
    let doc = PdfDocument::LoadFromStreamAsync(&stream)?.get().map_err(|e| anyhow::anyhow!("pdf 열기 실패: {e}"))?;
    let engine = Language::CreateLanguage(&HSTRING::from("ko-KR")).ok().and_then(|l| OcrEngine::TryCreateFromLanguage(&l).ok())
        .or_else(|| OcrEngine::TryCreateFromUserProfileLanguages().ok())
        .ok_or_else(|| anyhow::anyhow!("Windows OCR 엔진을 만들 수 없습니다. 설정 > 시간 및 언어 > 언어에서 한국어 언어팩(광학 문자 인식)을 설치하세요."))?;
    let maxdim = OcrEngine::MaxImageDimension().unwrap_or(2600) as f32;
    let mut out = String::new();
    for i in 0..doc.PageCount()? {
        let page = doc.GetPage(i)?; let size = page.Size()?;
        let scale = (2.5f32).min((maxdim - 8.0) / size.Width.max(size.Height));
        let ps = InMemoryRandomAccessStream::new()?; let opts = PdfPageRenderOptions::new()?; opts.SetDestinationWidth((size.Width * scale) as u32)?; opts.SetDestinationHeight((size.Height * scale) as u32)?;
        page.RenderWithOptionsToStreamAsync(&ps, &opts)?.get()?; ps.Seek(0)?;
        let dec = BitmapDecoder::CreateAsync(&ps)?.get()?; let bmp = dec.GetSoftwareBitmapAsync()?.get()?;
        let res = engine.RecognizeAsync(&bmp)?.get()?;
        for line in res.Lines()? { out.push_str(&line.Text()?.to_string()); out.push('\n'); }
        out.push_str("\n\u{0C}\n");
    }
    Ok(out)
}
