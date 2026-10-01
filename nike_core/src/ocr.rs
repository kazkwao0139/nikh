//! 스캔본 OCR — macOS: 동봉 Vision 헬퍼(nike_ocr) 프로세스 / iPadOS: PDFKit+Vision 인프로세스 / Windows: WinRT Windows.Media.Ocr. 전부 기기 안.
use anyhow::Result;
use std::path::Path;

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
