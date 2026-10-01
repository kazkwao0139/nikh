// 검색 실행·결과 렌더
const escapeHtml=s=>String(s).replace(/[&<>]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;'}[c]));
function chip(t,cls){return `<span class="chip ${cls||''}">${escapeHtml(t)}</span>`}
function resultCls(t){if(/승소|인용|파기/.test(t))return 'w';if(/패소|기각|각하|유죄/.test(t))return 'l';return ''}
function highlight(text,terms){return escapeHtml(text)}   // 형광펜(질의어 강조) 제거 — 사용자 결정 2026-09-27
const US_OPTYPE={'010combined':'Opinion','015unamimous':'Unanimous opinion','020lead':'Lead opinion','025plurality':'Plurality opinion','030concurrence':'Concurrence','035concurrenceinpart':'Concurrence in part','040dissent':'Dissent','050addendum':'Addendum','060remittitur':'Remittitur','070rehearing':'Rehearing','080onthemerits':'On the merits','090onmotiontostrike':'On motion to strike','100trialcourt':'Trial court','majority':'Majority opinion','concurrence':'Concurrence','dissent':'Dissent','rehearing':'Rehearing','on-the-merits':'On the merits','unanimous':'Unanimous opinion','concurring-in-part-and-dissenting-in-part':'Concurring in part, dissenting in part'};
function usPretty(t){ // CourtListener plain_text: 레이아웃 공백·들여쓰기·쪽머리 정리, 문장 중간 줄바꿈 병합(빈 줄=문단), 두 열 캡션·각주·줄끝 하이픈 처리
  const raw=t.replace(/\r/g,'').split('\n'); const lines=raw.map(l=>l.replace(/[ \t]+/g,' ').trim()); const out=[]; let buf='';
  const isHead=l=>/^(Page \d+ of \d+|C\.A\. No\. [\w.-]+|Case No\.? [\w:.-]+|No\. \d[\w.-]*|-\s*\d+\s*-|(January|February|March|April|May|June|July|August|September|October|November|December) \d{1,2}, \d{4})$/.test(l);   // 쪽머리·쪽번호·날짜 줄
  const isLayout=(r,l)=>/§/.test(l)||(r.trim().split(/ {6,}|\t/).length>=2&&l.length<90&&!/[.;,]$/.test(l));   // 6칸 이상 벌어진 두 열(양쪽 정렬 본문의 넓은 띄어쓰기는 제외)   // 두 열 캡션(당사자 § 사건번호)·오른쪽 정렬 라벨 → 줄 그대로
  const flush=()=>{ if(buf){ out.push(buf); buf=''; } };
  let pairs=0,single=0; for(let i=1;i<lines.length-1;i++){ if(lines[i]===''&&lines[i-1]&&lines[i+1]){ single++; } if(lines[i]&&lines[i-1]) pairs++; }
  const dbl=single>pairs;   // 법원 문서 더블스페이스(줄마다 빈 줄) → 문장이 안 끝난 빈 줄은 문단 경계로 안 봄
  let fn=false;   // 각주 문단 진행 중
  for(let i=0;i<lines.length;i++){ const l=lines[i];
    if(!l){ const nx=lines.slice(i+1).find(x=>x)||''; if(buf&&!fn&&!/[.:;!?”")\]]$/.test(buf)&&(dbl||/^[a-z]/.test(nx))) continue; flush(); fn=false; if(out.length&&out[out.length-1]!=='') out.push(''); continue; }   // 문장이 안 끝났고(더블스페이스 문서 또는 다음 줄이 소문자 시작) → 빈 줄 무시
    if(isHead(l)) continue;
    if(/^\d{1,2}$/.test(l)){ const nx=lines.slice(i+1).find(x=>x); if(nx&&!/^\d{1,2}$/.test(nx)){ flush(); buf='['+l+'] '; fn=true; continue; } continue; }   // 단독 숫자 줄 + 본문 = 각주 번호
    if(/:$/.test(l)&&l.length<30){ flush(); out.push(l); out.push(''); continue; }   // 'Dear Counsel:' 같은 인사·표제 줄
    if(isLayout(raw[i],l)){ flush(); fn=false; out.push(l); continue; }
    const shortCap=l.length<60&&l===l.toUpperCase()&&/[A-Z]{3}/.test(l);   // 제목 줄(대문자)은 단독 문단
    if(shortCap){ flush(); fn=false; out.push(l); out.push(''); continue; }
    const cont=buf&&!/[.:;!?”")\]]$/.test(buf)&&!/^(\d{1,2}|[A-Z]\.|[IVX]+\.|\(\d+\)|[a-z]\)|•|-)\s/.test(l);
    if(buf&&/^\[\d+\] $/.test(buf)) buf+=l;                                  // 각주 첫 줄
    else if(cont&&/-$/.test(buf)&&/^[a-z]/.test(l)) buf=buf.slice(0,-1)+l;   // 줄끝 하이픈 결합(un-\nlawful → unlawful)
    else if(cont) buf+=' '+l; else { flush(); buf=l; } }
  flush(); return out.join('\n').replace(/\n{3,}/g,'\n\n').trim(); }
function renderSections(full,us){ // 원문의 【】 헤더를 소제목으로만. 미국(CourtListener)은 의견 유형 코드 → 이름, 본문 정리
  const body=us?usPretty(full):full;
  return escapeHtml(body).replace(/【\s*([^】]{1,60}?)\s*】/g,(m,h)=>{ const parts=h.split('·').map(x=>x.trim()); const ty=US_OPTYPE[parts[0]]||(us?parts[0]:h.replace(/\s+/g,'')); return `</pre><h4>${escapeHtml(ty)}${us&&parts[1]?' · '+escapeHtml(parts[1]):''}</h4><pre>`; })
}
async function runSearch(){
  const text=queryInput.value.trim(); if(!text)return; if(MODE==='us'){ syncUS(); if(!SCOPE.us.size){ res.innerHTML='<div class="empty">Select your State first.</div>'; return; } } home.classList.add('up'); res.innerHTML='<div class="empty">'+(window.__ready?tr('검색 중…','Searching…'):tr('데이터 팩 여는 중… (첫 검색만 수 초 걸립니다)','Opening data packs… (only the first search takes a few seconds)'))+'</div>';
  $('#sbar').hidden=false; $('#go').disabled=true;
  const t0=performance.now(); const k=document.querySelector('input[name=k]:checked').value; let d; try{ if(window.__TAURI__&&window.__TAURI__.core){ d=await window.__TAURI__.core.invoke('search',{q:text,k:+k,filter:null,mode:modeParam()}); d.rows=d.rows.map(normalizeHit); } else { const r=await fetch('/api/search?q='+encodeURIComponent(text)+'&k='+k+'&mode='+encodeURIComponent(modeParam())+'&hybrid='+($('#hyb').checked?1:0)); d=await r.json(); if(d.rows.length&&d.rows[0].title!==undefined) d.rows=d.rows.map(normalizeHit); } }catch(e){ $('#sbar').hidden=true; $('#go').disabled=false; res.innerHTML='<div class="empty">'+tr('검색 실패: ','Search failed: ')+escapeHtml(String(e))+'</div>'; return; } $('#sbar').hidden=true; $('#go').disabled=false; const ms=Math.round(performance.now()-t0);
  window.__ready=true;
  const terms=[...new Set(text.replace(/[^가-힣a-zA-Z0-9\s]/g,' ').split(/\s+/).filter(w=>w.length>=2))].slice(0,12);
  if(!d.rows.length){res.innerHTML='<div class="empty">'+tr('결과 없음','No results')+'</div>';return}
  window.__rows=d.rows; window.__ms=ms; window.__terms=terms; window.__sortLaw=null; renderResults();
}
// Rust 엔진(영문 키) → 화면 키
function shortNo(no){ // 병합 사건번호(배상명령 초기사건 수십 건 등) → 앞 2건 + 외 N건, 전체는 title 로
  const a=String(no||'').split(',').map(s=>s.trim()).filter(Boolean); if(a.length<=3) return escapeHtml(a.join(', '));
  return `<span title="${escapeHtml(a.join(', '))}">${escapeHtml(a.slice(0,2).join(', '))} <span style="color:var(--faint)">${tr('외 '+(a.length-2)+'건','+'+(a.length-2)+' more')}</span></span>`; }
function normalizeHit(x){ if(x.사건명!==undefined) return x; return {score:x.score,사건명:x.title,사건번호:x.caseno,법원:x.court,선고일자:x.date,심급:x.level,종류:x.kind,결과:x.result,적용법률:x.laws,매칭구분:x.sec,매칭문단:x.snippet,판시사항:x.issue,원문:x.url,키프리스:x.kipris,
  chain:(x.chain||[]).map(c=>({id:c.id,심급:c.level,법원:c.court,사건번호:c.caseno,선고일자:c.date,결과:c.result,판시사항:c.issue,판결요지:c.summary,참조판례:c.refs,참조조문:c.laws,전문:c.full,원심표기:c.prev_text,원심미수록:c.prev_missing,제1심표기:c.first_text,원문:c.url}))}; }
const cleanCourt=s=>/^([가-힣] )+[가-힣]$/.test(s||'')?s.replace(/ /g,''):(s||'');   // '국 민 권 익 위 원 회' 처럼 글자마다 띄운 기관명 정리
function snippet(t,terms){ t=String(t||'').replace(/\s+/g,' '); if(t.length<=220) return t; let i=-1; for(const w of terms){ const j=t.indexOf(w); if(j>=0&&(i<0||j<i)) i=j; } const a=Math.max(0,(i<0?0:i)-80); return (a>0?'…':'')+t.slice(a,a+220)+(a+220<t.length?'…':''); }
function lawName(t){return (t||'').replace(/\s*제.*$/,'').trim()}   // "민법 제750조" → "민법"
function renderResults(){
  const rows=window.__rows, terms=window.__terms, ms=window.__ms, sortLaw=window.__sortLaw;
  const cnt={}; rows.forEach(x=>{[...new Set((x.적용법률||[]).map(lawName))].forEach(l=>{if(l)cnt[l]=(cnt[l]||0)+1})});
  const facets=Object.entries(cnt).sort((a,b)=>b[1]-a[1]).slice(0,14);
  const kinds={}; rows.forEach(x=>{if(x.종류)kinds[x.종류]=(kinds[x.종류]||0)+1});
  const ordered=sortLaw?[...rows].sort((a,b)=>((b.적용법률||[]).some(t=>lawName(t)===sortLaw)?1:0)-((a.적용법률||[]).some(t=>lawName(t)===sortLaw)?1:0)):rows;
  res.innerHTML=`<p class="meta">${rows.length}${tr('건',' results')} · ${ms} ms · ${sortLaw?`<b>${escapeHtml(sortLaw)}</b> ${tr('적용 판례 먼저','cases first')}`:($('#hyb').checked?tr('의미·용어 결합 순','semantic + term order'):tr('유사도 순','by similarity'))}${MODE==='us'?' · '+escapeHtml($('#ussel').textContent):(SCOPE[MODE].size?tr(' · 범위: ',' · scope: ')+[...SCOPE[MODE]].map(k=>escapeHtml(PACK_LABEL[k]||k)).join(', '):'')}</p>
  <div class="facets"><span class="lbl">${MODE==='patent'?'IPC로 정렬':tr('법률로 정렬','Sort by statute')}</span>${facets.map(([l,n])=>`<span class="f ${sortLaw===l?'on':''}" data-l="${escapeHtml(l)}">${escapeHtml(l)} ${n}</span>`).join('')}${sortLaw?'<span class="f" data-l="">✕ '+tr('해제','Clear')+'</span>':''}</div>`
  +ordered.map((x,i)=>x.종류==='특허공보'?`
  <div class="row"><div class="h">
    <div class="chips">${chip('특허','k')}${(x.결과||[]).map(t=>chip(t,/등록/.test(t)?'w':'')).join('')}${(x.적용법률||[]).slice(0,4).map(t=>chip(t,'r')).join('')}</div>
    <p class="title"><b>${escapeHtml(x.사건명)}</b> · ${escapeHtml(cleanCourt(x.법원))} · ${shortNo(x.사건번호)} · ${escapeHtml(fmtDate(x.선고일자))}</p>
    <p class="issue">${highlight(x.판시사항||x.매칭문단||'',terms)}</p>
    <p class="links">${x.키프리스?`<a href="${escapeHtml(x.키프리스)}" target="_blank" rel="noopener">KIPRIS 원문 ↗</a>`:''}<a href="${escapeHtml(x.원문||'')}" target="_blank" rel="noopener">Google Patents ↗</a></p>
  </div></div>`:`
  <div class="row"><div class="h">
    <div class="chips">${chip(x.종류||'',"k")}${(x.chain&&x.chain.length>1)?chip(x.chain.map(c=>(c.심급||'')+' '+(c.결과||[]).join('·')).join(' → '),'r'):(x.심급?chip(x.심급):'')}${(x.chain&&x.chain.length<=1)?(x.결과||[]).map(t=>chip(t,resultCls(t))).join(''):''}${(x.적용법률||[]).slice(0,4).map(t=>chip(t,'r')).join('')}</div>
    <p class="title"><b>${escapeHtml(x.사건명)}</b> · ${escapeHtml(cleanCourt(x.법원))} ${shortNo(x.사건번호)} · ${escapeHtml(fmtDate(x.선고일자))}</p>
    <p class="issue">${highlight(x.판시사항||x.매칭문단||'',terms)}</p>
    ${(x.매칭구분&&x.매칭구분!=='판시사항'&&x.매칭문단)?`<p class="hit"><span class="lbl">${tr('매칭','Match')} · ${escapeHtml(x.매칭구분)}</span>${highlight(snippet(x.매칭문단,terms),terms)}</p>`:(x.매칭구분?`<p class="hit"><span class="lbl">${tr('매칭','Match')} · ${escapeHtml(x.매칭구분)}</span></p>`:'')}
  </div>
  </div></div>`).join('');
  const ordIds=ordered.map(x=>x); document.querySelectorAll('.row .h').forEach((h,i)=>h.onclick=e=>{if(e.target.tagName==='A')return; if(ordIds[i].종류==='특허공보'){openExternal(ordIds[i].키프리스||ordIds[i].원문);return;} openReader(ordIds,i)});
  document.querySelectorAll('.facets .f').forEach(f=>f.onclick=()=>{window.__sortLaw=f.dataset.l||null;renderResults();window.scrollTo({top:res.offsetTop-10,behavior:'smooth'});});
}
// ── 원심 표기 → 정규화 링크 (법원 약칭 → 정식명, 사건번호) ──
