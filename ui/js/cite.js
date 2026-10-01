// 인용 복사(한국 판결·결정 / 미국 Bluebook)·날짜
const STATE_ABBR={'Alabama':'Ala.','Alaska':'Alaska','Arizona':'Ariz.','Arkansas':'Ark.','California':'Cal.','Colorado':'Colo.','Connecticut':'Conn.','Delaware':'Del.','Florida':'Fla.','Georgia':'Ga.','Hawaii':'Haw.','Idaho':'Idaho','Illinois':'Ill.','Indiana':'Ind.','Iowa':'Iowa','Kansas':'Kan.','Kentucky':'Ky.','Louisiana':'La.','Maine':'Me.','Maryland':'Md.','Massachusetts':'Mass.','Michigan':'Mich.','Minnesota':'Minn.','Mississippi':'Miss.','Missouri':'Mo.','Montana':'Mont.','Nebraska':'Neb.','Nevada':'Nev.','New Hampshire':'N.H.','New Jersey':'N.J.','New Mexico':'N.M.','New York':'N.Y.','North Carolina':'N.C.','North Dakota':'N.D.','Ohio':'Ohio','Oklahoma':'Okla.','Oregon':'Or.','Pennsylvania':'Pa.','Rhode Island':'R.I.','South Carolina':'S.C.','South Dakota':'S.D.','Tennessee':'Tenn.','Texas':'Tex.','Utah':'Utah','Vermont':'Vt.','Virginia':'Va.','Washington':'Wash.','West Virginia':'W. Va.','Wisconsin':'Wis.','Wyoming':'Wyo.','District of Columbia':'D.C.','Puerto Rico':'P.R.','Guam':'Guam','Virgin Islands':'V.I.'};
const CIRCUIT_ORD={First:'1st',Second:'2d',Third:'3d',Fourth:'4th',Fifth:'5th',Sixth:'6th',Seventh:'7th',Eighth:'8th',Ninth:'9th',Tenth:'10th',Eleventh:'11th',Federal:'Fed.','District of Columbia':'D.C.','D.C.':'D.C.'};
const DISTRICT_DIR={Southern:'S.D.',Northern:'N.D.',Eastern:'E.D.',Western:'W.D.',Middle:'M.D.',Central:'C.D.'};
const stateRe=Object.keys(STATE_ABBR).sort((a,b)=>b.length-a.length).join('|');
function stateAbbr(s){ return STATE_ABBR[s]||s; }
function districtAbbr(dir,state){ const a=stateAbbr(state); const d=DISTRICT_DIR[dir]; if(!d) return 'D. '+a; return (a.split(' ').length===1&&/^[A-Z]\.[A-Z]?\.?$/.test(a))? d+a.replace(/\s/g,'') : d+' '+a; }   // 두 글자 이니셜 주(N.Y., N.J., N.C.)는 붙여쓰기, 나머지는 띄움
function courtAbbr(name){ let s=String(name||'').trim().replace(/\s+/g,' ');
  let m;
  if(/^Supreme Court of the United States$/i.test(s)) return 'U.S.';
  if((m=s.match(/^(?:United States )?Court of Appeals(?:,| for the)? (\w+|D\.C\.) Circuit$/i))) return (CIRCUIT_ORD[m[1]]||m[1])+' Cir.';
  if((m=s.match(/^(?:United States )?Court of Appeals for the (\w+)$/))) return (CIRCUIT_ORD[m[1]]||m[1])+' Cir.';   // 잘린 이름("Ninth")
  if((m=s.match(/^(?:United States )?(Bankruptcy Court|District Court|Circuit Court) for the (\w+) District of (.+)$/i))||(m=s.match(/^(?:United States )?(Bankruptcy Court|District Court|Circuit Court),? ([NSEWMC])\.D\. (.+)$/i))){ const kind=m[1].toLowerCase(); let dir=m[2]; if(dir.length===1) dir={S:'Southern',N:'Northern',E:'Eastern',W:'Western',M:'Middle',C:'Central'}[dir.toUpperCase()]; const st=m[3].replace(/,.*$/,''); const d=districtAbbr(dir,st); return kind.startsWith('bankruptcy')?'Bankr. '+d:(kind.startsWith('circuit')?'C.C. '+d:d); }
  if((m=s.match(/^(?:United States )?(Bankruptcy Court|District Court|Circuit Court) for the District of (.+)$/i))||(m=s.match(/^(?:United States )?(Bankruptcy Court|District Court|Circuit Court),? D\. (.+)$/i))){ const kind=m[1].toLowerCase(); const d='D. '+stateAbbr(m[2].replace(/,.*$/,'')); return kind.startsWith('bankruptcy')?'Bankr. '+d:(kind.startsWith('circuit')?'C.C. '+d:d); }
  const F={'United States Court of Claims':'Ct. Cl.','United States Claims Court':'Cl. Ct.','United States Court of Federal Claims':'Fed. Cl.','United States Customs Court':'Cust. Ct.','United States Court of Customs and Patent Appeals':'C.C.P.A.','United States Court of International Trade':"Ct. Int'l Trade",'United States Tax Court':'T.C.','United States Board of Tax Appeals':'B.T.A.','United States Court of Military Appeals':'C.M.A.','United States Court of Appeals for the Armed Forces':'C.A.A.F.','United States Court of Appeals for Veterans Claims':'Vet. App.','United States Army Court of Military Review':'A.C.M.R.','United States Air Force Court of Military Review':'A.F.C.M.R.','United States Navy-Marine Corps Court of Military Review':'N.M.C.M.R.','United States Army Court of Criminal Appeals':'A. Ct. Crim. App.','United States Navy-Marine Corps Court of Criminal Appeals':'N-M. Ct. Crim. App.','United States Air Force Court of Criminal Appeals':'A.F. Ct. Crim. App.','United States Coast Guard Court of Criminal Appeals':'C.G. Ct. Crim. App.','Temporary Emergency Court of Appeals':'Temp. Emer. Ct. App.','United States Emergency Court of Appeals':'Emer. Ct. App.','Judicial Panel on Multidistrict Litigation':'J.P.M.L.'};
  if(F[s]) return F[s];
  if(/^Court of Customs and Patent Appeals$/i.test(s)) return 'C.C.P.A.';
  if(/Temporary Emergency Court of Appeals/i.test(s)) return 'Temp. Emer. Ct. App.';
  if(/^United States Court of Veterans Appeals$/i.test(s)) return 'Vet. App.';
  if(/^United States Courts? of Military Review$/i.test(s)) return 'C.M.R.';
  if(/Coast Guard Court of Military Review/i.test(s)) return 'C.G.C.M.R.';
  if(/^County Court of New York/i.test(s)||/^New York County Court/i.test(s)) return 'N.Y. Cnty. Ct.';
  if(/^New York Family Court$/i.test(s)) return 'N.Y. Fam. Ct.';
  if(/^New York Court of General Sessions$/i.test(s)) return 'N.Y. Gen. Sess.';
  if(/^New York Court of Special Sessions$/i.test(s)) return 'N.Y. Spec. Sess.';
  if(/^New York Domestic Relations Court$/i.test(s)) return 'N.Y. Dom. Rel. Ct.';
  if(/Criminal Court of the City of New York/i.test(s)) return 'N.Y. Crim. Ct.';
  if(/^New York Supreme Court \(other\)$/i.test(s)) return 'N.Y. Sup. Ct.';
  if(/^Illinois Circuit Court/i.test(s)) return 'Ill. Cir. Ct.';
  if(/^[A-Z][a-z]+ City Court$/.test(s)) return 'N.Y. City Ct.';
  // New York (T1: Court of Appeals = highest)
  if(/^New York Court of Appeals$/i.test(s)) return 'N.Y.';
  if(/^New York Commission of Appeals$/i.test(s)) return "N.Y. Comm'n App.";
  if(/Appellate Division/i.test(s)&&/New York/i.test(s)) return 'N.Y. App. Div.';
  if(/Appellate Term/i.test(s)&&/New York/i.test(s)) return 'N.Y. App. Term';
  if(/^New York Supreme Court(,.*)?$/i.test(s)||/^Supreme Court of (the State of )?New York/i.test(s)) return 'N.Y. Sup. Ct.';
  if(/Surrogate/i.test(s)&&/New York/i.test(s)) return 'N.Y. Sur. Ct.';
  if(/^(New York City |Civil Court of the City of New York)/i.test(s)){ if(/Civil/i.test(s)) return 'N.Y. Civ. Ct.'; if(/Criminal/i.test(s)) return 'N.Y. Crim. Ct.'; if(/Family/i.test(s)) return 'N.Y. Fam. Ct.'; if(/Municipal/i.test(s)) return 'N.Y. Mun. Ct.'; return 'N.Y. City Ct.'; }
  if(/^New York (State )?Court of Claims$/i.test(s)) return 'N.Y. Ct. Cl.';
  if(/^New York Court of Common Pleas$/i.test(s)) return 'N.Y. C.P.';
  if(/^New York County Court$/i.test(s)) return 'N.Y. Cnty. Ct.';
  if((m=s.match(/^(\w+) County (District|Family|Surrogate's|Supreme) Court$/))) return `N.Y. ${ {District:'Dist. Ct.',Family:'Fam. Ct.',"Surrogate's":'Sur. Ct.',Supreme:'Sup. Ct.'}[m[2]] }`;
  // Illinois
  if(/^(Illinois Supreme Court|Supreme Court of Illinois)$/i.test(s)) return 'Ill.';
  if(/^(Illinois Appellate Court|Appellate Court of Illinois)/i.test(s)) return 'Ill. App. Ct.';
  if(/^Illinois Court of Claims$/i.test(s)) return 'Ill. Ct. Cl.';
  // generic states
  if((m=s.match(new RegExp(`^(?:Supreme Court of (?:the State of )?(${stateRe})|(${stateRe}) Supreme Court)$`)))) return stateAbbr(m[1]||m[2]);
  if((m=s.match(new RegExp(`^(${stateRe}) Court of Chancery$|^Court of Chancery of (${stateRe})$`)))) return stateAbbr(m[1]||m[2])+' Ch.';
  if((m=s.match(new RegExp(`^(${stateRe}) Court of Appeals$|^Court of Appeals of (${stateRe})$`)))) return stateAbbr(m[1]||m[2])+' Ct. App.';
  if((m=s.match(new RegExp(`^(${stateRe}) Superior Court$|^Superior Court of (${stateRe})$`)))) return stateAbbr(m[1]||m[2])+' Super. Ct.';
  if((m=s.match(new RegExp(`^(${stateRe}) (?:Court of )?Appellate Court$|^Appellate Court of (${stateRe})$`)))) return stateAbbr(m[1]||m[2])+' App. Ct.';
  if((m=s.match(new RegExp(`^(${stateRe}) Court of (Criminal|Civil) Appeals$`)))) return stateAbbr(m[1])+' Ct. '+m[2].slice(0,4)+'. App.';
  if((m=s.match(new RegExp(`^(${stateRe}) District Court$`)))) return stateAbbr(m[1])+' Dist. Ct.';
  if((m=s.match(new RegExp(`^(${stateRe}) Court of Claims$`)))) return stateAbbr(m[1])+' Ct. Cl.';
  return null; }
// reporter → court implied (Bluebook: 리포터가 법원을 특정하면 괄호에 법원 생략)
function reporterImpliesCourt(rep){ return /^(U\.S\.|S\. ?Ct\.|L\. ?Ed\.|N\.Y\.(2d|3d)?|A\.D\.(2d|3d)?|Ill\.( 2d)?|Ill\. App\.( 2d| 3d)?|Cust\. Ct\.|Ct\. Cl\.|T\.C\.|B\.T\.A\.|C\.C\.P\.A\.|M\.J\.|C\.M\.A\.|Fed\. Cl\.|Cl\. Ct\.|Vet\. App\.|Ct\. Int'l Trade|Cal\.( 2d| 3d| 4th| 5th)?|Mass\.|Pa\.|N\.J\.|Del\.( Ch\.)?|Tex\.|Ohio St\.( 2d| 3d)?|Wash\.( 2d)?|Colo\.|Conn\.|Md\.|Mich\.|Minn\.|Mo\.|Wis\.( 2d)?|Or\.|Ariz\.|Ga\.|Fla\.|Va\.|N\.C\.|S\.C\.|Ky\.|Tenn\.|La\.|Ala\.|Okla\.|Kan\.|Iowa|Neb\.|Nev\.|Utah( 2d)?|Idaho|Mont\.|Wyo\.|N\.M\.|N\.D\.|S\.D\.|Me\.|N\.H\.|Vt\.|R\.I\.|W\. Va\.|Haw\.|Alaska|Ind\.|Ark\.|Miss\.|Ohio)$/.test(rep); }

// Bluebook PARTY_ABBR — 사건명 단어 약어(미국 인용 복사용). 단어 경계·대소문자 유지, 'v.'·'In re'·'ex rel.' 은 그대로.
const PARTY_ABBR={Administration:"Admin.",Administrator:"Adm'r",America:"Am.",American:"Am.",Associate:"Assoc.",Association:"Ass'n",Atlantic:"Atl.",Authority:"Auth.",Automobile:"Auto.",Board:"Bd.",Broadcasting:"Broad.",Brothers:"Bros.",Building:"Bldg.",Business:"Bus.",Casualty:"Cas.",Central:"Cent.",Chemical:"Chem.",Commission:"Comm'n",Commissioner:"Comm'r",Committee:"Comm.",Communications:"Commc'ns",Company:"Co.",Consolidated:"Consol.",Construction:"Constr.",Continental:"Cont'l",Cooperative:"Coop.",Corporation:"Corp.",County:"Cnty.",Department:"Dep't",Development:"Dev.",Director:"Dir.",Distributing:"Distrib.",Distribution:"Distrib.",District:"Dist.",Division:"Div.",Eastern:"E.",Education:"Educ.",Electric:"Elec.",Electrical:"Elec.",Electronic:"Elec.",Engineering:"Eng'g",Enterprise:"Enter.",Enterprises:"Enters.",Entertainment:"Ent.",Environment:"Env't",Environmental:"Env't",Equipment:"Equip.",Federal:"Fed.",Federation:"Fed'n",Financial:"Fin.",Foundation:"Found.",General:"Gen.",Government:"Gov't",Hospital:"Hosp.",Housing:"Hous.",Incorporated:"Inc.",Industrial:"Indus.",Industries:"Indus.",Industry:"Indus.",Information:"Info.",Institute:"Inst.",Institution:"Inst.",Insurance:"Ins.",International:"Int'l",Investment:"Inv.",Laboratory:"Lab.",Laboratories:"Labs.",Liability:"Liab.",Limited:"Ltd.",Litigation:"Litig.",Machine:"Mach.",Machines:"Machs.",Machinery:"Mach.",Maintenance:"Maint.",Management:"Mgmt.",Manufacturer:"Mfr.",Manufacturing:"Mfg.",Maritime:"Mar.",Market:"Mkt.",Marketing:"Mktg.",Mechanical:"Mech.",Medical:"Med.",Medicine:"Med.",Memorial:"Mem'l",Merchant:"Merch.",Metropolitan:"Metro.",Municipal:"Mun.",Mutual:"Mut.",National:"Nat'l",Northern:"N.",Northeast:"Ne.",Northwest:"Nw.",Organization:"Org.",Pacific:"Pac.",Partnership:"P'ship",Pharmaceutical:"Pharm.",Pharmaceuticals:"Pharms.",Preserve:"Pres.",Products:"Prods.",Production:"Prod.",Professional:"Pro.",Property:"Prop.",Protection:"Prot.",Public:"Pub.",Publishing:"Publ'g",Railroad:"R.R.",Railway:"Ry.",Refining:"Ref.",Resource:"Res.",Resources:"Res.",Restaurant:"Rest.",Retirement:"Ret.",Savings:"Sav.",Science:"Sci.",Securities:"Sec.",Security:"Sec.",Service:"Serv.",Services:"Servs.",Shareholder:"S'holder",Shareholders:"S'holders",Society:"Soc'y",Southern:"S.",Southeast:"Se.",Southwest:"Sw.",Steamship:"S.S.",Street:"St.",Surety:"Sur.",System:"Sys.",Systems:"Sys.",Technology:"Tech.",Technologies:"Techs.",Telecommunications:"Telecomm.",Telephone:"Tel.",Television:"TV",Transportation:"Transp.",Trust:"Tr.",Trustee:"Tr.",University:"Univ.",Utility:"Util.",Utilities:"Utils.",Village:"Vill.",Western:"W."};
function partyAbbr(name){ return String(name||'').replace(/[A-Za-z][A-Za-z']*/g, w => { const ab=PARTY_ABBR[w]; if(ab) return ab; const cap=w.charAt(0).toUpperCase()+w.slice(1).toLowerCase(); const ab2=PARTY_ABBR[cap]; return ab2 ? (w===w.toUpperCase() ? ab2.toUpperCase() : ab2) : w; }); }
function fmtDate(d){ let s=String(d||'').trim(); let y,mo,da; if(/^\d{4,8}$/.test(s)){ y=s.slice(0,4); mo=s.length>=6?s.slice(4,6):''; da=s.length>=8?s.slice(6,8):''; } else { const m=s.match(/(\d{4})\D*(\d{1,2})\D*(\d{1,2})/); if(!m) return s; [y,mo,da]=[m[1],m[2],m[3]]; } if(+y>=4000) y=String(+y-2333);   /* 단기(檀紀) → 서기 */ if(!mo) return y; const BB=['Jan.','Feb.','Mar.','Apr.','May','June','July','Aug.','Sept.','Oct.','Nov.','Dec.'][+mo-1]||mo;   /* Bluebook 월 약어 */ if(!da) return LANG==='en'?`${BB} ${y}`:`${y}. ${+mo}.`; return LANG==='en'?`${BB} ${+da}, ${y}`:`${y}. ${+mo}. ${+da}.`; }
function cite(c,x){ // 판례: 대법원 2023. 5. 18. 선고 2022다12345 판결 / 그 외: 기관 사건번호 (날짜)
  const ct=cleanCourt(c.법원||x.법원||''); const no=c.사건번호||x.사건번호||''; const d=fmtDate(c.선고일자||x.선고일자);
  { const code=(String(no).match(/^\d{2,4}([가-힣]+)\d/)||[])[1]||'';   /* 사건부호 → 재판 종류 */
    if(/^헌법재판소/.test(ct)||/^헌/.test(code)) return `${ct} ${d} 선고 ${no} 결정`;
    if(/^(느|드|스|브|즈)/.test(code)&&/가정법원|가법|대법원/.test(ct)&&/^(느|스|브|즈)/.test(code)) return `${ct} ${d}자 ${no} ${/^느/.test(code)?'심판':'결정'}`;
    if(/^(마|모|라|카|초|정|그|으)/.test(code)) return `${ct} ${d}자 ${no} 결정`;   /* 항고·재항고·신청·보전·집행 = 결정 */
    if(/^조세심판원/.test(ct)) return `${ct} ${d}자 ${no} 결정`;
    if(/노동위원회$/.test(ct)) return `${ct} ${d}자 ${no} 판정`;
    if(/공정거래위원회$/.test(ct)) return `${ct} ${d}자 ${no} 의결`;
    if(/(행정심판위원회|국민권익위원회)$/.test(ct)) return `${ct} ${d}자 ${no} 재결`;
    if(/^법제처$/.test(ct)) return `${ct} ${d} 회신 ${no} 법령해석례`;
    if(/법원$|고법$|지법$|가법$/.test(ct)||/^\d{2,4}[가-힣]{1,2}\d+/.test(no)) return `${ct} ${d} 선고 ${no} 판결`; }
  if(String(x.종류||'').startsWith('US')){ const y=String(c.선고일자||x.선고일자||'').slice(0,4); const rep=(String(no).match(/^\d+ (.+?) \d+/)||[])[1]||''; const ab=courtAbbr(ct); const cc=(ab&&reporterImpliesCourt(rep))?'':(ab||ct); return `${partyAbbr(x.사건명)}, ${no} (${[cc,y].filter(Boolean).join(' ')})`; }   /* Bluebook: T7 법원 약어, 리포터가 법원을 특정하면 생략 */
  return `${ct} ${no} (${d})`; }
async function copyText(t){ try{ await navigator.clipboard.writeText(t); }catch(e){ const ta=document.createElement('textarea'); ta.value=t; ta.style.position='fixed'; ta.style.opacity='0'; document.body.appendChild(ta); ta.select(); document.execCommand('copy'); ta.remove(); } }
