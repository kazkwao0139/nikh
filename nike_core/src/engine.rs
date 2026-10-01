//! 엔진 — 팩 로드(mmap int8 + zstd 텍스트 저장소), bge-m3 임베딩, 코사인 검색, 판례 단위 집계, 심급 체인.
use anyhow::Result;
use memmap2::Mmap;
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};
use tokenizers::Tokenizer;

#[derive(Deserialize)]
struct Chunk {
    i: String,
    s: String,
    t: String,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Record {
    pub id: String,
    #[serde(rename = "사건명", default)]
    pub title: String,
    #[serde(rename = "사건번호", default)]
    pub caseno: String,
    #[serde(rename = "선고일자", default)]
    pub date: String,
    #[serde(rename = "법원", default)]
    pub court: Option<String>,
    #[serde(rename = "종류", default)]
    pub kind: Option<String>,
    #[serde(rename = "심급", default)]
    pub level: Option<String>,
    #[serde(rename = "적용법률", default)]
    pub laws: Vec<String>,
    #[serde(rename = "결과", default)]
    pub result: Vec<String>,
    #[serde(rename = "인정금액", default)]
    pub amount: Option<i64>,
    #[serde(rename = "참조판례", default)]
    pub refs: String,
    #[serde(rename = "판시사항", default)]
    pub issue: String,
    #[serde(rename = "판결요지", default)]
    pub summary: String,
    #[serde(rename = "전문", default)]
    pub full: String,
    #[serde(default)]
    pub prev_no: Option<String>,
    #[serde(default)]
    pub prev_txt: Option<String>,
    #[serde(default)]
    pub first_txt: Option<String>, // v2 meta 사전계산
    #[serde(rename = "원문", default)]
    pub url: Option<String>, // 특허공보: Google Patents 링크
    #[serde(rename = "키프리스", default)]
    pub kipris: Option<String>, // 특허공보: KIPRIS(DOI) 링크
}

/// 레코드 id → 원문 링크 (판례는 순수 번호, 나머지는 '접두:번호')
pub fn source_url(id: &str, r: &Record) -> String {
    if let Some(u) = &r.url {
        return u.clone();
    }
    match id.split_once(':') {
        None => format!("https://www.law.go.kr/LSW/precInfoP.do?precSeq={}", id),
        Some(("expc", n)) => format!("https://www.law.go.kr/LSW/expcInfoP.do?expcSeq={}", n),
        Some(("decc", n)) => format!("https://www.law.go.kr/LSW/deccInfoP.do?deccSeq={}", n),
        Some(("detc", n)) => format!("https://www.law.go.kr/LSW/detcInfoP.do?detcSeq={}", n),
        Some(("trty", n)) => format!("https://www.law.go.kr/LSW/trtyInfoP.do?trtySeq={}", n),
        Some(("tax", n)) => format!("https://www.law.go.kr/LSW/precInfoR.do?precSeq={}", n),
        Some(("tt", n)) => format!("https://www.law.go.kr/LSW/specialDeccInfoP.do?ttSpecialDeccSeq={}", n), // 공개 페이지(API 키 불필요)
        Some((t @ ("ftc" | "kcc" | "iaciac" | "ecc" | "eiac" | "oclt"), n)) => format!("https://www.law.go.kr/LSW/{}InfoR.do?{}Seq={}", t, t, n), // 공개 페이지(본문 직접 렌더)
        Some((t, n)) => format!("https://www.law.go.kr/LSW/{}InfoP.do?{}Seq={}", t, t, n), // ppc·acr·nlrc 결정문 공개 페이지
    }
}

#[derive(Serialize, Clone)]
pub struct ChainNode {
    pub id: String,
    pub level: Option<String>,
    pub court: Option<String>,
    pub caseno: String,
    pub date: String,
    pub result: Vec<String>,
    pub issue: String,
    pub summary: String,
    pub refs: String,
    pub laws: String,
    pub full: String,
    pub prev_text: Option<String>,
    pub prev_missing: bool,
    pub first_text: Option<String>,
    pub url: String,
}

#[derive(Serialize)]
pub struct Hit {
    pub score: f32,
    pub title: String,
    pub caseno: String,
    pub court: Option<String>,
    pub date: String,
    pub level: Option<String>,
    pub kind: Option<String>,
    pub result: Vec<String>,
    pub laws: Vec<String>,
    pub sec: String,
    pub snippet: String,
    pub issue: String,
    pub url: String,
    pub chain: Vec<ChainNode>,
    pub kipris: Option<String>,
}

struct PackData {
    emb: Mmap,
    scale: Vec<f32>,
    n: usize,
    key: String,
    recs: Option<ZstdStore>,
    chunks: Option<ZstdStore>,
    texts: Vec<String>,
} // texts: v1 팩만(팩 내부 인덱스)

/// v2 텍스트 저장소: zstd 블록 mmap + 블록 표. 마지막 블록 1개 캐시. 검색엔 안 쓰이고 표시할 때만 해제.
pub struct ZstdStore {
    mmap: Mmap,
    blocks: Vec<(u64, u64)>,
    cache: std::sync::Mutex<Option<(u32, Vec<u8>)>>,
}
impl ZstdStore {
    fn open(zst: &Path, blocks: Vec<(u64, u64)>) -> Result<Self> {
        Ok(ZstdStore { mmap: unsafe { Mmap::map(&File::open(zst)?)? }, blocks, cache: std::sync::Mutex::new(None) })
    }
    fn get(&self, block: u32, off: u32, len: u32) -> String {
        let mut c = self.cache.lock().unwrap();
        if c.as_ref().map(|(b, _)| *b != block).unwrap_or(true) {
            let (o, l) = self.blocks[block as usize];
            let raw = &self.mmap[o as usize..(o + l) as usize];
            let dec = zstd::stream::decode_all(raw).unwrap_or_default();
            *c = Some((block, dec));
        }
        let d = &c.as_ref().unwrap().1;
        String::from_utf8_lossy(&d[off as usize..(off + len) as usize]).to_string()
    }
}
#[derive(Deserialize)]
struct RecordIndex {
    blocks: Vec<(u64, u64)>,
    items: Vec<(String, u32, u32, u32)>,
}
#[derive(Deserialize)]
struct ChunkIndex {
    blocks: Vec<(u64, u64)>,
    items: Vec<(String, String, u32, u32, u32)>,
}
#[derive(Clone, Copy)]
struct BlockPos {
    pack: u16,
    block: u32,
    off: u32,
    len: u32,
}
/// 모드: 특허 팩(pat_*)만 / 특허 제외 / 전부
fn pack_in_mode(key: &str, mode: Option<&str>) -> bool {
    match mode {
        Some("patent") => key.starts_with("pat_"),
        Some("us") => key.starts_with("us_"),
        Some("case") => !key.starts_with("pat_") && !key.starts_with("us_"),
        Some(m) if m.starts_with("packs:") => m[6..].split(',').any(|k| k.trim() == key), // "packs:civil,criminal" → 그 팩만 검색
        _ => true,
    }
}
pub struct Engine {
    n: usize,
    dim: usize,
    packs: Vec<PackData>, // 도메인별 팩 (int8 n_k×dim), 청크 배열은 팩 순서로 이어 붙임
    pub loaded: Vec<String>,
    chunk_id: Vec<String>,
    chunk_sec: Vec<String>,
    chunk_pos: Vec<Option<BlockPos>>,                           // v2 팩: 청크 텍스트 위치
    rec_pos: HashMap<String, BlockPos>,                         // v2 팩: 원본 레코드 위치 (전문·요지는 여기서 지연 로드)
    recs: HashMap<String, Record>,                              // v1: 전체 / v2: meta (전문 없음)
    by_no: HashMap<String, String>,                             // 사건번호(공백 제거) → id
    prev_of: HashMap<String, (Option<String>, Option<String>)>, // id → (원심 사건번호, 원심 표기)
    next_of: HashMap<String, String>,
    tok: Tokenizer,
    sess: Session,
    re_first: Regex,
}

fn case_number_key(s: &str) -> String {
    s.split(',').next().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect()
}

impl Engine {
    pub fn load(pack: &Path, model_dir: &Path) -> Result<Self> {
        Self::load_packs(&[pack.to_path_buf()], model_dir)
    }

    /// 여러 팩 디렉터리를 이어 붙여 로드 (체크한 팩만). 각 팩: meta.json(n, dim), emb_i8.bin, emb_scale.bin, chunks.json, recs.jsonl
    pub fn load_packs(packs: &[PathBuf], model_dir: &Path) -> Result<Self> {
        Self::load_packs_with(packs, model_dir, &|_, _, _| {})
    }

    /// progress(done, total, 현재 팩 키): 팩 하나 읽기 전에 호출 (게이지용)
    pub fn load_packs_with(packs: &[PathBuf], model_dir: &Path, progress: &dyn Fn(usize, usize, &str)) -> Result<Self> {
        let mut dim = 0usize;
        let mut mems = Vec::new();
        let mut chunk_id = Vec::new();
        let mut chunk_sec = Vec::new();
        let mut v1_texts: Vec<String> = Vec::new();
        let mut recs: HashMap<String, Record> = HashMap::new();
        let mut loaded = Vec::new();
        let mut chunk_pos: Vec<Option<BlockPos>> = Vec::new();
        let mut rec_pos: HashMap<String, BlockPos> = HashMap::new();
        for (pi, pack) in packs.iter().enumerate() {
            progress(pi, packs.len(), &pack.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            let meta: serde_json::Value = serde_json::from_reader(File::open(pack.join("meta.json"))?)?;
            let n = meta["n"].as_u64().unwrap() as usize;
            let d = meta["dim"].as_u64().unwrap() as usize;
            if dim == 0 {
                dim = d;
            }
            anyhow::ensure!(dim == d, "dim mismatch in {:?}", pack);
            let emb = unsafe { Mmap::map(&File::open(pack.join("emb_i8.bin"))?)? };
            anyhow::ensure!(emb.len() == n * dim, "emb size mismatch in {:?}", pack);
            let sb = std::fs::read(pack.join("emb_scale.bin"))?;
            let scale: Vec<f32> = sb.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect(); // clippy 1.98 chunks_exact_to_as_chunks
            let key = meta["key"].as_str().unwrap_or("pack").to_string();
            let (rstore, cstore) = if pack.join("meta.jsonl").exists() {
                // v2: 텍스트는 zstd 블록, 메타만 상주
                let ci: ChunkIndex = serde_json::from_reader(BufReader::new(File::open(pack.join("chunks.idx"))?))?;
                anyhow::ensure!(ci.items.len() == n, "chunks.idx size mismatch in {:?}", pack);
                for (i, s_, b, o, l) in ci.items {
                    chunk_id.push(i);
                    chunk_sec.push(s_);
                    chunk_pos.push(Some(BlockPos { pack: pi as u16, block: b, off: o, len: l }));
                }
                let ri: RecordIndex = serde_json::from_reader(BufReader::new(File::open(pack.join("recs.idx"))?))?;
                for (i, b, o, l) in ri.items {
                    rec_pos.insert(i, BlockPos { pack: pi as u16, block: b, off: o, len: l });
                }
                for line in BufReader::new(File::open(pack.join("meta.jsonl"))?).lines() {
                    let r: Record = serde_json::from_str(&line?)?;
                    recs.insert(r.id.clone(), r);
                }
                (Some(ZstdStore::open(&pack.join("recs.zst"), ri.blocks)?), Some(ZstdStore::open(&pack.join("chunks.zst"), ci.blocks)?))
            } else {
                let chunks: Vec<Chunk> = serde_json::from_reader(BufReader::new(File::open(pack.join("chunks.json"))?))?;
                for (li, c) in chunks.into_iter().enumerate() {
                    chunk_id.push(c.i);
                    chunk_sec.push(c.s);
                    v1_texts.push(c.t);
                    chunk_pos.push(Some(BlockPos { pack: pi as u16, block: li as u32, off: 0, len: 0 }));
                } // v1: block = 팩 내부 인덱스
                for line in BufReader::new(File::open(pack.join("recs.jsonl"))?).lines() {
                    let r: Record = serde_json::from_str(&line?)?;
                    recs.insert(r.id.clone(), r);
                }
                (None, None)
            };
            mems.push(PackData { emb, scale, n, key: key.clone(), recs: rstore, chunks: cstore, texts: std::mem::take(&mut v1_texts) });
            loaded.push(key);
        }
        let n: usize = mems.iter().map(|p| p.n).sum();
        let re_prev = Regex::new(r"【\s*원심판결\s*】\s*([^\n]*)")?;
        let re_no = Regex::new(r"(\d{4}[가-힣]{1,2}\d+)")?;
        let re_first = Regex::new(r"【\s*제\s*1\s*심\s*판\s*결\s*】\s*([^\n]*)")?;
        let mut by_no = HashMap::new();
        let mut prev_of = HashMap::new();
        for (id, r) in &recs {
            by_no.insert(case_number_key(&r.caseno), id.clone());
            let (pno, ptxt) = if r.prev_txt.is_some() || r.prev_no.is_some() {
                (r.prev_no.clone(), r.prev_txt.clone())
            } else {
                match re_prev.captures(&r.full) {
                    Some(c) => {
                        let t = c[1].trim().chars().take(80).collect::<String>();
                        (re_no.captures(&t).map(|m| m[1].to_string()), Some(t))
                    }
                    None => (None, None),
                }
            };
            prev_of.insert(id.clone(), (pno, ptxt));
        }
        let mut next_of = HashMap::new();
        for (id, (pno, _)) in &prev_of {
            if let Some(p) = pno {
                if let Some(pid) = by_no.get(p) {
                    next_of.insert(pid.clone(), id.clone());
                }
            }
        }
        progress(packs.len(), packs.len() + 2, "tokenizer");
        let tok = Tokenizer::from_file(model_dir.join("tokenizer.json")).map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;
        progress(packs.len() + 1, packs.len() + 2, "onnx");
        // int8(양자화) 우선, 로드 실패(축소 빌드 onnxruntime 등)면 fp32 model.onnx 로 폴백
        let candidates: Vec<std::path::PathBuf> = ["model_int8.onnx", "model.onnx"].iter().map(|f| model_dir.join(f)).filter(|p| p.exists()).collect();
        if candidates.is_empty() {
            anyhow::bail!("onnx 모델 없음: {}", model_dir.display());
        }
        let mut sess_res = None;
        let mut last_err = String::new();
        for model_path in &candidates {
            match (|| -> Result<Session> {
                Session::builder()
                    .map_err(|e| anyhow::anyhow!("ort: {e}"))?
                    .with_optimization_level(GraphOptimizationLevel::Level3)
                    .map_err(|e| anyhow::anyhow!("ort: {e}"))?
                    .with_intra_threads(4)
                    .map_err(|e| anyhow::anyhow!("ort: {e}"))?
                    .commit_from_file(model_path)
                    .map_err(|e| anyhow::anyhow!("ort: {e}"))
            })() {
                Ok(s) => {
                    sess_res = Some(s);
                    break;
                }
                Err(e) => {
                    last_err = format!("{}: {e}", model_path.display());
                }
            }
        }
        let sess = match sess_res {
            Some(s) => s,
            None => anyhow::bail!("ort: {last_err}"),
        };
        Ok(Engine { n, dim, packs: mems, loaded, chunk_id, chunk_sec, chunk_pos, rec_pos, recs, by_no, prev_of, next_of, tok, sess, re_first })
    }

    /// bge-m3: CLS 풀링 + L2 정규화 (sentence-transformers 설정과 동일)
    pub fn embed(&mut self, q: &str) -> Result<Vec<f32>> {
        let enc = self.tok.encode(q, true).map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
        let mask: Vec<i64> = enc.get_attention_mask().iter().map(|&x| x as i64).collect();
        let len = ids.len();
        let ids_t = Tensor::from_array(([1usize, len], ids)).map_err(|e| anyhow::anyhow!("ort: {e}"))?;
        let mask_t = Tensor::from_array(([1usize, len], mask)).map_err(|e| anyhow::anyhow!("ort: {e}"))?;
        let out = self.sess.run(ort::inputs!["input_ids" => ids_t, "attention_mask" => mask_t]).map_err(|e| anyhow::anyhow!("ort: {e}"))?;
        let (shape, data) = out[0].try_extract_tensor::<f32>().map_err(|e| anyhow::anyhow!("ort: {e}"))?; // [1, len, dim] last_hidden_state
        let dim = shape[2] as usize;
        let mut v: Vec<f32> = data[..dim].to_vec(); // CLS = 위치 0
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        v.iter_mut().for_each(|x| *x /= norm);
        Ok(v)
    }

    fn cosine_scores(&self, v: &[f32], mode: Option<&str>) -> Vec<f32> {
        let dim = self.dim;
        let mut out = Vec::with_capacity(self.n);
        for p in &self.packs {
            if !pack_in_mode(&p.key, mode) {
                out.extend(std::iter::repeat_n(f32::NEG_INFINITY, p.n));
                continue;
            } // 모드 밖 팩은 계산 생략
            let emb = &p.emb[..];
            let part: Vec<f32> = (0..p.n)
                .into_par_iter()
                .map(|i| {
                    let row = &emb[i * dim..(i + 1) * dim];
                    let dot: f32 = row.iter().zip(v).map(|(&a, &b)| (a as i8) as f32 * b).sum();
                    dot * p.scale[i]
                })
                .collect();
            out.extend(part);
        }
        out
    }

    pub fn search(&mut self, q: &str, k: usize, filter: Option<(&str, &str)>) -> Result<Vec<Hit>> {
        self.search_mode(q, k, filter, None)
    }

    /// mode: Some("patent") = 특허 팩만, Some("case") = 특허 제외, None = 전부
    pub fn search_mode(&mut self, q: &str, k: usize, filter: Option<(&str, &str)>, mode: Option<&str>) -> Result<Vec<Hit>> {
        let v = self.embed(q)?;
        let s = self.cosine_scores(&v, mode);
        let mut order: Vec<usize> = (0..self.n).collect();
        // 상위 k*6 판례만 필요 → 부분 정렬
        let take = (k * 8).min(self.n);
        order.select_nth_unstable_by(take.saturating_sub(1), |&a, &b| s[b].partial_cmp(&s[a]).unwrap());
        order.truncate(take);
        order.sort_by(|&a, &b| s[b].partial_cmp(&s[a]).unwrap());
        let mut best: Vec<(String, f32, usize)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for i in order {
            if s[i] == f32::NEG_INFINITY {
                break;
            }
            let id = &self.chunk_id[i];
            if seen.insert(id.clone()) {
                best.push((id.clone(), s[i], i));
            }
            if best.len() >= k * 6 {
                break;
            }
        }
        let mut hits = Vec::new();
        for (id, sc, i) in best {
            let r = match self.recs.get(&id) {
                Some(r) => r,
                None => continue,
            };
            if let Some((key, val)) = filter {
                let hay: Vec<String> = match key {
                    "적용법률" => r.laws.clone(),
                    "결과" => r.result.clone(),
                    "종류" => vec![r.kind.clone().unwrap_or_default()],
                    "심급" => vec![r.level.clone().unwrap_or_default()],
                    _ => vec![],
                };
                let v2: String = val.chars().filter(|c| !c.is_whitespace()).collect();
                if !hay.iter().any(|h| h.chars().filter(|c| !c.is_whitespace()).collect::<String>().contains(&v2)) {
                    continue;
                }
            }
            hits.push(Hit {
                score: (sc * 1000.0).round() / 1000.0,
                title: r.title.clone(),
                caseno: r.caseno.clone(),
                court: r.court.clone(),
                date: r.date.clone(),
                level: r.level.clone(),
                kind: r.kind.clone(),
                result: r.result.clone(),
                laws: r.laws.iter().take(6).cloned().collect(),
                sec: self.chunk_sec[i].clone(),
                snippet: self.chunk_text(i).chars().take(400).collect(),
                issue: r.issue.clone(),
                url: source_url(&id, r),
                kipris: r.kipris.clone(),
                chain: if id.contains(':') && !id.starts_with("cap:") { vec![] } else { self.chain_of(&id) },
            });
            if hits.len() >= k {
                break;
            }
        }
        Ok(hits)
    }

    /// 청크 텍스트(발췌용): v2 는 zstd 블록에서 지연 해제
    pub fn chunk_text(&self, i: usize) -> String {
        match self.chunk_pos.get(i).copied().flatten() {
            Some(p) => {
                let pk = &self.packs[p.pack as usize];
                match pk.chunks.as_ref() {
                    Some(s) => s.get(p.block, p.off, p.len),
                    None => pk.texts.get(p.block as usize).cloned().unwrap_or_default(),
                }
            }
            None => String::new(),
        }
    }
    /// 원본 레코드 전체(전문·요지 포함): v2 는 recs.zst 에서 지연 해제
    pub fn full_record(&self, id: &str) -> Option<Record> {
        if let Some(p) = self.rec_pos.get(id) {
            let s = self.packs[p.pack as usize].recs.as_ref()?.get(p.block, p.off, p.len);
            return serde_json::from_str(&s).ok();
        }
        self.recs.get(id).cloned()
    }

    pub fn chain_of(&self, pid: &str) -> Vec<ChainNode> {
        let mut first = pid.to_string();
        while let Some((Some(pno), _)) = self.prev_of.get(&first) {
            match self.by_no.get(pno) {
                Some(p) if p != &first => first = p.clone(),
                _ => break,
            }
        }
        let mut out = Vec::new();
        let mut cur = Some(first);
        while let Some(c) = cur {
            if out.len() >= 4 || out.iter().any(|o: &ChainNode| o.id == c) {
                break;
            }
            let r = match self.full_record(&c) {
                Some(r) => r,
                None => break,
            };
            let (pno, ptxt) = self.prev_of.get(&c).cloned().unwrap_or((None, None));
            let first_txt = self
                .recs
                .get(&c)
                .and_then(|m| m.first_txt.clone())
                .or_else(|| self.re_first.captures(&r.full).map(|m| m[1].trim().chars().take(80).collect::<String>()));
            out.push(ChainNode {
                id: c.clone(),
                level: r.level.clone(),
                court: r.court.clone(),
                caseno: r.caseno.clone(),
                date: r.date.clone(),
                result: r.result.clone(),
                issue: r.issue.clone(),
                summary: r.summary.clone(),
                refs: r.refs.clone(),
                laws: r.laws.iter().take(12).cloned().collect::<Vec<_>>().join(" / "),
                full: r.full.clone(),
                prev_missing: pno.as_ref().map(|p| !self.by_no.contains_key(p)).unwrap_or(false),
                prev_text: ptxt,
                first_text: first_txt,
                url: source_url(&c, &r),
            });
            cur = self.next_of.get(&c).cloned();
        }
        out
    }

    pub fn stats(&self) -> (usize, usize) {
        (self.recs.len(), self.n)
    }

    /// 콜드스타트 제거: 임베딩 파일을 한 번 훑어 OS 페이지 캐시에 올리고(첫 검색의 디스크 읽기 선불), 모델 첫 추론(커널 초기화)도 미리.
    pub fn warm(&mut self) -> Result<()> {
        let _ = self.embed("준비")?;
        Ok(())
    } // ONNX 커널 초기화(수백 ms)

    /// 임베딩 파일 페이지-인 (엔진 락 없이 파일만 읽음). progress(읽은 바이트, 전체). 검색은 이와 무관하게 가능 — 캐시가 덜 된 동안만 느림.
    pub fn warm_files(packs: &[PathBuf], progress: &dyn Fn(u64, u64)) {
        use std::io::Read;
        let files: Vec<PathBuf> = packs.iter().map(|p| p.join("emb_i8.bin")).filter(|p| p.exists()).collect();
        let total: u64 = files.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        let mut done = 0u64;
        let mut buf = vec![0u8; 8 << 20];
        for f in files {
            if let Ok(mut fh) = File::open(&f) {
                while let Ok(n) = fh.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    done += n as u64;
                    std::hint::black_box(&buf[..n]);
                    progress(done, total);
                }
            }
        }
    }
}
