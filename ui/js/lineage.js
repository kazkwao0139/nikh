// 심급 체인·원심 링크
const COURT={'고법':'고등법원','지법':'지방법원','가정법원':'가정법원','행정법원':'행정법원','회생법원':'회생법원','특허법원':'특허법원','지원':'지원'};
function normCourt(s){ // "서울고법" → "서울고등법원", "부산고법 (창원)" → "부산고등법원 창원재판부", "수원지법 안양지원" 그대로
  let c=s.replace(/\s+/g,' ').trim(); const br=c.match(/\((.+?)\)/); c=c.replace(/\(.+?\)/,'').trim();
  c=c.replace(/고법$/,'고등법원').replace(/지법(?=\s|$)/,'지방법원').replace(/지법\s/,'지방법원 ');
  if(/^(서울|수원|대전|대구|부산|광주)고등법원$/.test(c)&&br) c+=' '+br[1]+'재판부';
  if(c==='대법원') return c; return c;
}
function prevLinks(txt){ // "서울고법 2023. 9. 7. 선고 2023나2001980, 2001997 판결"
  const m=(txt||'').match(/^(.+?)\s+\d{4}\.\s*\d{1,2}\.\s*\d{1,2}\.?\s*(?:선고|자)\s*(\(?[^\s]*\)?\s*\d{4}[가-힣]{1,2}\d+)/);
  if(!m) return '';
  const court=normCourt(m[1]), no=m[2].replace(/\(.+?\)\s*/,'').replace(/,.*$/,'').trim();
  return `<a href="https://casenote.kr/${encodeURIComponent(court)}/${encodeURIComponent(no)}" target="_blank" rel="noopener">케이스노트 ↗</a> · <a href="https://www.law.go.kr/precSc.do?query=${encodeURIComponent(no)}" target="_blank" rel="noopener">법제처 ↗</a> · <a href="https://www.scourt.go.kr/portal/information/finalruling/peruse/peruse_status.jsp" target="_blank" rel="noopener">판결서 열람 신청 ↗</a>`;
}
function lineage(chain){ // 항상 1심→2심→3심 사다리. 수록/미수록(링크)/번호 불명. 미국 판례는 Trial→Appellate→Highest
  const us=chain.some(c=>['Trial','Appellate','Highest'].includes(c.심급));
  if(us){ const byLv={}; chain.forEach(c=>{byLv[c.심급]=c}); const lvs=['Trial','Appellate','Highest'];
    const nodes=lvs.map(lv=>{ const c=byLv[lv]; if(c) return `<span class="n in">${lv} · ${esc(c.법원||'')} ${shortNo(c.사건번호||'')}${(c.결과||[]).length?' · '+esc((c.결과||[]).join('·')):''}</span>`;
      let txt=null; if(lv==='Appellate'&&byLv['Highest']) txt=byLv['Highest'].원심표기; if(lv==='Trial'&&byLv['Appellate']) txt=byLv['Appellate'].원심표기;
      if(txt){ const cite=(txt.split(',').slice(-1)[0]||'').trim(); return `<span class="n miss">${lv} · not in packs · ${esc(txt)} · <a href="https://www.courtlistener.com/?q=${encodeURIComponent('"'+cite+'"')}" target="_blank" rel="noopener">CourtListener ↗</a></span>`; }
      return `<span class="n miss">${lv} · —</span>`; });
    return '<span style="color:var(--faint);letter-spacing:.1em;margin-right:4px">HISTORY</span>'+nodes.join('<span class="arr">→</span>'); }
  const byLv={}; chain.forEach(c=>{byLv[c.심급]=c});
  const has3=!!byLv['3심'], has2=!!byLv['2심'], has1=!!byLv['1심'];
  const lvs=(has3||has2)?['1심','2심','3심']:['1심'];
  if(!has3&&has2) lvs.pop();                          // 2심까지만 존재하면 3심 칸 없음(상고 여부 불명)
  const nodes=lvs.map(lv=>{
    const c=byLv[lv];
    if(c) return `<span class="n in">${lv} · 수록 · ${esc(c.법원||'')} ${shortNo(c.사건번호||'')}${(c.결과||[]).length?' · '+esc((c.결과||[]).join('·')):''}</span>`;
    // 미수록: 표기를 알 수 있는 경우 링크, 아니면 번호 불명
    let txt=null;
    if(lv==='2심'&&byLv['3심']) txt=byLv['3심'].원심표기;
    if(lv==='1심'&&byLv['2심']) txt=byLv['2심'].제1심표기||byLv['2심'].원심표기;
    const short=(txt||'').replace(/\s+\d{4}\.\s*\d{1,2}\.\s*\d{1,2}\.?\s*(선고|자)\s*/,' ').replace(/\s*판결.*$/,'').replace(/,.*$/,'');
    return txt?`<span class="n miss">${lv} · 미수록 · ${esc(short)} · ${prevLinks(txt)||''}</span>`:`<span class="n miss">${lv} · 미수록 · 번호 불명${lv==='1심'?' (2심 판결문에 기재)':''}</span>`;
  });
  return '<span style="color:var(--faint);letter-spacing:.1em;margin-right:4px">계통</span>'+nodes.join('<span class="arr">→</span>');
}
