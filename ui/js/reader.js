// 원문 리더
let R={list:[],i:0};
function openReader(list,i){R={list,i}; drawReader(); $('#reader').classList.add('on'); document.body.style.overflow='hidden';}
function closeReader(){$('#reader').classList.remove('on'); document.body.style.overflow='';}
function drawReader(){
  const x=R.list[R.i]; const chain=(x.chain&&x.chain.length)?x.chain:[{심급:x.심급,법원:x.법원,사건번호:x.사건번호,선고일자:x.선고일자,결과:x.결과,판시사항:x.판시사항,전문:'',원문:x.원문}];
  const terms=window.__terms||[];
  const isUS=String(x.종류||'').startsWith('US'); const isCourt=isUS||['민사','형사','일반행정','행정','가사','특허'].includes(x.종류);   // 해석례·행정심판·헌재·조약·조세심판·위원회·특허공보는 심급 개념 없음
  $('#lineage').style.display=isCourt?'':'none'; $('#lineage').innerHTML=isCourt?lineage(chain):'';
  $('#rtitle').innerHTML=`<b>${esc(x.사건명)}</b> · ${R.i+1}/${R.list.length}${isCourt?' · '+(chain.length>1?L(chain.length+'개 심급 나란히',chain.length+' instances side by side'):L('단일 심급','single instance')):' · '+esc(x.종류||'')}`;
  window.__afterCols=()=>{ const cols=$('#cols'); const n=cols.children.length; let d=$('#swdots'); if(!d){ d=document.createElement('div'); d.id='swdots'; d.className='swdots'; cols.parentElement.insertBefore(d, cols); }
    const draw=()=>{ const i=Math.round(cols.scrollLeft/Math.max(1,cols.clientWidth)); d.innerHTML=n>1?Array.from({length:n},(_,k)=>`<i class="${k===i?'on':''}"></i>`).join('')+`<span style="margin-left:8px">${esc((chain[i]&&chain[i].심급)||'')}${L(' · 좌우로 넘기기',' · swipe')}</span>`:''; };
    cols.onscroll=draw; if(window.innerWidth<=900&&n>1){ requestAnimationFrame(()=>{ cols.scrollLeft=cols.clientWidth*(n-1); draw(); }); } else draw(); };
  $('#cols').innerHTML=chain.map(c=>`<div class="col"><div class="ch"><div class="lv">${esc(isCourt?(c.심급||''):(x.종류||''))} ${(c.결과||[]).map(t=>chip(t,resultCls(t))).join('')}</div><div class="t">${esc(c.법원||'')} ${shortNo(c.사건번호||'')} · ${esc(fmtDate(c.선고일자||''))}</div>${c.원심미수록?`<div class="miss">원심 ${esc(c.원심표기||'')} — 미수록 · ${prevLinks(c.원심표기)}</div>`:''}</div>
    ${c.판시사항&&!(isUS&&(c.전문||'').replace(/【[^】]*】\n?/,'').slice(0,200).replace(/\s+/g,' ').startsWith((c.판시사항||'').slice(0,120).replace(/\s+/g,' ')))?`<h4>${L('판시사항','Headnote / Syllabus')}</h4><pre>${isUS?esc(usPretty(c.판시사항)):hl(c.판시사항,terms)}</pre>`:''}
    ${c.판결요지?`<h4>${L('판결요지','Summary')}</h4><pre>${hl(c.판결요지,terms)}</pre>`:''}
    ${c.참조조문?`<h4>${L('참조조문','Statutes cited')}</h4><pre>${esc(c.참조조문)}</pre>`:''}
    ${c.참조판례?`<h4>${L('참조판례','Cases cited')}</h4><pre>${esc(c.참조판례)}</pre>`:''}
    <h4>${L('전문','Full opinion')}</h4><pre>${sections(c.전문||'',isUS)}</pre>
    <p><a href="${c.원문}" target="_blank" rel="noopener">${isUS?'CourtListener ↗':'국가법령정보센터 ↗'}</a>${isCourt&&!isUS&&c.법원&&c.사건번호?` · <a href="https://casenote.kr/${encodeURIComponent(normCourt(c.법원))}/${encodeURIComponent(String(c.사건번호).split(',')[0].trim())}" target="_blank" rel="noopener">케이스노트 ↗</a>`:''}</p></div>`).join(''); window.__afterCols();
}
// Bluebook T1/T7/T10 court abbreviation (rule-based) — 팩의 미국 법원명 → 인용 괄호용 약어
