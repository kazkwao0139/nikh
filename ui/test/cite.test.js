// 인용 복사·날짜 표시 회귀 테스트: node --test ui/test  (의존성 없음)
// js/cite.js 를 전역 스코프에 그대로 올리고(LANG·court 는 다른 파일 전역이라 여기서 대신 정의) 결과 문자열을 고정값과 비교
const test = require('node:test'); const assert = require('node:assert/strict'); const fs = require('fs'); const path = require('path'); const vm = require('vm');
const src = fs.readFileSync(path.join(__dirname, '..', 'js', 'cite.js'), 'utf8');
function load(lang) { const ctx = { LANG: lang, cleanCourt: s => s || '', console }; vm.createContext(ctx); vm.runInContext(src, ctx); return ctx; }

test('fmtDate: 8·6·4자리, 단기(檀紀)→서기, Bluebook 영어', () => {
  const en = load('en'), ko = load('ko');
  assert.equal(en.fmtDate('20021122'), 'Nov. 22, 2002'); assert.equal(en.fmtDate('187409'), 'Sept. 1874'); assert.equal(en.fmtDate('1883'), '1883');
  assert.equal(en.fmtDate('42841231'), 'Dec. 31, 1951');   // 4284 − 2333
  assert.equal(ko.fmtDate('42841231'), '1951. 12. 31.'); assert.equal(ko.fmtDate('20241105'), '2024. 11. 5.'); assert.equal(ko.fmtDate('187409'), '1874. 9.');
  assert.equal(en.fmtDate('2008-09-02'), 'Sept. 2, 2008'); assert.equal(en.fmtDate(''), '');
});

test('cite(한국): 사건부호별 판결·결정·심판·재결·판정·의결·해석례', () => {
  const c = load('ko'); const k = (ct, no, d) => c.cite({}, { 법원: ct, 사건번호: no, 선고일자: d, 종류: '민사' });
  assert.equal(k('대법원', '2002다12345', '20021122'), '대법원 2002. 11. 22. 선고 2002다12345 판결');
  assert.equal(k('대법원', '84마81', '19841113'), '대법원 1984. 11. 13.자 84마81 결정');
  assert.equal(k('서울고법', '4281행1', '42841231'), '서울고법 1951. 12. 31. 선고 4281행1 판결');
  assert.equal(k('헌법재판소', '2004헌마554', '20041021'), '헌법재판소 2004. 10. 21. 선고 2004헌마554 결정');
  assert.equal(k('서울가정법원', '2009느단123', '20100101'), '서울가정법원 2010. 1. 1.자 2009느단123 심판');
  assert.equal(k('대법원', '2010스1', '20100301'), '대법원 2010. 3. 1.자 2010스1 결정');
  assert.equal(k('서울고등법원', '2019라1', '20200101'), '서울고등법원 2020. 1. 1.자 2019라1 결정');
  assert.equal(k('조세심판원', '조심2019서1234', '20200101'), '조세심판원 2020. 1. 1.자 조심2019서1234 결정');
  assert.equal(k('노동위원회', '2019부해123', '20190501'), '노동위원회 2019. 5. 1.자 2019부해123 판정');
  assert.equal(k('공정거래위원회', '2019공정1', '20190501'), '공정거래위원회 2019. 5. 1.자 2019공정1 의결');
  assert.equal(k('국민권익위원회', '2019-12345', '20190501'), '국민권익위원회 2019. 5. 1.자 2019-12345 재결');
  assert.equal(k('법제처', '26-0668', '20260101'), '법제처 2026. 1. 1. 회신 26-0668 법령해석례');
  assert.equal(k('특허법원', '2019허1234', '20200101'), '특허법원 2020. 1. 1. 선고 2019허1234 판결');
  assert.equal(k('서울행정법원', '2019구합1', '20200101'), '서울행정법원 2020. 1. 1. 선고 2019구합1 판결');
});

test('cite(미국): Bluebook — 법원 약어, 리포터가 법원을 특정하면 생략', () => {
  const c = load('en'); const u = (name, no, ct, d) => c.cite({}, { 사건명: name, 사건번호: no, 법원: ct, 선고일자: d, 종류: 'US Civil' });
  assert.equal(u('Evans v. Brown', '109 U.S. 180', 'Supreme Court of the United States', '1883'), 'Evans v. Brown, 109 U.S. 180 (1883)');
  assert.equal(u('In re NCS Healthcare, Inc., Shareholders Litigation', '825 A.2d 240', 'Delaware Court of Chancery', '20021122'), 'In re NCS Healthcare, Inc., S\'holders Litig., 825 A.2d 240 (Del. Ch. 2002)');
  assert.equal(u('Smith v. Jones', '611 F.2d 15', 'United States Court of Appeals for the Second Circuit', '19791101'), 'Smith v. Jones, 611 F.2d 15 (2d Cir. 1979)');
  assert.equal(u('A v. B', '41 F. Supp. 100', 'District Court, S.D. New York', '1941'), 'A v. B, 41 F. Supp. 100 (S.D.N.Y. 1941)');
  assert.equal(u('A v. B', '12 A.D.2d 300', 'New York Supreme Court, Appellate Division', '1960'), 'A v. B, 12 A.D.2d 300 (1960)');
  assert.equal(u('A v. B', '55 Misc. 2d 10', 'New York Supreme Court', '1967'), 'A v. B, 55 Misc. 2d 10 (N.Y. Sup. Ct. 1967)');
  assert.equal(u('A v. B', '2015 NY Slip Op 01234', 'New York Supreme Court, Appellate Division', '20150201'), 'A v. B, 2015 NY Slip Op 01234 (N.Y. App. Div. 2015)');
});

test('courtAbbr(): 법원명 약어 표본 + 팩 법원명 718종 커버리지(건수 가중) ≥ 99.5%', () => {
  const c = load('en');
  for (const [n, want] of [['United States District Court for the District of Delaware', 'D. Del.'], ['United States Bankruptcy Court for the Eastern District of New York', 'Bankr. E.D.N.Y.'], ['Illinois Appellate Court', 'Ill. App. Ct.'], ['United States Court of Appeals for the Federal Circuit', 'Fed. Cir.'], ['New York Court of Appeals', 'N.Y.'], ['United States Court of Appeals for the Ninth', '9th Cir.'], ['County Court of New York, Nassau County', 'N.Y. Cnty. Ct.']]) assert.equal(c.courtAbbr(n), want, n);
  const rows = fs.readFileSync(path.join(__dirname, 'fixtures', 'us_courts.tsv'), 'utf8').trim().split('\n').map(l => { const [cnt, ...name] = l.split('\t'); return [+cnt, name.join('\t')]; });
  let tot = 0, ok = 0; for (const [cnt, name] of rows) { tot += cnt; if (c.courtAbbr(name)) ok += cnt; }
  assert.ok(ok / tot >= 0.995, `coverage ${(100 * ok / tot).toFixed(2)}%`);
});

test('partyAbbr(): 사건명 단어 약어(Bluebook PARTY_ABBR)', () => {
  const c = load('en');
  assert.equal(c.partyAbbr('International Business Machines Corporation v. United States Department of Justice'), "Int'l Bus. Machs. Corp. v. United States Dep't of Justice");
  assert.equal(c.partyAbbr('Smith v. Jones'), 'Smith v. Jones');
  assert.equal(c.partyAbbr('In re Pacific Gas Company Shareholders Litigation'), "In re Pac. Gas Co. S'holders Litig.");
  assert.equal(c.partyAbbr('NATIONAL ASSOCIATION OF MANUFACTURERS v. SEC'), "NAT'L ASS'N OF MANUFACTURERS v. SEC");
});
