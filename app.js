const courses = {
  linear: { title: '線形代数学 第二', code: 'MTH.A202', room: 'S423', instructor: 'Haruka Mori', credits: 2, color: '#7687a5', schedule: 'Mon & Thu · 1–2', language: 'Japanese' },
  parallel: { title: '並列プログラミング', code: 'CSC.T354', room: 'Room —', instructor: 'Suguru Saito / Youmi Ma', credits: 2, color: '#355ba7', schedule: 'Tue & Fri · 3–4', language: 'Japanese' },
  quantum: { title: '量子力学 基礎', code: 'PHY.Q207', room: 'Room —', instructor: 'Kei Nakajima', credits: 2, color: '#806a9d', schedule: 'Wed · 3–4', language: 'Japanese' },
  algorithms: { title: 'データ構造とアルゴリズム', code: 'CSC.T262', room: 'W831', instructor: 'Akira Ito', credits: 2, color: '#5c7d93', schedule: 'Mon · 5–6', language: 'Japanese' },
  english: { title: 'アカデミック英語', code: 'LAJ.A201', room: 'S221', instructor: 'Emma Wilson', credits: 1, color: '#668879', schedule: 'Thu · 7–8', language: 'English' },
  seminar: { title: '研究室ゼミ', code: 'Personal', room: 'Room —', instructor: 'Personal event', credits: 0, color: '#8a887f', schedule: 'Wed · 5–8', language: '—', personal: true },
};

const assignments = [
  ['Problem set 3', 'MTH.A202', '15 Jun, 23:59', 'Overdue', 'overdue'],
  ['Pre-class quiz 4', 'PHY.Q207', '17 Jun, 23:59', 'Due today', 'today'],
  ['OpenMP exercise 2', 'CSC.T354', '18 Jun, 23:59', 'Tomorrow', ''],
  ['Binary tree implementation', 'CSC.T262', '22 Jun, 23:59', 'In 5 days', ''],
  ['Essay draft 2', 'LAJ.A201', '25 Jun, 17:00', 'In 8 days', ''],
];

const previous = [
  ['確率論と統計', 'MTH.C203 · 2026 1Q', 2], ['コンピュータネットワーク', 'CSC.T333 · 2026 1Q', 2],
  ['社会モデリング B', 'LAH.T209 · 2025 4Q', 2], ['Algorithms and Data Structures', 'CSC.T241 · 2025 4Q', 2],
];

const defaultCalendarMetadata = {
  schemaVersion: 1,
  profileYears: [2024, 2025, 2027, 2028, 2029],
  academicYears: {
    2024: { quarters: { '1Q':'04-01', '2Q':'06-10', '3Q':'10-01', '4Q':'12-02' }, events: 39, source: 'Official schedule' },
    2025: { quarters: { '1Q':'04-01', '2Q':'06-09', '3Q':'10-01', '4Q':'12-01' }, events: 41, source: 'Official schedule' },
    2026: { quarters: { '1Q':'04-01', '2Q':'06-10', '3Q':'10-01', '4Q':'12-01' }, events: 42, source: 'Official schedule' },
    2027: { quarters: { '1Q':'04-01', '2Q':'06-11', '3Q':'10-01', '4Q':'12-01' }, events: 40, source: 'Draft pack' },
    2028: { quarters: { '1Q':'04-03', '2Q':'06-09', '3Q':'10-02', '4Q':'12-01' }, events: 40, source: 'Custom' },
    2029: { quarters: { '1Q':'04-02', '2Q':'06-08', '3Q':'10-01', '4Q':'12-03' }, events: 38, source: 'Custom' },
  }
};

const savedCalendarMetadata = (() => {
  try { return JSON.parse(localStorage.getItem('isct-companion-calendar-metadata') || 'null') || defaultCalendarMetadata; }
  catch { return defaultCalendarMetadata; }
})();

const savedNotice = (() => {
  const fallback = { title:'Friday timetable changed', body:'19 June follows the Monday timetable.', action:'View calendar', enabled:true };
  try { return JSON.parse(localStorage.getItem('isct-companion-notice') || 'null') || fallback; }
  catch { return fallback; }
})();

let state = {
  page: 'plan', term: '2026-2Q', drawer: null, drawerTab: 'overview', editorOpen: false,
  catalogSelection: 'CSC.T363', courseView: 'table', webSite: 'portal', settingsSection: 'general',
  appearance: localStorage.getItem('isct-companion-appearance') || 'system', density: localStorage.getItem('isct-companion-density') || 'comfortable',
  calendarMetadata: savedCalendarMetadata, webSettingsSite: 'portal',
  notice: savedNotice, hookEditor: null,
  planned: new Set(['CSC.T363', 'CSC.T273']), comparing: new Set(['CSC.T363', 'CSC.T273']),
};

const catalog = [
  { code:'CSC.T251', title:'Automata and Formal Languages', instructor:'Computer Science faculty', quarter:'2Q', credits:2, language:'Japanese', schedule:'Not published' },
  { code:'CSC.T253', title:'Advanced Procedural Programming', instructor:'Computer Science faculty', quarter:'2Q', credits:2, language:'Japanese', schedule:'Not published' },
  { code:'CSC.T272', title:'Artificial Intelligence', instructor:'Computer Science faculty', quarter:'2Q', credits:2, language:'English', schedule:'Not published' },
  { code:'CSC.T343', title:'Databases', instructor:'Computer Science faculty', quarter:'2Q', credits:2, language:'Japanese', schedule:'Not published' },
  { code:'CSC.T354', title:'Parallel Programming', instructor:'Suguru Saito / Youmi Ma', quarter:'2Q', credits:2, language:'Japanese', schedule:'Tue & Fri · 3–4' },
  { code:'CSC.T261', title:'Logic in Computer Science', instructor:'Makoto Tatsuta', quarter:'3Q', credits:2, language:'English', schedule:'Not published' },
  { code:'CSC.T273', title:'Object-Oriented Programming', instructor:'Computer Science faculty', quarter:'3Q', credits:2, language:'Japanese', schedule:'Tue · 7–8' },
  { code:'CSC.T363', title:'Computer Architecture', instructor:'Computer Science faculty', quarter:'3Q', credits:2, language:'Japanese', schedule:'Wed · 5–6' },
  { code:'CSC.T364', title:'Workshop on System Design', instructor:'Computer Science faculty', quarter:'3Q', credits:2, language:'Japanese', schedule:'Not published' },
  { code:'CSC.T365', title:'Time Series Modeling', instructor:'Computer Science faculty', quarter:'3Q', credits:2, language:'English', schedule:'Not published' },
  { code:'CSC.T371', title:'Information Security', instructor:'Computer Science faculty', quarter:'4Q', credits:2, language:'Japanese', schedule:'Not published' },
  { code:'CSC.T374', title:'Software Design and Development', instructor:'Computer Science faculty', quarter:'4Q', credits:2, language:'English', schedule:'Not published' },
];

const $ = (s) => document.querySelector(s);
const esc = (v='') => String(v).replace(/[&<>'"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;',"'":'&#39;','"':'&quot;'}[c]));

function icon(name) {
  const icons = { today:'▣', plan:'▤', web:'◎', sync:'↻', settings:'⚙', close:'×', prev:'‹', next:'›', plus:'＋', calendar:'▦', external:'↗', home:'⌂' };
  return `<span aria-hidden="true">${icons[name] || ''}</span>`;
}

function render() {
  applyPreferences();
  $('#app').innerHTML = `
    <div class="shell">
      <header class="topbar">
        <div class="brand">Science Tokyo</div>
        <nav class="primary-nav" aria-label="Primary navigation">
          ${navButton('today','Today')}${navButton('plan','Courses')}${navButton('web','Web')}${navButton('settings','Settings')}
        </nav>
        <div class="top-actions">
          <button class="icon-button" data-action="sync" aria-label="Sync">${icon('sync')}</button>
        </div>
      </header>
      <main class="main ${state.page==='web'?'web-main':''}">${state.page === 'today' ? todayPage() : state.page === 'plan' ? planPage() : state.page === 'web' ? webPage() : settingsPage()}</main>
      ${statusBar()}
      ${state.drawer ? drawer() : ''}
      ${state.editorOpen ? editDialog() : ''}
    </div>`;
  bind();
}

function navButton(key, label) {
  return `<button class="nav-button ${state.page === key ? 'active' : ''}" data-page="${key}">${icon(key)} ${label}</button>`;
}

function planPage() {
  const meta = termMeta(state.term);
  return `<div class="courses-heading"><h1 class="page-heading">Courses</h1></div>
    <div class="courses-layout">
      ${quarterTimeline()}
      <section class="quarter-workspace">
        ${quarterContent(meta)}
      </section>
    </div>`;
}

function termMeta(term) {
  const terms = {
    '2025-4Q': { title:'2025 4Q', label:'Completed', kind:'past' },
    '2026-1Q': { title:'2026 1Q', label:'Completed', kind:'past' },
    '2026-2Q': { title:'2026 2Q', label:'Current', kind:'current' },
    '2026-3Q': { title:'2026 3Q', label:'Draft · local', kind:'future' },
    '2026-4Q': { title:'2026 4Q', label:'Not started', kind:'future' },
  };
  return terms[term];
}

function quarterTimeline() {
  const items = [['2025-4Q','2025','4Q'],['2026-1Q','2026','1Q'],['2026-2Q','2026','2Q'],['2026-3Q','2026','3Q'],['2026-4Q','2026','4Q']];
  return `<nav class="quarter-timeline" aria-label="Quarter timeline">${items.map(([value,year,q])=>{ const meta=termMeta(value); return `<button data-quarter-track="${value}" class="${state.term===value?'active':''}"><i></i><span><small>${year}</small><strong>${q}</strong></span><em>${meta.label}</em></button>`; }).join('')}</nav>`;
}

function quarterContent(meta) {
  if (meta.kind === 'past') return previousPlan();
  if (meta.kind === 'current') return currentPlan();
  if (state.term === '2026-4Q') return `<div class="surface empty"><strong>No plan for 2026 4Q</strong><div><button class="button primary" data-mode="edit">Start planning</button></div></div>`;
  return planningView();
}

function currentPlan() {
  return `<div class="section-header"><div><h2 class="section-title">Enrolled courses <span>5 courses · 9 credits</span></h2></div>
    <div class="actions">${layoutSwitch()}<button class="button primary small" data-mode="edit">Edit</button><button class="button small" data-action="add-event">${icon('plus')} Personal event</button><button class="button small" data-action="sync-lms">${icon('sync')} Sync from LMS</button></div></div>
    ${state.courseView==='table'?`<div class="surface">${timetable(false)}</div>`:currentCourseList()}`;
}

function currentCourseList() {
  const keys = ['linear','parallel','quantum','algorithms','english','seminar'];
  return `<div class="surface list-surface course-list">${keys.map(key=>{ const c=courses[key]; return `<div class="list-row"><button class="row-main" data-course="${key}"><strong>${c.title}</strong><p>${c.code} · ${c.schedule}</p></button><span class="credits">${c.personal?'Personal':`${c.credits} credits`}　›</span></div>`; }).join('')}</div>`;
}

function timetable(today=false) {
  const c = (key) => courseCard(key);
  return `<table class="timetable"><thead><tr><th></th>
    <th class="day mon mobile-hide">Mon</th><th class="day tue">Tue${today ? '<small class="no-class">16 · Replacement</small>' : ''}</th><th class="day wed">Wed${today ? '<small class="no-class">17</small>' : ''}</th><th class="day thu">Thu${today ? '<small class="no-class">18</small>' : ''}</th><th class="day fri optional mobile-hide">Fri${!today ? '<small class="no-class">No class</small>' : ''}</th>
    </tr></thead><tbody>
    <tr><th>1–2<small>08:50</small></th><td class="day mon mobile-hide">${c('linear')}</td><td class="day tue">${today ? c('linear') : ''}</td><td class="day wed"></td><td class="day thu">${today ? '' : c('linear')}</td><td class="day fri optional mobile-hide"></td></tr>
    <tr><th>3–4<small>10:45</small></th><td class="day mon mobile-hide"></td><td class="day tue">${c('parallel')}</td><td class="day wed">${c('quantum')}</td><td class="day thu"></td><td class="day fri optional mobile-hide">${today ? '' : c('parallel')}</td></tr>
    <tr class="lunch-row"><th></th><td class="day mon mobile-hide"></td><td class="day tue"></td><td class="day wed">Lunch ${today ? '12:25–13:30' : ''}</td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
    <tr><th>5–6<small>13:30</small></th><td class="day mon mobile-hide">${today ? '' : c('algorithms')}</td><td class="day tue"></td><td class="day wed merged-cell" rowspan="2">${courseCard('seminar', true)}</td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
    <tr><th>7–8<small>15:25</small></th><td class="day mon mobile-hide"></td><td class="day tue"></td><td class="day thu">${c('english')}</td><td class="day fri optional mobile-hide"></td></tr>
    <tr><th>9–10<small>17:15</small></th><td class="day mon mobile-hide"></td><td class="day tue"></td><td class="day wed"></td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
  </tbody></table>`;
}

function courseCard(key, merged=false) {
  const c = courses[key];
  return `<button class="course ${merged?'merged-course':''}" style="--course:${c.color}" data-course="${key}"><strong>${c.title}</strong><span>${c.code}</span><span>${merged?'13:30–17:05 · Periods 5–8':c.room}</span></button>`;
}

function todayPage() {
  return `<h1 class="page-heading">2026 2Q</h1><p class="page-meta">Mon 15 – Fri 19 June 2026　5 courses · 5 open</p>
    ${noticeBlock()}
    <div class="today-layout">
      <section class="surface"><div class="schedule-tools"><div class="tool-cluster"><button class="icon-button">${icon('prev')}</button><button class="icon-button">${icon('next')}</button><button class="button small">Today</button><span class="range">16 Jun – 18 Jun</span></div>
        <button class="button small" data-action="calendar">${icon('calendar')} Calendar</button></div>${timetable(true)}</section>
      <section class="surface assignments"><h2 class="eyebrow">ASSIGNMENTS</h2>${assignments.map(assignmentRow).join('')}</section>
    </div>`;
}

function noticeBlock() {
  if (!state.notice.enabled) return '';
  return `<section class="notice-block" aria-label="Notice"><span class="notice-label">Notice</span><div><strong>${esc(state.notice.title)}</strong><span>${esc(state.notice.body)}</span></div><button class="button small" data-notice-open>${esc(state.notice.action || 'Details')}　›</button></section>`;
}

function assignmentRow(a, i) {
  i = assignments.indexOf(a);
  const selected = state.drawer?.type==='assignment' && state.drawer.index===i;
  return `<div class="assignment ${selected?'selected':''}"><button class="assignment-main" data-assignment="${i}"><strong>${a[0]}</strong><span>${a[1]} · ${a[2]}</span></button><div class="assignment-side"><span class="due ${a[4]}">${a[3]}</span><button class="button ghost small" data-open-lms>LMS ${icon('external')}</button></div></div>`;
}

function previousPlan() {
  return `<div class="section-header"><h2 class="section-title">Completed courses <span>8 credits</span></h2><div class="credits">67 earned credits total</div></div>
    <div class="surface list-surface">${previous.map((p,i)=>`<div class="list-row"><button class="row-main" data-previous="${i}"><strong>${p[0]}</strong><p>${p[1]}</p></button><span class="credits">${p[2]} credits　›</span></div>`).join('')}</div>`;
}

function planningView() {
  return `<div class="section-header"><div><h2 class="section-title">Draft courses <span>${state.term} · 2 candidates</span></h2></div><div class="actions">${layoutSwitch()}<button class="button primary small" data-mode="edit">Edit plan</button></div></div>
    ${state.planned.has('CSC.T363')?'<div class="conflict"><span><strong>1 conflict needs attention.</strong> Open Edit to review it.</span><button class="button small" data-mode="edit">Review</button></div>':''}
    ${state.courseView==='table'?`<div class="surface">${planningTimetable()}</div>`:plannedCourseList()}`;
}

function layoutSwitch() {
  return `<div class="mode-switch" role="tablist" aria-label="Course layout"><button data-layout="table" class="${state.courseView==='table'?'active':''}" aria-selected="${state.courseView==='table'}">Table</button><button data-layout="list" class="${state.courseView==='list'?'active':''}" aria-selected="${state.courseView==='list'}">List</button></div>`;
}

function plannedCourseList() {
  const rows = catalog.filter(c=>state.planned.has(c.code));
  return `<div class="surface list-surface course-list">${rows.map(c=>`<div class="list-row"><button class="row-main" data-mode="edit"><strong>${c.title}</strong><p>${c.code} · ${c.schedule}</p></button><span class="credits">${c.credits} credits　›</span></div>`).join('')}</div>`;
}

function editDialog() {
  const quarter = state.term.split('-')[1];
  const quarterOptions = ['1Q','2Q','3Q','4Q'].map(q=>`<option ${q===quarter?'selected':''}>${q}</option>`).join('');
  const results = catalog.filter(c=>c.quarter===quarter);
  const selected = results.find(c=>c.code===state.catalogSelection) || results[0];
  const added = state.planned.has(selected.code);
  return `<div class="editor-backdrop" data-editor-close></div>
    <section class="edit-dialog" role="dialog" aria-modal="true" aria-labelledby="edit-title">
      <header class="edit-dialog-header"><h2 id="edit-title">Edit ${termMeta(state.term).title}</h2><div class="actions"><a class="button syllabus-link" href="https://syllabus.s.isct.ac.jp/courses/2026/search?hl=en" target="_blank" rel="noreferrer">Syllabus ↗</a><button class="button primary" data-editor-close>Done</button></div></header>
      <div class="edit-dialog-body">
        <section class="editor-compose">
          <div class="editor-timetable"><div class="editor-pane-title"><strong>Timetable</strong><span>${termMeta(state.term).kind==='current'?5:state.planned.size} courses</span></div><div class="surface">${termMeta(state.term).kind==='current'?timetable(false):planningTimetable()}</div></div>
          <div class="editor-catalog">
            <aside class="catalog-filters" aria-label="Course filters"><h3>Filters</h3>
              <label>Year<select><option>2026</option><option>2025</option></select></label>
              <label>Quarter<select data-editor-quarter>${quarterOptions}</select></label>
              <label>Department<select><option>Computer Science</option><option>Mathematics</option><option>Physics</option></select></label>
              <fieldset><legend>Level</legend><label><input type="checkbox" checked> 200</label><label><input type="checkbox" checked> 300</label></fieldset>
            </aside>
            <section class="catalog-results" aria-label="Filtered courses"><div class="catalog-search"><input class="input" value="CSC.T" aria-label="Search courses"><span>${results.length} courses</span></div>
              <div class="catalog-list">${results.map(c=>`<button class="catalog-row ${selected.code===c.code?'active':''}" data-catalog-select="${c.code}"><span><strong>${c.title}</strong><small>${c.code} · ${c.quarter} · ${c.credits} credits</small></span><em>${state.planned.has(c.code)?'Added':'›'}</em></button>`).join('')}</div></section>
          </div>
        </section>
        <aside class="catalog-detail" aria-label="Course details"><div class="detail-kicker">${selected.code}</div><h3>${selected.title}</h3>
          <div class="fact-grid"><div class="label">Instructor</div><div>${selected.instructor}</div><div class="label">Quarter</div><div>2026 ${selected.quarter}</div><div class="label">Schedule</div><div class="${selected.schedule==='Not published'?'unknown':''}">${selected.schedule}</div><div class="label">Credits</div><div>${selected.credits}</div><div class="label">Language</div><div>${selected.language}</div></div>
          <div class="detail-actions"><button class="button ${added?'danger':'primary'}" data-toggle-plan="${selected.code}">${added?'Remove from timetable':'Add to timetable'}</button><a href="https://syllabus.s.isct.ac.jp/courses/2026/search?hl=en" target="_blank" rel="noreferrer">Open course syllabus ↗</a></div>
        </aside>
      </div>
    </section>`;
}

function planningTimetable() {
  const candidate = (title, code, room, conflict=false) => `<button class="course candidate-course ${conflict?'has-conflict':''}" style="--course:${conflict?'#a33d32':'#5d719f'}"><strong>${title}</strong><span>${code}</span><span>${room}${conflict?' · Conflict':''}</span></button>`;
  return `<table class="timetable"><thead><tr><th></th><th class="day mon mobile-hide">Mon</th><th class="day tue">Tue</th><th class="day wed">Wed</th><th class="day thu">Thu</th><th class="day fri optional mobile-hide">Fri</th></tr></thead><tbody>
    <tr><th>1–2<small>08:50</small></th><td class="day mon mobile-hide"></td><td class="day tue"></td><td class="day wed"></td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
    <tr><th>3–4<small>10:45</small></th><td class="day mon mobile-hide"></td><td class="day tue"></td><td class="day wed"></td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
    <tr class="lunch-row"><th></th><td class="day mon mobile-hide"></td><td class="day tue"></td><td class="day wed">Lunch</td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
    <tr><th>5–6<small>13:30</small></th><td class="day mon mobile-hide"></td><td class="day tue"></td><td class="day wed">${state.planned.has('CSC.T363')?candidate('Computer Architecture','CSC.T363','Draft time',true):''}</td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
    <tr><th>7–8<small>15:25</small></th><td class="day mon mobile-hide"></td><td class="day tue">${state.planned.has('CSC.T273')?candidate('Object-Oriented Programming','CSC.T273','Draft time'):''}</td><td class="day wed"></td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
    <tr><th>9–10<small>17:15</small></th><td class="day mon mobile-hide"></td><td class="day tue"></td><td class="day wed"></td><td class="day thu"></td><td class="day fri optional mobile-hide"></td></tr>
  </tbody></table>`;
}

function plannedPlan() {
  const candidates = [
    ['CSC.T363','Computer Architecture','Wed 5–6 · illustrative','2 credits'],
    ['CSC.T273','Object-Oriented Programming','Tue 7–8 · illustrative','2 credits'],
    ['MEC.C201','Engineering Design','Thu 5–6 · I124','2 credits'],
  ];
  const selected = candidates.filter(c => state.planned.has(c[0]));
  return `<div class="section-header"><div><h2 class="section-title">Edit plan <span>${selected.length} candidates · ${state.term}</span></h2></div><button class="button primary small" data-action="add-course">${icon('plus')} Add course</button></div>
    ${state.planned.has('CSC.T363') ? `<div class="conflict"><span><strong>Time conflict:</strong> CSC.T363 clashes with 研究室ゼミ at Wed 5–6 in this illustrative draft.</span><button class="button danger small" data-remove="CSC.T363">Remove CSC.T363</button></div>` : ''}
    <div class="surface list-surface">${candidates.map(c => `<div class="list-row"><label class="candidate-select"><input type="checkbox" data-candidate="${c[0]}" ${state.planned.has(c[0])?'checked':''}><span><strong>${c[1]}</strong><p>${c[0]} · ${c[2]}</p></span></label><span class="credits">${c[3]}</span></div>`).join('')}</div>
    ${selected.length >= 2 ? comparison(selected) : ''}`;
}

function comparison(selected) {
  const a = selected[0], b = selected[1];
  const rows = [['Code',a[0],b[0]],['Schedule',a[2],b[2]],['Credits',a[3],b[3]],['Exam','<em class="unknown">Unknown</em>','<em class="unknown">Unknown</em>'],['Workload','<em class="unknown">Unknown</em>','<em class="unknown">Unknown</em>'],['Status','Candidate','Candidate']];
  return `<section class="surface comparison"><h2 class="section-title">Comparison (${selected.length})</h2><div class="comparison-grid">${rows.map((r,i)=>`<div class="label">${r[0]}</div><div>${r[1]}</div><div>${r[2]}</div>`).join('')}</div></section>`;
}

function webPage() {
  const services = {
    portal: { label:'Portal', url:'https://portal.isct.ac.jp/en/' },
    lms: { label:'T2SCHOLA', url:'https://lms.s.isct.ac.jp/' },
    syllabus: { label:'Syllabus', url:'https://syllabus.s.isct.ac.jp/courses/2026/search?hl=en' },
  };
  const active = services[state.webSite];
  return `<section class="web-viewer" aria-label="Web viewer">
    <nav class="web-tabs" aria-label="University websites">${Object.entries(services).map(([key,s])=>`<button class="${state.webSite===key?'active':''}" data-web-site="${key}">${s.label}</button>`).join('')}</nav>
    <div class="browser-bar"><div class="browser-controls"><button class="icon-button" disabled aria-label="Back">${icon('prev')}</button><button class="icon-button" data-web-action="reload" aria-label="Reload">${icon('sync')}</button></div><div class="browser-address"><span>${icon('home')}</span><span>${active.url}</span></div><button class="button small" data-web-action="external">Open in browser ${icon('external')}</button></div>
    <iframe id="web-frame" class="web-frame" src="${active.url}" title="${active.label}" referrerpolicy="no-referrer" allow="clipboard-read; clipboard-write"></iframe>
  </section>`;
}

function settingsPage() {
  const sections = {
    general: generalSettings(), accounts: accountSettings(), webview: webviewSettings(), notifications: notificationSettings(),
    metadata: metadataSettings(), hooks: hookSettings(), custom: customSettings(), logs: logSettings(), about: aboutSettings(),
  };
  const nav = [
    ['Student',[['general','General'],['notifications','Notifications'],['accounts','Accounts']]],
    ['Academic',[['metadata','Academic calendar'],['hooks','Data & hooks']]],
    ['Advanced',[['webview','Web viewer'],['custom','Customization'],['logs','Logs']]],
    ['', [['about','About']]],
  ];
  return `<h1 class="page-heading">Settings</h1><div class="settings-layout"><nav class="settings-nav" aria-label="Settings sections">${nav.map(([group,items])=>`${group?`<span class="settings-nav-label">${group}</span>`:''}${items.map(([key,label])=>`<button class="${state.settingsSection===key?'active':''}" data-settings-section="${key}">${label}</button>`).join('')}`).join('')}</nav>${sections[state.settingsSection]}</div>`;
}

function settingsPanel(title, body, actions='') {
  return `<section class="settings-panel"><div class="settings-panel-head"><h2>${title}</h2>${actions}</div>${body}</section>`;
}

function settingRow(label, control, hint='') {
  return `<div class="setting-row"><div><strong>${label}</strong>${hint?`<small>${hint}</small>`:''}</div><div>${control}</div></div>`;
}

function selectControl(name, value, options, disabled=false) {
  return `<select class="setting-control" data-pref="${name}" ${disabled?'disabled':''}>${options.map(([v,l])=>`<option value="${v}" ${v===value?'selected':''}>${l}</option>`).join('')}</select>`;
}

function toggleControl(label, checked=true) {
  return `<label class="switch"><input type="checkbox" ${checked?'checked':''}><span></span><em>${label}</em></label>`;
}

function generalSettings() {
  const language = selectControl('language','en',[['en','English'],['jp','日本語'],['zh-cn','简体中文']],true);
  const appearance = selectControl('appearance',state.appearance,[['system','System'],['light','Light'],['dark','Dark']]);
  const density = selectControl('density',state.density,[['comfortable','Comfortable'],['compact','Compact'],['spacious','Spacious']]);
  return settingsPanel('General', settingRow('Language',language,'Planned: en · jp · zh-cn')+settingRow('Appearance',appearance)+settingRow('Layout density',density));
}

function accountSettings() {
  return settingsPanel('Accounts',
    settingRow('Science Tokyo account','<span class="setting-status">Not connected</span><button class="button small">Configure</button>')+
    settingRow('TokyoTech account','<span class="setting-status">Not connected</span><button class="button small">Configure</button>')+
    settingRow('Matrix code','<span class="setting-status">Not configured</span><button class="button small">Configure</button>')+
    settingRow('Science Tokyo OTP','<span class="setting-status">Not configured</span><button class="button small">Configure</button>')+
    `<div class="settings-subhead"><strong>Cloud sync</strong><span>PWA backup</span></div>`+
    cloudProviderRow('gdrive','G','Google Drive')+cloudProviderRow('onedrive','O','OneDrive')+
    settingRow('Sync frequency',`<select class="setting-control"><option>On change</option><option>Every hour</option><option>Daily</option><option>Manual</option></select>`)+
    settingRow('Conflict handling',`<select class="setting-control"><option>Ask me</option><option>Newest wins</option><option>Keep both</option></select>`)+
    settingRow('Synced data',`<div class="sync-scope"><label><input type="checkbox" checked> Courses</label><label><input type="checkbox" checked> Calendar</label><label><input type="checkbox" checked> Settings</label><label><input type="checkbox" checked> Personal events</label></div>`)
  );
}

function cloudProviderRow(key, mark, name) {
  return `<div class="provider-row"><span class="provider-mark ${key}">${mark}</span><div><strong>${name}</strong><small>Not connected</small></div><button class="button small" data-cloud-connect="${key}">Connect</button></div>`;
}

function webviewSettings() {
  const sites = { portal:['Portal','portal.isct.ac.jp'], lms:['T2SCHOLA','lms.s.isct.ac.jp'], syllabus:['Syllabus','syllabus.s.isct.ac.jp'] };
  const active = sites[state.webSettingsSite];
  const tabs = `<div class="calendar-tabs web-settings-tabs" role="tablist" aria-label="Web viewer sites">${Object.entries(sites).map(([key,site])=>`<button class="${state.webSettingsSite===key?'active':''}" data-web-settings-site="${key}">${site[0]}</button>`).join('')}</div>`;
  return settingsPanel('Web viewer', tabs+`<div class="viewer-target"><div><span class="pack-kicker">CURRENT TARGET</span><strong>${active[0]}</strong><small>${active[1]}</small></div><span class="status-pill">Electron webview</span></div>`+settingRow('Custom injection',toggleControl('Enabled',false),'This website only')+settingRow('CSS injection',`<textarea class="code-input" id="webview-css" rows="7" placeholder="body { … }">${esc(localStorage.getItem(`isct-companion-webview-${state.webSettingsSite}-css`)||'')}</textarea>`)+settingRow('JavaScript injection',`<textarea class="code-input" id="webview-js" rows="7" placeholder="document.querySelector(…)">${esc(localStorage.getItem(`isct-companion-webview-${state.webSettingsSite}-js`)||'')}</textarea>`), `<button class="button primary small" data-save-custom="webview">Save ${active[0]}</button>`);
}

function notificationSettings() {
  return settingsPanel('Notifications', settingRow('Push notifications',toggleControl('Enabled'))+settingRow('Assignments',toggleControl('Due dates'))+settingRow('Course updates',toggleControl('Changes'))+settingRow('Portal banners',toggleControl('New notices'))+settingRow('Quiet hours',`<input class="setting-control" type="time" value="22:00">`));
}

function metadataSettings() {
  const metadata = state.calendarMetadata;
  const profile = metadata.profileYears || [];
  const years = Object.entries(metadata.academicYears || {}).sort(([a],[b])=>+a-+b);
  const profileMap = `<div class="profile-year-map">${profile.map((year,i)=>`<div><span>Grade ${i+1}</span><strong>${esc(year)}</strong></div>`).join('')}</div>`;
  const yearRows = years.map(([year,data])=>`<div class="metadata-year-row"><strong>${esc(year)}</strong><span>${Object.entries(data.quarters||{}).map(([q,date])=>`${esc(q)} ${esc(date)}`).join('　')}</span><span>${esc(data.events||0)} events</span><span>${esc(data.source||'Custom')}</span></div>`).join('');
  const raw = esc(JSON.stringify(metadata,null,2));
  return settingsPanel('Academic calendar',
    `<div class="metadata-status"><div><span class="pack-kicker">STUDENT PATH</span><strong>${profile.length} grades · ${profile.map(esc).join(' → ')}</strong></div><span class="status-pill success">Valid</span></div>${profileMap}
    <div class="settings-subhead"><strong>Academic year metadata</strong><span>${years.length} years</span></div>
    <div class="metadata-years"><div class="metadata-year-head"><span>Year</span><span>Quarter starts</span><span>Calendar</span><span>Source</span></div>${yearRows}</div>
    <div class="settings-subhead"><strong>Calendar subscription</strong><span>Published by annual workflow</span></div>
    <div class="calendar-publish"><div><span class="provider-mark google-calendar">31</span></div><div><strong>Science Tokyo academic calendar</strong><small>Public Google Calendar · updated from validated metadata</small></div><span class="status-pill success">Published</span><button class="button small" data-calendar-subscribe>Subscribe</button></div>
    <details class="raw-metadata"><summary><span><strong>Advanced</strong><small>Edit the complete metadata document</small></span><span>JSON　⌄</span></summary><form id="metadata-form"><textarea class="code-input metadata-code" id="metadata-json" rows="18" spellcheck="false">${raw}</textarea><div class="raw-actions"><span>Local changes replace generated values on this device.</span><button type="button" class="button small" data-reset-metadata>Reset</button><button class="button primary small" type="submit">Parse & save</button></div></form></details>`,
    '<button class="button small" data-setting-demo="Metadata exported">Export JSON</button>'
  );
}

function hookSettings() {
  const rows = [
    ['notice','Notices','Today','On publish',state.notice.enabled?'Active':'Hidden'],
    ['calendar','Academic calendar','Google Calendar','Annual','Active'],
    ['courses','Course data','LMS','Manual','Active'],
    ['exports','Exports','Webhook','Scheduled','Paused'],
  ];
  return settingsPanel('Data & hooks', `<div class="hook-table"><div class="hook-head"><span>Data</span><span>Target</span><span>Schedule</span><span>Status</span><span></span></div>${rows.map(r=>`<div class="hook-row"><strong>${r[1]}</strong><span>${r[2]}</span><span>${r[3]}</span><span>${r[4]}</span><button class="button ghost small" data-edit-hook="${r[0]}">Edit</button></div>`).join('')}</div>${hookEditor()}`);
}

function hookEditor() {
  if (!state.hookEditor) return '';
  if (state.hookEditor !== 'notice') return `<div class="hook-editor"><div class="hook-editor-head"><div><span class="pack-kicker">${esc(state.hookEditor.toUpperCase())}</span><strong>Edit hook</strong></div><button class="icon-button" data-close-hook aria-label="Close">×</button></div>${settingRow('Schedule','<select class="setting-control"><option>Automatic</option><option>Daily</option><option>Manual</option></select>')}${settingRow('Status',toggleControl('Enabled'))}<div class="form-actions"><button class="button" data-close-hook>Cancel</button><button class="button primary" data-save-hook="generic">Save</button></div></div>`;
  return `<form class="hook-editor" id="notice-hook-form"><div class="hook-editor-head"><div><span class="pack-kicker">TODAY</span><strong>Edit notice</strong></div><button type="button" class="icon-button" data-close-hook aria-label="Close">×</button></div><div class="form-row"><label>Title</label><input class="input" name="title" value="${esc(state.notice.title)}" required></div><div class="form-row"><label>Message</label><textarea class="input" name="body" rows="3" required>${esc(state.notice.body)}</textarea></div><div class="form-two"><div class="form-row"><label>Action label</label><input class="input" name="action" value="${esc(state.notice.action)}"></div><div class="form-row"><label>Visibility</label><label class="switch hook-switch"><input name="enabled" type="checkbox" ${state.notice.enabled?'checked':''}><span></span><em>Show on Today</em></label></div></div><div class="form-actions"><button type="button" class="button" data-close-hook>Cancel</button><button class="button primary" type="submit">Save notice</button></div></form>`;
}

function customSettings() {
  return settingsPanel('Customization', settingRow('Background image',`<input class="setting-control wide" id="background-input" value="${esc(localStorage.getItem('isct-companion-background')||'')}" placeholder="Image URL">`)+settingRow('App CSS override',`<textarea class="code-input" id="custom-css" rows="7" placeholder=".course { … }">${esc(localStorage.getItem('isct-companion-custom-css')||'')}</textarea>`)+`<div class="settings-subhead"><strong>Custom values</strong><button class="button small" data-setting-demo="Custom setting added">Add value</button></div>`+settingRow('campus','Ookayama')+settingRow('default_department','Computer Science')+settingRow('show_room_codes','true'), '<button class="button primary small" data-save-custom="custom">Save</button>');
}

function logSettings() {
  const logs = [['10:42:18','LMS sync','5 courses updated'],['10:41:54','Metadata','Calendar source checked'],['09:03:11','Notification','Assignment reminder delivered']];
  return settingsPanel('Logs', `<div class="log-list">${logs.map(l=>`<div><time>${l[0]}</time><strong>${l[1]}</strong><span>${l[2]}</span></div>`).join('')}</div>`, '<button class="button small">Export</button><button class="button ghost small">Clear</button>');
}

function aboutSettings() {
  return settingsPanel('About', settingRow('IsctCompanion','Version 0.1.0')+settingRow('Privacy policy','<button class="button ghost small">Open ↗</button>')+settingRow('Licenses','<button class="button ghost small">Open ↗</button>')+settingRow('Contact','<button class="button ghost small">Open ↗</button>')+settingRow('Source code','<button class="button ghost small">Open ↗</button>'));
}

function applyPreferences() {
  document.documentElement.dataset.theme=state.appearance;
  document.documentElement.dataset.density=state.density;
  const background=localStorage.getItem('isct-companion-background')||'';
  document.body.style.backgroundImage=background?`url("${background.replace(/["\\]/g,'')}")`:'';
  let style=$('#user-css');
  if (!style) { style=document.createElement('style'); style.id='user-css'; document.head.appendChild(style); }
  style.textContent=localStorage.getItem('isct-companion-custom-css')||'';
}

function statusBar() {
  return `<footer class="statusbar"><div class="status-left"><span><i class="dot"></i>Up to date</span><span>Last success 17 Jun 10:42</span><button class="button ghost small" data-action="sync">↻ Sync now</button></div><div class="status-right"><span>Demo data</span><span>67 + 9 / 124 credits (illustrative)</span><span>2026 2Q</span></div></footer>`;
}

function drawer() {
  if (state.drawer.type === 'assignment') return `<aside class="drawer drawer-left" role="dialog" aria-modal="false" aria-label="Assignment details">${drawerContent()}</aside>`;
  return `<div class="backdrop" data-close></div><aside class="drawer" role="dialog" aria-modal="true">${drawerContent()}</aside>`;
}

function drawerContent() {
  if (state.drawer.type === 'course') return courseDrawer(state.drawer.key);
  if (state.drawer.type === 'assignment') return assignmentDrawer(state.drawer.index);
  if (state.drawer.type === 'event') return eventDrawer();
  if (state.drawer.type === 'calendar') return calendarDrawer();
  return calendarDrawer();
}

function drawerHead(title, subtitle='') {
  return `<div class="drawer-header"><div><h2>${title}</h2>${subtitle?`<div class="drawer-subtitle">${subtitle}</div>`:''}</div><button class="icon-button" data-close aria-label="Close">${icon('close')}</button></div>`;
}

function courseDrawer(key) {
  const c = courses[key];
  if (c.personal) return eventDrawer(true);
  const tabs = ['overview','materials','assignments'];
  return `${drawerHead(c.title, `${c.code} · ${c.schedule}`)}<div class="drawer-body"><div class="drawer-tabs">${tabs.map(t=>`<button class="drawer-tab ${state.drawerTab===t?'active':''}" data-drawer-tab="${t}">${t[0].toUpperCase()+t.slice(1)}</button>`).join('')}</div>
  ${state.drawerTab === 'overview' ? `<div class="fact-grid"><div class="label">Course code</div><div>${c.code}</div><div class="label">Instructor</div><div>${c.instructor}</div><div class="label">Schedule</div><div>${c.schedule}</div><div class="label">Room</div><div class="${c.room.includes('—')?'unknown':''}">${c.room}</div><div class="label">Credits</div><div>${c.credits}</div><div class="label">Language</div><div>${c.language}</div></div><div class="verified">✓ Verified course fields · <u>Open Science Tokyo Syllabus ↗</u></div><section class="drawer-section"><h3>About</h3><p>This is illustrative course content for the local prototype. Check the original syllabus and LMS for authoritative details.</p></section>` :
  state.drawerTab === 'materials' ? `<section class="drawer-section"><h3>Course resources</h3>${['Week 4 · Lecture notes','Week 3 · Practice worksheet','Course guide'].map((x,i)=>`<div class="resource-row"><span class="resource-kind">${i===1?'DOC':'PDF'}</span><strong>${x}</strong><button class="button ghost small">Open ↗</button></div>`).join('')}</section>` :
  `<section class="drawer-section"><h3>Open assignments</h3>${assignments.filter(a=>a[1]===c.code).map((a,i)=>assignmentRow(a,i)).join('') || '<p class="unknown">No open assignments.</p>'}</section>`}
  </div>`;
}

function assignmentDrawer(i) {
  const a = assignments[i];
  return `${drawerHead(a[0], `${a[1]} · due ${a[2]}`)}<div class="drawer-body"><section class="drawer-section"><h3>Status</h3><p><strong class="due ${a[4]}">${a[3]}</strong></p></section><section class="drawer-section"><h3>Instructions</h3><p>Complete the activity on the original LMS. This local prototype never submits coursework.</p></section><button class="button primary" data-open-lms>Open in LMS ${icon('external')}</button></div>`;
}

function eventDrawer(edit=false) {
  return `${drawerHead(edit?'Edit personal event':'Add personal event', 'Saved locally in this browser')}<form class="drawer-body" id="event-form"><div class="form-row"><label>Event</label><input class="input" name="title" value="${edit?'研究室ゼミ':''}" placeholder="e.g. Lab seminar" required></div><div class="form-two"><div class="form-row"><label>Day</label><select class="input" name="day"><option>Wednesday</option><option>Monday</option><option>Tuesday</option><option>Thursday</option><option>Friday</option></select></div><div class="form-row"><label>Periods</label><select class="input" name="period"><option>5–6</option><option>1–2</option><option>3–4</option><option>7–8</option></select></div></div><div class="form-row"><label>Room</label><input class="input" name="room" value="${edit?'Room —':''}" placeholder="Optional"></div><div class="form-row"><label><input type="checkbox" checked> Block this time in planned schedules</label></div><div class="form-actions">${edit?'<button type="button" class="button danger" data-action="delete-event">Delete</button>':''}<button type="button" class="button" data-close>Cancel</button><button class="button primary" type="submit">Save event</button></div></form>`;
}

function calendarDrawer() {
  return `${drawerHead('Calendar & holidays','Exceptions affect occurrences, not enrollment')}<div class="drawer-body"><section class="drawer-section"><h3>School calendar</h3><div class="list-row"><div><strong>University no-class day</strong><p>19 Jun 2026 · illustrative</p></div><span class="credits">Imported</span></div><div class="list-row"><div><strong>Replacement Monday timetable</strong><p>16 Jun 2026 · illustrative</p></div><span class="credits">Imported</span></div></section><section class="drawer-section"><h3>Actions</h3><button class="button">Import from school website</button> <button class="button">Export timetable</button></section><div class="verified">Demo only · no calendar account is connected.</div></div>`;
}

function bind() {
  document.querySelectorAll('[data-page]').forEach(b=>b.onclick=()=>{ state.page=b.dataset.page; state.drawer=null; state.editorOpen=false; render(); });
  document.querySelectorAll('[data-quarter-track]').forEach(b=>b.onclick=()=>{ state.term=b.dataset.quarterTrack; state.editorOpen=false; render(); });
  document.querySelectorAll('[data-layout]').forEach(b=>b.onclick=()=>{ state.courseView=b.dataset.layout; render(); });
  document.querySelectorAll('[data-mode]').forEach(b=>b.onclick=()=>{ state.editorOpen=b.dataset.mode==='edit'; render(); });
  document.querySelectorAll('[data-editor-close]').forEach(b=>b.onclick=()=>{ state.editorOpen=false; render(); });
  document.querySelectorAll('[data-catalog-select]').forEach(b=>b.onclick=()=>{ state.catalogSelection=b.dataset.catalogSelect; render(); });
  document.querySelectorAll('[data-editor-quarter]').forEach(s=>s.onchange=()=>{ state.term=`2026-${s.value}`; state.catalogSelection=''; render(); });
  document.querySelectorAll('[data-web-site]').forEach(b=>b.onclick=()=>{ state.webSite=b.dataset.webSite; render(); });
  document.querySelectorAll('[data-web-settings-site]').forEach(b=>b.onclick=()=>{ state.webSettingsSite=b.dataset.webSettingsSite; render(); });
  document.querySelectorAll('[data-web-action]').forEach(b=>b.onclick=()=>webAction(b.dataset.webAction));
  document.querySelectorAll('[data-settings-section]').forEach(b=>b.onclick=()=>{ state.settingsSection=b.dataset.settingsSection; render(); });
  document.querySelectorAll('[data-edit-hook]').forEach(b=>b.onclick=()=>{ state.hookEditor=b.dataset.editHook; render(); });
  document.querySelectorAll('[data-close-hook]').forEach(b=>b.onclick=()=>{ state.hookEditor=null; render(); });
  document.querySelectorAll('[data-save-hook="generic"]').forEach(b=>b.onclick=()=>{ state.hookEditor=null; render(); toast('Hook settings saved'); });
  document.querySelectorAll('[data-notice-open]').forEach(b=>b.onclick=()=>{ state.drawer={type:'calendar'}; render(); });
  document.querySelectorAll('[data-pref]').forEach(control=>control.onchange=()=>{ state[control.dataset.pref]=control.value; localStorage.setItem(`isct-companion-${control.dataset.pref}`,control.value); render(); });
  document.querySelectorAll('[data-save-custom]').forEach(b=>b.onclick=()=>{
    if (b.dataset.saveCustom==='webview') { localStorage.setItem(`isct-companion-webview-${state.webSettingsSite}-css`,$('#webview-css').value); localStorage.setItem(`isct-companion-webview-${state.webSettingsSite}-js`,$('#webview-js').value); }
    else { localStorage.setItem('isct-companion-background',$('#background-input').value); localStorage.setItem('isct-companion-custom-css',$('#custom-css').value); }
    applyPreferences(); toast('Settings saved');
  });
  document.querySelectorAll('[data-setting-demo]').forEach(b=>b.onclick=()=>toast(b.dataset.settingDemo));
  document.querySelectorAll('[data-calendar-subscribe]').forEach(b=>b.onclick=()=>toast('Google Calendar subscription opened'));
  document.querySelectorAll('[data-reset-metadata]').forEach(b=>b.onclick=()=>{ state.calendarMetadata=structuredClone(defaultCalendarMetadata); localStorage.removeItem('isct-companion-calendar-metadata'); render(); toast('Metadata reset'); });
  document.querySelectorAll('[data-cloud-connect]').forEach(b=>b.onclick=()=>toast(`${b.dataset.cloudConnect==='gdrive'?'Google Drive':'OneDrive'} OAuth setup required`));
  document.querySelectorAll('[data-toggle-plan]').forEach(b=>b.onclick=()=>{ const code=b.dataset.togglePlan; const adding=!state.planned.has(code); adding?state.planned.add(code):state.planned.delete(code); render(); toast(`${code} ${adding?'added to':'removed from'} ${state.term}`); });
  document.querySelectorAll('[data-course]').forEach(b=>b.onclick=()=>{ state.drawer={type:'course',key:b.dataset.course}; state.drawerTab='overview'; render(); });
  document.querySelectorAll('[data-assignment]').forEach(b=>b.onclick=()=>{ state.drawer={type:'assignment',index:+b.dataset.assignment}; render(); });
  document.querySelectorAll('[data-open-lms]').forEach(b=>b.onclick=(e)=>{ e.stopPropagation(); state.page='web'; state.webSite='lms'; state.drawer=null; render(); });
  document.querySelectorAll('[data-drawer-tab]').forEach(b=>b.onclick=()=>{ state.drawerTab=b.dataset.drawerTab; render(); });
  document.querySelectorAll('[data-close]').forEach(b=>b.onclick=()=>{ state.drawer=null; render(); });
  document.querySelectorAll('[data-candidate]').forEach(b=>b.onchange=()=>{ b.checked?state.planned.add(b.dataset.candidate):state.planned.delete(b.dataset.candidate); render(); });
  document.querySelectorAll('[data-remove]').forEach(b=>b.onclick=()=>{ state.planned.delete(b.dataset.remove); render(); });
  document.querySelectorAll('[data-add-result]').forEach(b=>b.onclick=()=>{ toast(`${b.dataset.addResult} added to ${state.term}`); });
  document.querySelectorAll('[data-action]').forEach(b=>b.onclick=()=>action(b.dataset.action));
  const form = $('#event-form'); if (form) form.onsubmit=(e)=>{ e.preventDefault(); localStorage.setItem('isct-companion-event', JSON.stringify(Object.fromEntries(new FormData(form)))); state.drawer=null; render(); toast('Event saved locally'); };
  const metadataForm = $('#metadata-form'); if (metadataForm) metadataForm.onsubmit=(e)=>{ e.preventDefault(); try { const next=JSON.parse($('#metadata-json').value); if (!Array.isArray(next.profileYears) || !next.academicYears || typeof next.academicYears!=='object') throw new Error(); state.calendarMetadata=next; localStorage.setItem('isct-companion-calendar-metadata',JSON.stringify(next)); render(); toast('Metadata parsed and saved'); } catch { toast('Invalid metadata · check profileYears and academicYears'); } };
  const noticeForm = $('#notice-hook-form'); if (noticeForm) noticeForm.onsubmit=(e)=>{ e.preventDefault(); state.notice={ title:noticeForm.elements.title.value, body:noticeForm.elements.body.value, action:noticeForm.elements.action.value, enabled:noticeForm.elements.enabled.checked }; localStorage.setItem('isct-companion-notice',JSON.stringify(state.notice)); state.hookEditor=null; render(); toast('Today notice updated'); };
}

function action(a) {
  if (a==='add-event') state.drawer={type:'event'};
  else if (a==='calendar') state.drawer={type:'calendar'};
  else if (a==='sync-lms') { toast('Courses synced from LMS'); return; }
  else if (a==='add-course') state.editorOpen=true;
  else if (a==='delete-event') { state.drawer=null; toast('Event removed · Undo available in demo'); }
  else if (a==='sync') { toast('Mock data refreshed'); return; }
  render();
}

function webAction(action) {
  const frame=$('#web-frame');
  if (action==='reload' && frame) frame.src=frame.src;
  if (action==='external' && frame) window.open(frame.src, '_blank', 'noopener');
}

function toast(message) {
  const t=document.createElement('div'); t.textContent=message; Object.assign(t.style,{position:'fixed',right:'18px',bottom:'44px',zIndex:50,background:'#20211f',color:'white',padding:'10px 13px',borderRadius:'6px',fontSize:'11px',boxShadow:'0 8px 24px rgba(0,0,0,.18)'}); document.body.appendChild(t); setTimeout(()=>t.remove(),2200);
}

document.addEventListener('keydown', e=>{
  if (e.key==='Escape' && state.editorOpen) { state.editorOpen=false; render(); }
  else if (e.key==='Escape' && state.drawer) { state.drawer=null; render(); }
  if ((e.metaKey||e.ctrlKey) && ['1','2','3','4'].includes(e.key)) { e.preventDefault(); state.page={1:'today',2:'plan',3:'web',4:'settings'}[e.key]; state.drawer=null; render(); }
});

render();
