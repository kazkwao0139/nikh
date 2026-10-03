// 초기 렌더·Tauri 상태 폴링·키보드
// ── 버튼·입력 바인딩 (runSearch 등 뒤 파일의 함수 참조 → 정의가 모두 끝난 뒤 실행) ──
$('#rcopy').onclick=async()=>{ const x=READER.list[READER.i]; const chain=(x.chain&&x.chain.length)?x.chain:[x]; const sel=window.getSelection(); const st=sel&&sel.toString().trim();
  let idx=0; if(st&&sel.anchorNode){ const col=(sel.anchorNode.nodeType===1?sel.anchorNode:sel.anchorNode.parentElement).closest('.col'); if(col) idx=[...document.querySelectorAll('#cols .col')].indexOf(col); }
  const c=chain[Math.max(idx,0)]||x; const src=cite(c,x); const t=st?`"${st}" (${src})`:src; await copyText(t); $('#rcopied').textContent=tr('복사됨: ','Copied: ')+t.slice(0,60)+(t.length>60?'…':''); setTimeout(()=>$('#rcopied').textContent='',4000); };
$('#rclose').onclick=closeReader; $('#rprev').onclick=()=>{if(READER.i>0){READER.i--;drawReader();}}; $('#rnext').onclick=()=>{if(READER.i<READER.list.length-1){READER.i++;drawReader();}};
document.querySelectorAll('#lang .l').forEach(b=>b.onclick=()=>{ setLang(b.dataset.l); if(queryInput.value.trim()) runSearch(); });   // 한 경로(setLang)
document.querySelectorAll('#modes .m').forEach(m=>m.onclick=()=>{MODE=m.dataset.m; if(IS_TAURI) invoke('prewarm',{mode:MODE}).catch(()=>{});document.querySelectorAll('#modes .m').forEach(x=>x.classList.toggle('on',x===m));queryInput.placeholder=PLACEHOLDER[MODE];drawScope();syncTips();if(queryInput.value.trim())runSearch();else queryInput.focus();});
const openExternal=u=>{ if(!u) return; if(IS_TAURI) invoke('open_url',{url:u}).catch(e=>alert(String(e))); else window.open(u,'_blank','noopener'); };
document.addEventListener('click',e=>{ const a=e.target.closest&&e.target.closest('a[href^="http"]'); if(a&&IS_TAURI){ e.preventDefault(); openExternal(a.href); } });
$('#plus').onclick=()=>$('#pdfin').click();
$('#pdfin').onchange=async e=>{ const f=e.target.files[0]; if(!f) return; e.target.value=''; const note=$('#pdfnote'); note.hidden=false; note.textContent=tr('PDF에서 텍스트를 읽는 중…','Reading text from the PDF…');
  try{ if(!IS_TAURI) throw new Error(tr('PDF 읽기는 앱에서만 됩니다','PDF import works only in the app')); const buf=new Uint8Array(await f.arrayBuffer()); if(buf.length>40e6) throw new Error(tr('40MB 이하 PDF만','PDFs up to 40MB only'));
    const r=await window.__TAURI__.core.invoke('pdf_facts',buf); queryInput.value=r.text; queryInput.dispatchEvent(new Event('input')); note.innerHTML=tr(`<b>${escapeHtml(f.name)}</b>에서 <b>${escapeHtml(r.label)}</b> ${r.text.length.toLocaleString()}자를 옮겼습니다 · 필요하면 다듬은 뒤 검색 · 파일은 저장되지 않습니다`,`Moved <b>${escapeHtml(r.label)}</b> (${r.text.length.toLocaleString()} chars) from <b>${escapeHtml(f.name)}</b> · edit if needed, then search · the file is not stored`); runSearch(); }
  catch(err){ note.textContent=tr('PDF 처리 실패: ','PDF failed: ')+String(err).replace(/^Error:\s*/,''); } };
$('#go').onclick=runSearch; $('#logo').onclick=()=>{queryInput.value='';queryInput.style.height='auto';res.innerHTML='';$('#pdfnote').hidden=true;home.classList.remove('up');closeReader();window.scrollTo({top:0});queryInput.focus();}; document.querySelectorAll('.ex span').forEach(s=>s.onclick=()=>{queryInput.value=s.textContent;queryInput.dispatchEvent(new Event('input'));runSearch();});
if(!IS_TAURI) fetch('/api/loaded').then(r=>r.json()).then(j=>{ if(j.loaded&&j.loaded.length){ LOADED=j.loaded; drawScope(); } }).catch(()=>{});   // 웹 모드도 실린 팩만 활성
drawScope();   // escapeHtml 등 정의 뒤에 초기 렌더
applyLang();   // 한국어 기본 문구(유머 포함)도 초기 1회 적용 (앞에서 부르면 TDZ ReferenceError로 스크립트 전체가 죽음)
// ── 데이터 팩 (Tauri) ──
if(IS_TAURI){$('#pkopen').style.display='inline-block';$('#pkopen').onclick=openPacks;
  const LBL={admin:'행정',appeal:'행정심판·위원회',civil:'민사',constitution:'헌법',criminal:'형사',family:'가사',interpretation:'해석례',patent:'특허법원',tax:'세법',treaty:'조약'};
  const gauge=(f)=>{ $('#gauge').style.display='inline-block'; $('#gaugeFill').style.width=Math.round(f*100)+'%'; };
  (async function poll(){ window.__poll=poll; const pk=$('#packs').classList.contains('on'); try{ const s=await invoke('stats');
      if(s.status==='ready'){ if(!window.__ready){ setTimeout(()=>checkUpdates(false),1500); } window.__ready=true; if(JSON.stringify(LOADED)!==JSON.stringify(s.loaded||[])){ LOADED=s.loaded||[]; drawScope(); } window.__S=s; const base=readyText(s);
        if(s.stage==='warm'&&s.total>0&&s.done<s.total){ $('#ready').textContent=base+tr(' · 디스크 캐시 ',' · disk cache ')+Math.round(100*s.done/s.total)+'%'; gauge(s.done/s.total); setTimeout(poll,800); return; }
        $('#ready').textContent=base; $('#gauge').style.display='none'; if(window.__reloading){ window.__reloading=false; $('#pkstatus').textContent=base; $('#pkgauge').style.display='none'; resetApplyButton(); if(window.__firstApply){ window.__firstApply=false; $('#packs').classList.remove('on'); $('#pkintro').hidden=true; queryInput.focus(); } else if(pk) openPacks(); } return; }
      if(String(s.status||'').startsWith('error')){ $('#gauge').style.display='none';
        if(/데이터 팩이 없/.test(s.status)){ window.__nopacks=true; if(LOADED===null||LOADED.length){ LOADED=[]; drawScope(); } $('#ready').textContent=tr('데이터 팩이 아직 없습니다 · 아래에서 필요한 것만 받으세요','No data packs yet · download only what you need below'); if(!window.__introShown && $('#country').hidden){ window.__introShown=true; $('#pkintro').hidden=false; openPacks(); } setTimeout(poll,3000); return; }
        if(/팩을 싣다가 종료/.test(s.status)){ window.__nopacks=true; if(LOADED===null||LOADED.length){ LOADED=[]; drawScope(); } $('#ready').textContent=tr('지난 실행이 팩을 싣다가 종료되어 모든 팩을 내렸습니다 · 필요한 팩만 다시 골라 주세요','The app closed while loading packs last time, so all packs were unloaded · re-select only what you need'); return; }   // 크래시 페일세이프(문구 임시)
        $('#ready').textContent=tr('준비 실패: ','Failed to load: ')+s.status.slice(7); if(window.__reloading){ window.__reloading=false; resetApplyButton(); $('#pkstatus').textContent=$('#ready').textContent; } return; }
      const lab=s.label?(PACK_LABEL[s.label]||LBL[s.label]||(s.label.startsWith('pat_')?'특허 '+s.label.slice(4):s.label)):''; $('#ready').textContent=tr('데이터 팩 여는 중 ','Opening data packs ')+(s.total?`${s.done+1}/${s.total}`:'')+(lab?' · '+lab:''); if(pk){ $('#pkstatus').textContent=$('#ready').textContent; if(s.total){ $('#pkgauge').style.display=''; $('#pkgaugeFill').style.width=Math.round(100*s.done/s.total)+'%'; } } if(s.total) gauge(s.done/s.total);
    }catch(e){} setTimeout(poll,500); })();}
document.addEventListener('keydown',e=>{if(!$('#reader').classList.contains('on'))return; if(e.key==='Escape')closeReader(); if(e.key==='ArrowLeft')$('#rprev').click(); if(e.key==='ArrowRight')$('#rnext').click();});
