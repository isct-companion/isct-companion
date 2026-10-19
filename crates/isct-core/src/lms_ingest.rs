use crate::data::{resolve_course_id, versioned::content_hash, DataError};
use crate::school_parser::lms::{id_of, lms_url, AssignState, LmsPayload};
use chrono::{Datelike, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("{0}")]
    InvalidPayload(String),
    #[error("{0}")]
    Storage(String),
}

impl From<String> for IngestError {
    fn from(message: String) -> Self {
        Self::InvalidPayload(message)
    }
}

impl From<&str> for IngestError {
    fn from(message: &str) -> Self {
        Self::InvalidPayload(message.to_string())
    }
}

impl From<DataError> for IngestError {
    fn from(error: DataError) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<rusqlite::Error> for IngestError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl From<serde_json::Error> for IngestError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidPayload(error.to_string())
    }
}

/// Read-only Moodle transport. Every request passes through `call`, whose
/// closed allowlist is also checked before the request is constructed.
pub const LMS_EXTRACT_SCRIPT: &str = concat!(
    r#"(async () => {
  const enrichCourseContent = "#,
    include_str!("lms_course_content.js"),
    r#";
  const forumContent = "#,
    include_str!("lms_forum_content.js"),
    r#";
  const resolveLmsResource = "#,
    include_str!("lms_resource_resolve.js"),
    r#";
  const submissionStatus = "#,
    include_str!("lms_submission_status.js"),
    r#";
  const methods = new Set([
    'core_course_get_enrolled_courses_by_timeline_classification',
    'core_courseformat_get_state',
    'core_calendar_get_action_events_by_timesort',
    'core_calendar_get_action_events_by_course',
    'core_calendar_get_calendar_upcoming_view',
    'core_calendar_get_calendar_monthly_view',
    'core_course_get_recent_courses'
  ]);
  const sourceUrl = location.href;
  const explicitCourseId = Number(globalThis.__ISCT_COMPANION_LMS_COURSE_ID__) || 0;
  const fullScan = globalThis.__ISCT_COMPANION_LMS_FULL__ === true;
  // The phones' refresh: the summary plus the content of the courses on the
  // current quarter's timetable, in one pass.
  const currentScan = !fullScan && globalThis.__ISCT_COMPANION_LMS_CURRENT__ === true;
  const reportProgress = value => {
    try { globalThis.__ISCT_COMPANION_LMS_PROGRESS__?.(value); } catch {}
  };
  const login = /\/(login|auth)(\/|$)/i.test(location.pathname)
    || Boolean(document.querySelector('input[type=password], form[action*="login" i]'));
  if (login) return {status:'login', sourceUrl, language:'', coursesComplete:false,
    courses:[], structures:[], actionEvents:[], datedEvents:[]};
  if (location.hostname !== 'lms.s.isct.ac.jp' || !location.pathname.startsWith('/2025/'))
    return {status:'unsupported', sourceUrl, language:'', coursesComplete:false,
      courses:[], structures:[], actionEvents:[], datedEvents:[]};
  const cfg = globalThis.M && globalThis.M.cfg;
  if (!cfg || typeof cfg.sesskey !== 'string' || !cfg.sesskey)
    return {status:'login', sourceUrl, language:'', coursesComplete:false,
      courses:[], structures:[], actionEvents:[], datedEvents:[]};
  const pageHasCourse = /^\/2025\/(course\/view\.php|mod\/[^/]+\/view\.php)$/.test(location.pathname);
  const pageCourseId = pageHasCourse && Number(cfg.courseId) > 1 ? Number(cfg.courseId) : 0;
  // Explicit refreshes win. Normal browsing is scoped to the course represented
  // by the loaded Moodle page; the dashboard remains summary-only.
  const requestedCourseId = explicitCourseId || (!fullScan ? pageCourseId : 0);
  const call = async (methodname, args) => {
    if (!methods.has(methodname)) throw new Error('method-not-allowed');
    const endpoint = '/2025/lib/ajax/service.php?sesskey=' + encodeURIComponent(cfg.sesskey)
      + '&info=' + encodeURIComponent(methodname);
    const response = await fetch(endpoint, {method:'POST', credentials:'same-origin',
      signal:AbortSignal.timeout(20000),
      headers:{'Content-Type':'application/json'},
      body:JSON.stringify([{index:0, methodname, args}])});
    if (!response.ok || !(response.headers.get('content-type') || '').includes('json'))
      throw new Error('reconnect-required');
    const envelope = await response.json();
    const result = Array.isArray(envelope) && envelope[0];
    if (!result || result.error) throw new Error(result?.exception?.errorcode || 'api-error');
    return result.data;
  };
  const pick = (object, keys) => Object.fromEntries(keys.filter(key => object && key in object).map(key => [key, object[key]]));
  // The dashboard's POST is a read-only year/quarter query. Only these three
  // filter fields and this exact endpoint are allowed; no Moodle edits run.
  const readTimetable = (page = document) => {
    const block = page.querySelector('[data-block="tttimetable"]');
    const [grid, other] = block ? block.querySelectorAll('table') : [];
    if (!grid || grid.rows.length < 2 || grid.rows[0].cells.length < 6 || !other) return null;
    const selected = name => block.querySelector('select[name="' + name + '"]')?.selectedOptions[0]?.textContent.trim() || '';
    const year = Number(selected('year')), quarter = selected('quota');
    if (!Number.isInteger(year) || year < 2000 || year >= 2100 || !/^[1-4]$/.test(quarter)) return null;
    const slots = [];
    const courseId = link => {
      const url = new URL(link.getAttribute('href'), sourceUrl);
      return url.origin === location.origin && url.pathname === '/2025/course/view.php'
        ? Number(url.searchParams.get('id')) : 0;
    };
    for (const row of [...grid.rows].slice(1)) {
      const period = Number(row.cells[0]?.textContent.trim());
      if (!Number.isInteger(period) || period < 1 || period > 12) return null;
      [...row.cells].slice(1).forEach((cell, index) => {
        for (const link of cell.querySelectorAll('a[href*="/course/view.php?id="]')) {
          const id = courseId(link);
          if (id > 0 && index < 7) slots.push({courseId:id, day:index + 1, period});
        }
      });
    }
    const unscheduled = [...other.querySelectorAll('a[href*="/course/view.php?id="]')].map(courseId).filter(id => id > 0);
    return {year, quarter, slots, unscheduled};
  };
  const readTimetables = async () => {
    const current = readTimetable();
    const grids = current ? [current] : [];
    const errors = [];
    if (!fullScan || requestedCourseId) return {grids, errors};
    const block = document.querySelector('[data-block="tttimetable"]');
    const years = [...block?.querySelectorAll('select[name="year"] option') || []]
      .filter(option => /^20\d{2}$/.test(option.textContent.trim()));
    const quarters = [...block?.querySelectorAll('select[name="quota"] option') || []]
      .filter(option => /^[1-4]$/.test(option.textContent.trim()));
    // Preserve the form's search flag; an empty replacement makes Moodle
    // ignore year/quota and return the current quarter for every request.
    const search = block?.querySelector('input[name="search"]')?.value ?? '';
    if (!years.length || quarters.length !== 4) errors.push('timetable-filters-unavailable');
    for (const year of years) for (const quarter of quarters) {
      const expected = year.textContent.trim() + '-' + quarter.textContent.trim() + 'Q';
      if (current && expected === current.year + '-' + current.quarter + 'Q') continue;
      try {
        const response = await fetch('/2025/my/index.php', {method:'POST', credentials:'same-origin',
          signal:AbortSignal.timeout(20000), headers:{'Content-Type':'application/x-www-form-urlencoded'},
          body:new URLSearchParams({year:year.value, quota:quarter.value, search}).toString()});
        if (!response.ok || new URL(response.url, sourceUrl).origin !== location.origin)
          throw new Error('timetable-request-failed: HTTP ' + response.status);
        const page = new DOMParser().parseFromString(await response.text(), 'text/html');
        const grid = readTimetable(page);
        if (!grid) {
          const block = page.querySelector('[data-block="tttimetable"]');
          const tables = [...block?.querySelectorAll('table') || []];
          const labels = [...block?.querySelectorAll('select') || []]
            .map(select => select.name + '=' + select.selectedOptions[0]?.textContent.trim()).join(',');
          throw new Error('timetable-unreadable: tables=' + tables.length + ', rows='
            + tables.map(table => table.rows.length).join(',') + ', selection=' + labels);
        }
        if (expected !== grid.year + '-' + grid.quarter + 'Q')
          throw new Error('timetable-quarter-mismatch: got ' + grid.year + '-' + grid.quarter + 'Q');
        grids.push(grid);
      } catch (error) { errors.push(expected + ': ' + String(error?.message || error)); }
    }
    return {grids, errors};
  };
  try {
    const {grids:timetables, errors:timetableErrors} = await readTimetables();
    const courses = [];
    let offset = 0;
    let coursesComplete = false;
    reportProgress({phase:'catalog', completed:0, total:1});
    for (let page = 0; page < 20; page += 1) {
      const data = await call('core_course_get_enrolled_courses_by_timeline_classification',
        {classification:'all', limit:0, offset, sort:'fullname'});
      const batch = Array.isArray(data?.courses) ? data.courses : [];
      courses.push(...batch.slice(0, 500 - courses.length).map(course => pick(course,
        ['id','fullname','shortname','idnumber','coursecategory','startdate','enddate','viewurl','hidden'])));
      const next = Number(data?.nextoffset);
      if (!Number.isFinite(next) || next <= offset || batch.length === 0) {
        coursesComplete = true;
        break;
      }
      offset = next;
      if (courses.length >= 500) throw new Error('course-limit');
    }
    reportProgress({phase:'catalog', completed:1, total:1});
    const actionEvents = [];
    let aftereventid;
    for (let page = 0; page < 100; page += 1) {
      const args = {timesortfrom:0, limitnum:50};
      if (aftereventid !== undefined) args.aftereventid = aftereventid;
      if (requestedCourseId) args.courseid = requestedCourseId;
      const data = await call(requestedCourseId
        ? 'core_calendar_get_action_events_by_course'
        : 'core_calendar_get_action_events_by_timesort', args);
      const batch = Array.isArray(data?.events) ? data.events : [];
      actionEvents.push(...batch.slice(0, 5000 - actionEvents.length).map(event => ({
        ...pick(event, ['id','instance','modulename','name','activityname','timesort','overdue','eventtype','url']),
        course:{id:event.course?.id}})));
      const last = batch.at(-1);
      const next = Number(data?.lastid ?? last?.id);
      if (batch.length < 50 || !Number.isFinite(next) || next === aftereventid) break;
      aftereventid = next;
      if (actionEvents.length >= 5000) throw new Error('event-limit');
    }
    // Course pages already provide their own action events. The dashboard/full
    // modes also keep non-action calendar entries used by Today.
    let datedEvents = [];
    if (!requestedCourseId) {
      const upcoming = await call('core_calendar_get_calendar_upcoming_view',
        {courseid:1, categoryid:0});
      datedEvents = (Array.isArray(upcoming?.events) ? upcoming.events.slice(0, 5000) : []).map(event => ({
        ...pick(event, ['id','instance','modulename','name','timesort','eventtype','url']), course:{id:event.course?.id}}));
    }
    // The category name is the course's quarter ("1"…"4").
    const structures = [];
    let contentBudget = 3000000;
    const boundedBody = text => { const body = text.slice(0, Math.min(100000, contentBudget)); contentBudget -= body.length; return body; };
    const courseContext = course => {
      const year = Number(String(course.shortname || '').match(/\[(\d{4})\]\s*$/)?.[1]);
      const quarter = Number(course.coursecategory);
      return {year, quarter, term:Number.isInteger(year) && quarter >= 1 && quarter <= 4 ? year + '-' + quarter + 'Q' : ''};
    };
    const currentGrid = timetables[0];
    const currentIds = new Set(currentGrid ? [...currentGrid.slots.map(slot => slot.courseId), ...currentGrid.unscheduled] : []);
    const selectedCourses = courses.filter(course => {
      if (requestedCourseId) return Number(course.id) === requestedCourseId;
      return fullScan || (currentScan && currentIds.has(Number(course.id)));
    });
    let structuresDone = 0;
    const loadStructure = async course => {
      const context = courseContext(course);
      try {
        const raw = await call('core_courseformat_get_state', {courseid:Number(course.id)});
        const full = JSON.parse(typeof raw === 'string' ? raw : raw?.data);
        const state = {section:(full?.section || []).map(item => pick(item, ['id','number','title','visible','anchor'])),
          cm:(full?.cm || []).map(item => pick(item, ['id','name','module','url','sectionid','visible','uservisible','anchor']))};
        // Shared course-format DOM only: preserve teacher content without
        // interpreting headings, dates or course-specific conventions.
        try {
          const response = await fetch('/2025/course/view.php?id=' + Number(course.id), {credentials:'same-origin', signal:AbortSignal.timeout(20000)});
          if (response.ok && response.url.includes('/course/view.php')) {
            const page = new DOMParser().parseFromString(await response.text(), 'text/html');
            enrichCourseContent(page, state, response.url, boundedBody);
          }
        } catch { /* Keep the API outline when the optional body GET fails. */ }
        structures.push({courseId:Number(course.id), complete:true, state});
      } catch (error) {
        structures.push({courseId:Number(course.id), complete:false, state:null,
          error:String(error?.message || error)});
      } finally {
        structuresDone += 1;
        reportProgress({phase:'course', completed:structuresDone, total:selectedCourses.length,
          courseId:Number(course.id), courseTitle:String(course.fullname || course.shortname || course.id), term:context.term});
      }
    };
    reportProgress({phase:'course', completed:0, total:selectedCourses.length,
      courseId:Number(selectedCourses[0]?.id) || 0,
      courseTitle:String(selectedCourses[0]?.fullname || selectedCourses[0]?.shortname || ''),
      term:selectedCourses[0] ? courseContext(selectedCourses[0]).term : ''});
    // Fetch deadlines include response bodies. A fixed worker pool keeps the
    // other slots moving when one school request is slow. No transport library
    // is available inside the injected, standalone browser script.
    const readWithWorkers = async (items, read) => {
      let next = 0;
      await Promise.all(Array.from({length:Math.min(4, items.length)}, async () => {
        while (next < items.length) await read(items[next++]);
      }));
    };
    await readWithWorkers(selectedCourses, loadStructure);
    // The upcoming view and open-work events forget finished work, so a full
    // pull reads each month of the enrolled academic years up to now; past
    // due/close events still carry the deadline of submitted and graded work.
    if (fullScan && !requestedCourseId) {
      const today = new Date();
      const months = [];
      for (const year of new Set(courses.map(course => courseContext(course).year).filter(Number.isInteger))) {
        for (let index = 0; index < 12 && months.length < 48; index += 1) {
          const month = new Date(year, 3 + index, 1);
          if (month <= today) months.push({year:month.getFullYear(), month:month.getMonth() + 1});
        }
      }
      await readWithWorkers(months, async ({year, month}) => {
        try {
          const view = await call('core_calendar_get_calendar_monthly_view',
            {year, month, courseid:1, categoryid:0, day:1});
          for (const week of Array.isArray(view?.weeks) ? view.weeks : []) {
            for (const day of Array.isArray(week?.days) ? week.days : []) {
              for (const event of Array.isArray(day?.events) ? day.events : []) {
                if (!['assign','quiz'].includes(event.modulename) || !['due','close'].includes(event.eventtype)) continue;
                if (datedEvents.length >= 5000) return;
                datedEvents.push({...pick(event, ['id','instance','modulename','name','eventtype','url']),
                  timesort:event.timesort ?? event.timestart, course:{id:event.course?.id}});
              }
            }
          }
        } catch {
          // A month that cannot be read leaves the deadlines already stored.
        }
      });
    }
    // Bounded read-only module details, using the same signed-in session.
    const courseById = new Map(courses.map(course => [Number(course.id), course]));
    // Two layers. A pull across courses reads only what Today needs: the
    // structure, section text and assignment states. Material details
    // (files, folders, links, pages, announcements) are read for one course
    // at a time, when it is opened in the app or refreshed there; files also
    // resolve on demand. Cached details stay meanwhile.
    // Details are written onto the returned state's own module objects. Each
    // course gets its own share, newest modules first, so a large or
    // fast-loading course cannot starve the rest; cached details fill gaps.
    const detailModules = requestedCourseId || fullScan || currentScan
      ? ['resource','folder','url','page','forum','assign']
      : ['assign'];
    const detailCms = structures.flatMap(item => (item.state?.cm || [])
      .filter(cm => cm.uservisible !== false && detailModules.includes(cm.module))
      .reverse().slice(0, 40).map(cm => ({cm, courseId:item.courseId}))).slice(0, 320);
    const forumBudget = new Map();
    const assignStatus = [];
    const readDetail = async (cm, courseId) => {
      try {
        const target = new URL(cm.url, sourceUrl);
        if (target.origin !== location.origin || !/^\/2025\/mod\/(resource|folder|url|page|forum|assign)\/view\.php$/.test(target.pathname)) return;
        const response = await fetch(target.href, {credentials:'same-origin', signal:AbortSignal.timeout(20000)});
        if (!response.ok) return;
        if (new URL(response.url).origin !== location.origin) { response.body?.cancel(); return; }
        if (response.url.includes('/pluginfile.php/')) {
          cm.resourceUrl = response.url; response.body?.cancel(); return;
        }
        if (!(response.headers.get('content-type') || '').includes('text/html')) { response.body?.cancel(); return; }
        const text = await response.text();
        if (text.length > 1000000) return;
        const page = new DOMParser().parseFromString(text, 'text/html');
        if (page.querySelector('input[type=password]') || !page.body.classList.contains('cmid-' + cm.id)) return;
        if (cm.module === 'assign') {
          const status = submissionStatus({page, cmid:Number(cm.id), module:'assign'});
          if (status) assignStatus.push(status);
        }
        const nodes = cm.module === 'forum' ? page.querySelectorAll('#intro, .forumheaderlist')
          : cm.module === 'folder' ? page.querySelectorAll('#intro, .foldertree')
          : cm.module === 'page' ? page.querySelectorAll('#intro, .generalbox')
          : page.querySelectorAll('#intro');
        if (cm.module === 'forum') {
          const discussions = [...new Set([...page.getElementsByTagName('a')].map(a => {
            try { const u = new URL(a.getAttribute('href'), response.url);
              if (u.origin !== location.origin || u.pathname !== '/2025/mod/forum/discuss.php' || !/^\d+$/.test(u.searchParams.get('d') || '')) return null;
              const id = u.searchParams.get('d'); u.hash = ''; u.search = '?d=' + id; return u.href;
            } catch { return null; }
          }).filter(Boolean))].slice(0, Math.min(8, forumBudget.get(courseId) ?? 16));
          forumBudget.set(courseId, (forumBudget.get(courseId) ?? 16) - discussions.length);
          const bodies = [];
          for (const url of discussions) {
            try {
              const detail = await fetch(url, {credentials:'same-origin', signal:AbortSignal.timeout(20000)});
              if (!detail.ok || new URL(detail.url).origin !== location.origin || !(detail.headers.get('content-type') || '').includes('text/html')) { detail.body?.cancel(); continue; }
              const html = await detail.text();
              if (html.length > 1000000) continue;
              bodies.push(forumContent(new DOMParser().parseFromString(html, 'text/html'), detail.url));
            } catch { /* Preserve the other announcements when one GET fails. */ }
          }
          const intro = page.getElementById('intro')?.innerHTML || '';
          if (contentBudget > 0) cm.bodyHtml = boundedBody(intro + bodies.join('\n'));
        } else if (contentBudget > 0) cm.bodyHtml = boundedBody([...nodes].map(node => node.innerHTML).join('\n'));
        const resolved = resolveLmsResource({page, responseUrl:response.url,
          cmid:Number(cm.id), module:String(cm.module || '')});
        if (resolved.kind === 'file' || resolved.kind === 'external')
          cm.resourceUrl = resolved.url;
      } catch { /* Keep cached detail on a later incomplete read. */ }
    };
    let detailsDone = 0;
    const readDetailWithProgress = async ({cm, courseId}) => {
      await readDetail(cm, courseId);
      detailsDone += 1;
      const course = courseById.get(Number(courseId));
      reportProgress({phase:'material', completed:detailsDone, total:detailCms.length,
        courseId:Number(courseId) || 0, courseTitle:String(course?.fullname || course?.shortname || ''),
        term:course ? courseContext(course).term : '', itemTitle:String(cm.name || '')});
    };
    await readWithWorkers(detailCms, readDetailWithProgress);
    // Submission state as the assignment or quiz page shows it (a plain GET),
    // for the work of the loaded quarters.
    const statusRead = new Set(assignStatus.map(item => item.cmid));
    const assignIds = structures.flatMap(item => (item.state?.cm || [])
      .filter(cm => cm.module === 'assign' || cm.module === 'quiz')
      .map(cm => ({id:Number(cm.id), module:cm.module, courseId:item.courseId, name:String(cm.name || '')})))
      .filter(item => item.id > 0 && !statusRead.has(item.id)).slice(0, 60);
    let statusesDone = 0;
    const readStatus = async item => {
      const cmid = item.id;
      try {
        const response = await fetch('/2025/mod/' + item.module + '/view.php?id=' + cmid, {credentials:'same-origin', signal:AbortSignal.timeout(20000)});
        if (!response.ok) return;
        const page = new DOMParser().parseFromString(await response.text(), 'text/html');
        const status = submissionStatus({page, cmid, module:item.module});
        if (status) assignStatus.push(status);
      } catch {} finally {
        statusesDone += 1;
        const course = courseById.get(Number(item.courseId));
        reportProgress({phase:'assignment', completed:statusesDone, total:assignIds.length,
          courseId:Number(item.courseId) || 0, courseTitle:String(course?.fullname || course?.shortname || ''),
          term:course ? courseContext(course).term : '', itemTitle:item.name});
      }
    };
    await readWithWorkers(assignIds, readStatus);
    return {status:'ok', sourceUrl, language:String(cfg.language || ''), coursesComplete,
      fullScan:fullScan && !requestedCourseId,
      courses, structures, actionEvents, datedEvents, timetables, timetableErrors, assignStatus};
  } catch (error) {
    return {status:String(error?.message || error) === 'reconnect-required' ? 'login' : 'error',
      sourceUrl, language:String(cfg.language || ''), coursesComplete:false,
      courses:[], structures:[], actionEvents:[], datedEvents:[],
      error:String(error?.message || error)};
  }
})()"#
);

fn split_title(fullname: &str, language: &str) -> Value {
    if let Some((ja, en)) = fullname.split_once(" / ") {
        return json!({"ja": ja.trim(), "en": en.trim()});
    }
    if language.to_ascii_lowercase().starts_with("ja") {
        json!({"ja": fullname.trim()})
    } else {
        json!({"en": fullname.trim()})
    }
}

fn year_of(shortname: &str) -> Option<u32> {
    let open = shortname.rfind('[')?;
    shortname
        .get(open + 1..shortname.len().checked_sub(1)?)?
        .parse()
        .ok()
}

/// The LMS category name is the quarter the course starts in (verified
/// 2026-10-03 for 2025 and 2026 courses); anything else is unplaced.
fn quarter_of(_year: Option<u32>, category: u64) -> Vec<String> {
    match category {
        1..=4 => vec![format!("{category}Q")],
        _ => Vec::new(),
    }
}

fn upsert_versioned(
    tx: &Transaction<'_>,
    table: &str,
    id: &str,
    value: &Value,
    columns: &[(&str, rusqlite::types::Value)],
) -> Result<bool, IngestError> {
    let hash = content_hash(value)?;
    let existing: Option<(String, Option<String>)> = tx
        .query_row(
            &format!("SELECT hash, missing_since FROM {table} WHERE id=?1"),
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(IngestError::from)?;
    let timestamp = Utc::now().to_rfc3339();
    if existing.as_ref().is_some_and(|(stored, _)| stored == &hash) {
        let changed = existing
            .as_ref()
            .is_some_and(|(_, missing)| missing.is_some());
        tx.execute(
            &format!("UPDATE {table} SET last_seen_at=?2, missing_since=NULL, deleted_at=NULL,
                value=json_remove(json_set(value,'$.lastSeenAt',?2),'$.missingSince','$.deletedAt') WHERE id=?1"),
            params![id, timestamp],
        ).map_err(IngestError::from)?;
        return Ok(changed);
    }
    let mut names = vec![
        "id",
        "value",
        "hash",
        "updated_at",
        "updated_by",
        "rev",
        "source",
        "deleted_at",
        "last_seen_at",
        "missing_since",
    ];
    names.extend(columns.iter().map(|(name, _)| *name));
    let marks = (1..=names.len())
        .map(|n| format!("?{n}"))
        .collect::<Vec<_>>()
        .join(",");
    let updates = names
        .iter()
        .skip(1)
        .map(|name| format!("{name}=excluded.{name}"))
        .collect::<Vec<_>>()
        .join(",");
    let old_rev: u64 = tx
        .query_row(&format!("SELECT rev FROM {table} WHERE id=?1"), [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(IngestError::from)?
        .unwrap_or(0);
    let rev = old_rev + 1;
    let mut stored = value.clone();
    if let Some(object) = stored.as_object_mut() {
        object.insert("rev".into(), rev.into());
        object.insert("updatedAt".into(), timestamp.clone().into());
        object.insert("updatedBy".into(), "lms".into());
        object.insert("source".into(), json!({"provider":"lms"}));
        object.insert("lastSeenAt".into(), timestamp.clone().into());
        object.remove("missingSince");
        object.remove("deletedAt");
    }
    let json = serde_json::to_string(&stored).map_err(IngestError::from)?;
    let mut values = vec![
        id.to_string().into(),
        json.into(),
        hash.into(),
        timestamp.clone().into(),
        String::from("lms").into(),
        (rev as i64).into(),
        String::from("lms").into(),
        rusqlite::types::Value::Null,
        timestamp.clone().into(),
        rusqlite::types::Value::Null,
    ];
    values.extend(columns.iter().map(|(_, value)| value.clone()));
    tx.execute(
        &format!(
            "INSERT INTO {table} ({}) VALUES ({marks}) ON CONFLICT(id) DO UPDATE SET {updates}",
            names.join(",")
        ),
        rusqlite::params_from_iter(values),
    )
    .map_err(IngestError::from)?;
    Ok(true)
}

struct AssignmentSource<'a> {
    cmid: u64,
    lms_course: u64,
    course: &'a str,
    title: &'a str,
    url: &'a str,
}

/// What the stored row already says, for a pull that could not read the page.
struct PriorAssignment {
    submission: String,
    due_at: Option<String>,
    grade: Option<String>,
    accepts_submission: Option<bool>,
}

/// One assignment row. `open` while Moodle lists it as work to do; a
/// previously open one that left that list before its deadline is submitted.
fn write_assignment(
    tx: &Transaction<'_>,
    source: &AssignmentSource<'_>,
    due: Option<i64>,
    listed_open: bool,
    page_state: Option<&AssignState>,
) -> Result<bool, IngestError> {
    let cmid = source.cmid;
    let prior = tx
        .query_row(
            "SELECT submission, due_at, json_extract(value,'$.grade'), json_extract(value,'$.acceptsSubmission') FROM assignments WHERE lms_cmid=?1 AND missing_since IS NULL",
            [cmid],
            |r| {
                Ok(PriorAssignment {
                    submission: r.get(0)?,
                    due_at: r.get(1)?,
                    grade: r.get(2)?,
                    accepts_submission: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(IngestError::from)?;
    let effective_due = due.or_else(|| {
        prior
            .as_ref()
            .and_then(|value| value.due_at.as_deref())
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.timestamp())
    });
    // The assignment page is authoritative; the event list is only a hint.
    // Keep the last known due date when Moodle temporarily omits its event.
    let submission = if let Some(state) = page_state {
        state.submission.as_str()
    } else if listed_open {
        "open"
    } else if prior.as_ref().map(|value| value.submission.as_str()) == Some("open")
        && effective_due.is_some_and(|d| d > Utc::now().timestamp())
    {
        "submitted"
    } else {
        // A pull that could not read the page keeps finished work finished.
        prior
            .as_ref()
            .map(|value| value.submission.as_str())
            .filter(|p| matches!(*p, "submitted" | "graded"))
            .unwrap_or("unknown")
    };
    let due_at = due
        .and_then(|d| chrono::DateTime::from_timestamp(d, 0))
        .map(|d| d.to_rfc3339())
        .or_else(|| prior.as_ref().and_then(|value| value.due_at.clone()));
    // The page's grade when it was read; otherwise the last one seen, while
    // the work still counts as graded.
    let grade = match page_state {
        Some(state) => state.grade.clone(),
        None => prior
            .as_ref()
            .and_then(|value| value.grade.clone())
            .filter(|_| submission == "graded"),
    };
    let mut assignment = json!({"id":format!("lms-assignment:{cmid}"),"sourceKey":format!("lms:default:assignment:{cmid}"),"kind":"assignment","courseId":source.course,
        "lmsCmid":cmid,"lmsCourseId":source.lms_course,"title":source.title,"details":"","sourceUrl":source.url,
        "dueAt":due_at,"submission":submission});
    if let Some(grade) = grade {
        assignment["grade"] = grade.into();
    }
    // Whether LMS still takes work past the deadline; the last page read wins.
    let accepts = match page_state {
        Some(state) => state.accepts_submission,
        None => prior.as_ref().and_then(|value| value.accepts_submission),
    };
    if let Some(accepts) = accepts {
        assignment["acceptsSubmission"] = accepts.into();
    }
    upsert_versioned(
        tx,
        "assignments",
        &format!("lms-assignment:{cmid}"),
        &assignment,
        &[
            ("title", source.title.to_string().into()),
            (
                "due_at",
                due_at
                    .map(Into::into)
                    .unwrap_or(rusqlite::types::Value::Null),
            ),
            ("source_url", source.url.to_string().into()),
            ("lms_cmid", (cmid as i64).into()),
            ("lms_course_id", (source.lms_course as i64).into()),
            ("submission", submission.to_string().into()),
        ],
    )
}

pub fn apply_lms_payload(
    connection: &mut Connection,
    payload: &LmsPayload,
) -> Result<usize, IngestError> {
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(IngestError::from)?;
    let mut changed = 0;
    let run_id = Uuid::new_v4().to_string();
    let started = Utc::now().to_rfc3339();
    let scope = if payload.full_scan
        && payload.courses_complete
        && payload.timetable_errors.is_empty()
        && payload
            .structures
            .iter()
            .all(|structure| structure.complete)
    {
        "all"
    } else {
        "summary"
    };
    tx.execute("INSERT INTO import_runs(id,provider,scope,state,started_at) VALUES(?1,'lms',?2,'running',?3)", params![run_id, scope, started]).map_err(IngestError::from)?;
    let action_ids: HashSet<u64> = payload
        .action_events
        .iter()
        .filter_map(|v| id_of(v.get("instance")))
        .collect();
    let dated: HashMap<u64, i64> = payload
        .dated_events
        .iter()
        .filter(|v| {
            matches!(
                v.get("eventtype").and_then(Value::as_str),
                Some("due" | "close")
            )
        })
        .filter_map(|v| Some((id_of(v.get("instance"))?, v.get("timesort")?.as_i64()?)))
        .collect();
    // Due dates: the calendar's due event, else the open-work event's time.
    let action_due: HashMap<u64, i64> = payload
        .action_events
        .iter()
        .filter_map(|v| Some((id_of(v.get("instance"))?, v.get("timesort")?.as_i64()?)))
        .collect();
    let due_of = |cmid: u64| dated.get(&cmid).or_else(|| action_due.get(&cmid)).copied();
    let mut written = HashSet::<u64>::new();
    let mut course_ids = HashMap::<u64, String>::new();
    for course in &payload.courses {
        let year = year_of(&course.shortname);
        let offering = (course.idnumber.len() == 9
            && course.idnumber.bytes().all(|b| b.is_ascii_digit()))
        .then(|| course.idnumber.clone());
        let lms_course_id = format!("lms-course:{}", course.id);
        // Weekly meetings by term from the dashboard grid; terms this pull did
        // not show keep what an earlier pull saw.
        let mut timetables = tx
            .query_row(
                "SELECT value FROM lms_courses WHERE id=?1",
                [&lms_course_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(IngestError::from)?
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|stored| stored.get("timetables").cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        for grid in payload.timetables.iter().chain(payload.timetable.iter()) {
            let shown = grid
                .meetings
                .get(&course.id)
                .cloned()
                .or_else(|| grid.unscheduled.contains(&course.id).then(Vec::new));
            if year.is_some_and(|year| grid.term.starts_with(&format!("{year}-"))) {
                if let Some(map) = timetables.as_object_mut() {
                    map.insert(
                        grid.term.clone(),
                        shown.map(Value::Array).unwrap_or(Value::Null),
                    );
                }
            }
        }
        let prior_sections: Option<Value> = tx
            .query_row(
                "SELECT value FROM lms_courses WHERE id=?1",
                [&lms_course_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(IngestError::from)?
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|stored| stored.get("sections").cloned());
        let mut sections = payload
            .structures
            .iter()
            .find(|s| s.course_id == course.id && s.complete)
            .and_then(|s| s.state.as_ref())
            .and_then(|s| s.get("section"))
            .cloned()
            .or_else(|| prior_sections.clone())
            .unwrap_or_else(|| json!([]));
        if let Some(current) = sections.as_array_mut() {
            for section in current {
                if section.get("bodyHtml").is_none() {
                    if let Some(body) = prior_sections
                        .as_ref()
                        .and_then(Value::as_array)
                        .and_then(|old| {
                            old.iter()
                                .find(|s| id_of(s.get("id")) == id_of(section.get("id")))
                        })
                        .and_then(|s| s.get("bodyHtml"))
                    {
                        if let Some(section) = section.as_object_mut() {
                            section.insert("bodyHtml".into(), body.clone());
                        }
                    }
                }
            }
        }
        let value = json!({"id":lms_course_id,"sourceKey":format!("lms:default:course:{}",course.id),"lmsId":course.id,"offeringId":offering,
            "shortname":course.shortname,"title":split_title(&course.fullname,&payload.language),"year":year,
            "quarters":quarter_of(year,course.coursecategory),"url":course.viewurl,"hidden":course.hidden,
            "timetables":timetables,"sections":sections});
        changed += usize::from(upsert_versioned(
            &tx,
            "lms_courses",
            &lms_course_id,
            &value,
            &[("lms_id", (course.id as i64).into())],
        )?);
        let offering_id = value.get("offeringId").and_then(Value::as_str);
        let lms_id = course.id.to_string();
        let academic_year = year.unwrap_or_else(|| Utc::now().year() as u32) as i32;
        let course_id = resolve_course_id(&tx, "lms", &lms_id, academic_year, offering_id)?;
        if let Some(offering_id) = offering_id {
            resolve_course_id(
                &tx,
                "syllabus",
                offering_id,
                academic_year,
                Some(offering_id),
            )?;
        }
        course_ids.insert(course.id, course_id);
    }
    if payload.courses_complete {
        let live: HashSet<String> = payload
            .courses
            .iter()
            .map(|c| format!("lms-course:{}", c.id))
            .collect();
        let mut statement = tx
            .prepare("SELECT id FROM lms_courses WHERE missing_since IS NULL")
            .map_err(IngestError::from)?;
        let old = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(IngestError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(IngestError::from)?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE lms_courses SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(IngestError::from)?;
        }
    }
    for structure in payload.structures.iter().filter(|s| s.complete) {
        let mut live_assignments = HashSet::new();
        let state = structure
            .state
            .as_ref()
            .ok_or("complete LMS structure has no state")?;
        let sections: HashMap<u64, (&str, i64)> = state
            .get("section")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|s| {
                Some((
                    id_of(s.get("id"))?,
                    (
                        s.get("title").and_then(Value::as_str).unwrap_or(""),
                        s.get("number").and_then(Value::as_i64).unwrap_or(0),
                    ),
                ))
            })
            .collect();
        let cms = state
            .get("cm")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut live = HashSet::new();
        for cm in cms {
            let Some(cmid) = id_of(cm.get("id")) else {
                continue;
            };
            let module = cm.get("module").and_then(Value::as_str).unwrap_or("");
            let url = cm.get("url").and_then(Value::as_str).unwrap_or("");
            if !url.is_empty() && !lms_url(url) {
                return Err("LMS item URL failed validation".into());
            }
            let section = id_of(cm.get("sectionid"))
                .and_then(|id| sections.get(&id).copied())
                .unwrap_or(("", 0));
            let item_id = format!("lms-item:{cmid}");
            live.insert(item_id.clone());
            let linked_course = course_ids
                .get(&structure.course_id)
                .cloned()
                .ok_or_else(|| "LMS structure referenced an unknown course".to_string())?;
            let name = cm.get("name").and_then(Value::as_str).unwrap_or("");
            let previous: Option<Value> = tx
                .query_row("SELECT value FROM lms_items WHERE id=?1", [&item_id], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
                .map_err(IngestError::from)?
                .and_then(|raw| serde_json::from_str(&raw).ok());
            let body = cm
                .get("bodyHtml")
                .or_else(|| previous.as_ref().and_then(|v| v.get("bodyHtml")));
            let resource_url = cm
                .get("resourceUrl")
                .or_else(|| previous.as_ref().and_then(|v| v.get("resourceUrl")));
            // A folder's files come from opening it, not from the crawl; keep them.
            let folder_files = previous.as_ref().and_then(|v| v.get("folderFiles"));
            let item = json!({"id":item_id,"sourceKey":format!("lms:default:material:{cmid}"),"kind":"material","courseId":linked_course,"cmid":cmid,"lmsCourseId":structure.course_id,"sectionNumber":section.1,
                "sectionId":id_of(cm.get("sectionid")),"bodyHtml":body,"resourceUrl":resource_url,"folderFiles":folder_files,"sectionTitle":section.0,"module":module,"name":cm.get("name").and_then(Value::as_str).unwrap_or(""),
                "session":section.1,"title":name,"type":"link","url":url,"visible":cm.get("visible").and_then(Value::as_bool).unwrap_or(false) && cm.get("uservisible").and_then(Value::as_bool).unwrap_or(false)});
            changed += usize::from(upsert_versioned(
                &tx,
                "lms_items",
                &item_id,
                &item,
                &[("cmid", (cmid as i64).into())],
            )?);
            if matches!(module, "assign" | "quiz") {
                let title = cm.get("name").and_then(Value::as_str).unwrap_or("");
                changed += usize::from(write_assignment(
                    &tx,
                    &AssignmentSource {
                        cmid,
                        lms_course: structure.course_id,
                        course: &linked_course,
                        title,
                        url,
                    },
                    due_of(cmid),
                    action_ids.contains(&cmid),
                    payload.assign_status.get(&cmid),
                )?);
                written.insert(cmid);
                live_assignments.insert(format!("lms-assignment:{cmid}"));
            }
        }
        let mut statement = tx.prepare("SELECT id FROM lms_items WHERE json_extract(value,'$.lmsCourseId')=?1 AND missing_since IS NULL").map_err(IngestError::from)?;
        let old = statement
            .query_map([structure.course_id], |r| r.get::<_, String>(0))
            .map_err(IngestError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(IngestError::from)?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE lms_items SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(IngestError::from)?;
        }
        let mut statement = tx
            .prepare("SELECT id FROM assignments WHERE lms_course_id=?1 AND missing_since IS NULL")
            .map_err(IngestError::from)?;
        let old = statement
            .query_map([structure.course_id], |r| r.get::<_, String>(0))
            .map_err(IngestError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(IngestError::from)?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live_assignments.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE assignments SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(IngestError::from)?;
        }
    }
    // Open work in courses whose structure was not loaded (other quarters).
    for event in &payload.action_events {
        let Some(cmid) = id_of(event.get("instance")) else {
            continue;
        };
        let module = event
            .get("modulename")
            .and_then(Value::as_str)
            .unwrap_or("");
        let url = event.get("url").and_then(Value::as_str).unwrap_or("");
        if written.contains(&cmid)
            || !matches!(module, "assign" | "quiz")
            || (!url.is_empty() && !lms_url(url))
        {
            continue;
        }
        let Some(lms_course) = id_of(event.get("course").and_then(|course| course.get("id")))
        else {
            continue;
        };
        let Some(course) = course_ids.get(&lms_course).cloned() else {
            continue;
        };
        let title = event
            .get("activityname")
            .or_else(|| event.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        changed += usize::from(write_assignment(
            &tx,
            &AssignmentSource {
                cmid,
                lms_course,
                course: &course,
                title,
                url,
            },
            due_of(cmid),
            true,
            payload.assign_status.get(&cmid),
        )?);
        written.insert(cmid);
    }
    tx.execute(
        "UPDATE import_runs SET state='complete',finished_at=?2,message=?3 WHERE id=?1",
        params![
            run_id,
            Utc::now().to_rfc3339(),
            format!("{changed} rows changed")
        ],
    )
    .map_err(IngestError::from)?;
    tx.commit().map_err(IngestError::from)?;
    Ok(changed)
}

#[cfg(test)]
#[path = "tests/lms_ingest.rs"]
mod tests;
