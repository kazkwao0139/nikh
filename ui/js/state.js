// 전역 상태·상수·입력창 바인딩
document.addEventListener('gesturestart',e=>e.preventDefault(),{passive:false});document.addEventListener('gesturechange',e=>e.preventDefault(),{passive:false});let __lt=0;document.addEventListener('touchend',e=>{const t=Date.now();if(t-__lt<300&&e.target&&!/INPUT|TEXTAREA|BUTTON|A/.test(e.target.tagName)){e.preventDefault();}__lt=t;},{passive:false});

const $=s=>document.querySelector(s); const q=$('#q'), home=$('#home'), res=$('#res');
const IS_TAURI=!!(window.__TAURI__&&window.__TAURI__.core); const IS_MOBILE=/iPad|iPhone|Android/.test(navigator.userAgent)||(navigator.maxTouchPoints>1&&/Mac/.test(navigator.userAgent)); const inv=(c,a)=>window.__TAURI__.core.invoke(c,a||{});
q.addEventListener('input',()=>{q.style.height='auto';q.style.height=Math.min(q.scrollHeight,200)+'px';});
q.addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();run();}});
// ── 모드: 판례(특허 제외) / 특허(특허 공보만). 같은 엔진, 팩만 다름 ──
const PACKS=[['civil','민사'],['criminal','형사'],['admin','행정'],['family','가사'],['tax','세법'],['constitution','헌법(헌재)'],['interpretation','법령해석례'],['appeal','행정심판·위원회'],['treaty','조약'],['patent','특허법원 판례']];
const PATS=[['pat_A','A 생활필수품'],['pat_B','B 처리·운수'],['pat_C','C 화학·야금'],['pat_D','D 섬유·지류'],['pat_E','E 건축·광업'],['pat_F','F 기계·조명·무기'],['pat_G','G 물리학'],['pat_H','H 전기'],['pat_X','X 분류없음']];
const CHIP={pat_A:'A 생활',pat_B:'B 운수',pat_C:'C 화학',pat_D:'D 섬유',pat_E:'E 건축',pat_F:'F 기계',pat_G:'G 물리',pat_H:'H 전기',pat_X:'X 미분류',patent:'특허법원',appeal:'행정심판',constitution:'헌재',interpretation:'해석례'};   // 칩용 짧은 이름(한 줄 유지), 전체 이름은 툴팁
const PLABEL=Object.fromEntries([...PACKS,...PATS]);
let LOADED=null;   // Tauri: 엔진에 실린 팩 키 목록 (null이면 웹 = 전부)
let SCOPE={case:new Set(),patent:new Set(),us:new Set()}; let US_STATE=null; let US_ERAS=new Set(); let US_SCOTUS=true; let US_ALLCIRC=false; let US_PATENT=false;   // All circuits: 13개 순회 항소법원 전부 / Patent courts: CAFC·CCPA + 특허 소송 집중 지구(E.D./W.D. Tex., D. Del., N.D. Cal.)
const PATENT_COURTS=['us_scotus','us_fed_cafc','us_fed_other','us_fedd_tx','us_fedd_de','us_fedd_ca'];
const CIRC={me:1,ma:1,nh:1,ri:1,ct:2,ny:2,vt:2,de:3,nj:3,pa:3,md:4,nc:4,sc:4,va:4,wv:4,la:5,ms:5,tx:5,ky:6,mi:6,oh:6,tn:6,il:7,in:7,wi:7,ar:8,ia:8,mn:8,mo:8,ne:8,nd:8,sd:8,ak:9,az:9,ca:9,hi:9,id:9,mt:9,nv:9,or:9,wa:9,co:10,ks:10,nm:10,ok:10,ut:10,wy:10,al:11,fl:11,ga:11,dc:'dc'};   // 연방 항소 순회구
const fedKeys=st=>{ const s=(st||'').replace(/^us_/,''); const c=CIRC[s]; return c?['us_fed_ca'+c,'us_fedd_'+s]:[]; };   // 그 주의 순회 항소법원 + 연방지방·파산법원 팩   // 연방대법원은 기본 포함(끌 수 있음)   // 검색 범위 칩 (비어 있으면 전체)
