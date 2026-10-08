use crate::data::{resolve_course_id, versioned::content_hash};
use crate::school_parser::lms::{id_of, lms_url, LmsPayload};
use chrono::{Datelike, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

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
  // The dashboard timetable block (rendered by the GET of /my/; its filter
  // form is a POST and is never submitted). Rows are periods (first cell),
  // columns are weekdays (header row); each cell links the course.
  const readTimetable = () => {
    const block = document.querySelector('[data-block="tttimetable"]');
    const [grid, other] = block ? block.querySelectorAll('table') : [];
    if (!grid || !grid.rows.length) return null;
    const selected = name => block.querySelector('select[name="' + name + '"]')?.selectedOptions[0]?.textContent.trim() || '';
    const dayNames = ['月','火','水','木','金','土','日'];
    const header = [...grid.rows[0].cells].map(cell => cell.textContent.trim());
    const courseId = link => Number(new URL(link.href, location.href).searchParams.get('id'));
    const slots = [];
    for (const row of [...grid.rows].slice(1)) {
      const period = Number(row.cells[0]?.textContent.trim());
      if (!Number.isInteger(period) || period < 1 || period > 12) continue;
      [...row.cells].slice(1).forEach((cell, index) => {
        const named = dayNames.indexOf(header[index + 1]);
        const day = named >= 0 ? named + 1 : index + 1;
        for (const link of cell.querySelectorAll('a[href*="/course/view.php?id="]')) {
          const id = courseId(link);
          if (id > 0) slots.push({courseId:id, day, period});
        }
      });
    }
    const unscheduled = other ? [...other.querySelectorAll('a[href*="/course/view.php?id="]')].map(courseId).filter(id => id > 0) : [];
    return {year:Number(selected('year')) || null, quarter:selected('quota'), slots, unscheduled};
  };
  try {
    const timetable = readTimetable();
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
    const now = new Date();
    const academicYear = now.getMonth() < 2 ? now.getFullYear() - 1 : now.getFullYear();
    const category = now.getMonth() < 2 ? 4 : now.getMonth() < 5 ? 1
      : now.getMonth() < 8 ? 2 : now.getMonth() < 11 ? 3 : 4;
    const wanted = new Set([category, category === 4 ? 1 : category + 1]);
    // The category name is the course's quarter ("1"…"4").
    const structures = [];
    let contentBudget = 3000000;
    const boundedBody = text => { const body = text.slice(0, Math.min(100000, contentBudget)); contentBudget -= body.length; return body; };
    const courseContext = course => {
      const year = Number(String(course.shortname || '').match(/\[(\d{4})\]\s*$/)?.[1]);
      const quarter = Number(course.coursecategory);
      return {year, quarter, term:Number.isInteger(year) && quarter >= 1 && quarter <= 4 ? year + '-' + quarter + 'Q' : ''};
    };
    const selectedCourses = courses.filter(course => {
      const {year, quarter} = courseContext(course);
      const nextYear = category === 4 ? academicYear + 1 : academicYear;
      return requestedCourseId
        ? Number(course.id) === requestedCourseId
        : fullScan && ((year === academicYear && wanted.has(quarter))
          || (category === 4 && year === nextYear && quarter === 1));
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
          const response = await fetch('/2025/course/view.php?id=' + Number(course.id), {credentials:'same-origin'});
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
    for (let index = 0; index < selectedCourses.length; index += 4)
      await Promise.all(selectedCourses.slice(index, index + 4).map(loadStructure));
    // Bounded read-only module details, using the same signed-in session.
    const courseById = new Map(courses.map(course => [Number(course.id), course]));
    // Details are written onto the returned state's own module objects. Each
    // course gets its own share, newest modules first, so a large or
    // fast-loading course cannot starve the rest; cached details fill gaps.
    const detailCms = structures.flatMap(item => (item.state?.cm || [])
      .filter(cm => cm.uservisible !== false && ['resource','folder','url','page','forum','assign'].includes(cm.module))
      .reverse().slice(0, 40).map(cm => ({cm, courseId:item.courseId}))).slice(0, 320);
    const forumBudget = new Map();
    const assignStatus = [];
    const readDetail = async (cm, courseId) => {
      try {
        const target = new URL(cm.url, sourceUrl);
        if (target.origin !== location.origin || !/^\/2025\/mod\/(resource|folder|url|page|forum|assign)\/view\.php$/.test(target.pathname)) return;
        const response = await fetch(target.href, {credentials:'same-origin'});
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
          const table = page.querySelector('.submissionstatustable');
          if (table) {
            const has = name => Boolean(table.querySelector('td.' + name));
            const submission = has('submissiongraded') ? 'graded'
              : has('submissionstatussubmitted') ? 'submitted' : 'open';
            assignStatus.push({cmid:Number(cm.id), submission,
              late:has('latesubmission') || has('overdue')});
          }
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
              const detail = await fetch(url, {credentials:'same-origin'});
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
    for (let index = 0; index < detailCms.length; index += 4)
      await Promise.all(detailCms.slice(index, index + 4).map(readDetailWithProgress));
    // Submission state as the assignment page shows it (a plain GET, read
    // through the classes Moodle puts on the status table), for the
    // assignments of the loaded quarters.
    const statusRead = new Set(assignStatus.map(item => item.cmid));
    const assignIds = structures.flatMap(item => (item.state?.cm || [])
      .filter(cm => cm.module === 'assign').map(cm => ({id:Number(cm.id), courseId:item.courseId, name:String(cm.name || '')})))
      .filter(item => item.id > 0 && !statusRead.has(item.id)).slice(0, 60);
    let statusesDone = 0;
    const readStatus = async item => {
      const cmid = item.id;
      try {
        const response = await fetch('/2025/mod/assign/view.php?id=' + cmid, {credentials:'same-origin'});
        if (!response.ok) return;
        const page = new DOMParser().parseFromString(await response.text(), 'text/html');
        if (!page.body.classList.contains('cmid-' + cmid)) return;
        const table = page.querySelector('.submissionstatustable');
        if (!table) return;
        const has = name => Boolean(table.querySelector('td.' + name));
        const submission = has('submissiongraded') ? 'graded'
          : has('submissionstatussubmitted') ? 'submitted' : 'open';
        assignStatus.push({cmid, submission, late:has('latesubmission') || has('overdue')});
      } catch {} finally {
        statusesDone += 1;
        const course = courseById.get(Number(item.courseId));
        reportProgress({phase:'assignment', completed:statusesDone, total:assignIds.length,
          courseId:Number(item.courseId) || 0, courseTitle:String(course?.fullname || course?.shortname || ''),
          term:course ? courseContext(course).term : '', itemTitle:item.name});
      }
    };
    for (let index = 0; index < assignIds.length; index += 4)
      await Promise.all(assignIds.slice(index, index + 4).map(readStatus));
    return {status:'ok', sourceUrl, language:String(cfg.language || ''), coursesComplete,
      courses, structures, actionEvents, datedEvents, timetable, assignStatus};
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
) -> Result<bool, String> {
    let hash = content_hash(value)?;
    let existing: Option<(String, Option<String>)> = tx
        .query_row(
            &format!("SELECT hash, missing_since FROM {table} WHERE id=?1"),
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let timestamp = Utc::now().to_rfc3339();
    if existing.as_ref().is_some_and(|(stored, _)| stored == &hash) {
        let changed = existing
            .as_ref()
            .is_some_and(|(_, missing)| missing.is_some());
        tx.execute(
            &format!("UPDATE {table} SET last_seen_at=?2, missing_since=NULL, deleted_at=NULL,
                value=json_remove(json_set(value,'$.lastSeenAt',?2),'$.missingSince','$.deletedAt') WHERE id=?1"),
            params![id, timestamp],
        ).map_err(|e| e.to_string())?;
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
        .map_err(|e| e.to_string())?
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
    let json = serde_json::to_string(&stored).map_err(|e| e.to_string())?;
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
    .map_err(|e| e.to_string())?;
    Ok(true)
}

struct AssignmentSource<'a> {
    cmid: u64,
    lms_course: u64,
    course: &'a str,
    title: &'a str,
    url: &'a str,
}

/// One assignment row. `open` while Moodle lists it as work to do; a
/// previously open one that left that list before its deadline is submitted.
fn write_assignment(
    tx: &Transaction<'_>,
    source: &AssignmentSource<'_>,
    due: Option<i64>,
    listed_open: bool,
    page_state: Option<&str>,
) -> Result<bool, String> {
    let cmid = source.cmid;
    let prior: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT submission, due_at FROM assignments WHERE lms_cmid=?1 AND missing_since IS NULL",
            [cmid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let effective_due = due.or_else(|| {
        prior
            .as_ref()
            .and_then(|value| value.1.as_deref())
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.timestamp())
    });
    // The assignment page is authoritative; the event list is only a hint.
    // Keep the last known due date when Moodle temporarily omits its event.
    let submission = if let Some(state) = page_state {
        state
    } else if listed_open {
        "open"
    } else if prior.as_ref().map(|value| value.0.as_str()) == Some("open")
        && effective_due.is_some_and(|d| d > Utc::now().timestamp())
    {
        "submitted"
    } else {
        prior
            .as_ref()
            .map(|value| value.0.as_str())
            .filter(|p| *p == "submitted")
            .unwrap_or("unknown")
    };
    let due_at = due
        .and_then(|d| chrono::DateTime::from_timestamp(d, 0))
        .map(|d| d.to_rfc3339())
        .or_else(|| prior.as_ref().and_then(|value| value.1.clone()));
    let assignment = json!({"id":format!("lms-assignment:{cmid}"),"sourceKey":format!("lms:default:assignment:{cmid}"),"kind":"assignment","courseId":source.course,
        "lmsCmid":cmid,"lmsCourseId":source.lms_course,"title":source.title,"details":"","sourceUrl":source.url,
        "dueAt":due_at,"submission":submission});
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
) -> Result<usize, String> {
    let tx = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let mut changed = 0;
    let run_id = Uuid::new_v4().to_string();
    let started = Utc::now().to_rfc3339();
    tx.execute("INSERT INTO import_runs(id,provider,scope,state,started_at) VALUES(?1,'lms','full','running',?2)", params![run_id, started]).map_err(|e| e.to_string())?;
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
            .map_err(|e| e.to_string())?
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|stored| stored.get("timetables").cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if let Some(grid) = &payload.timetable {
            let shown = grid
                .meetings
                .get(&course.id)
                .cloned()
                .or_else(|| grid.unscheduled.contains(&course.id).then(Vec::new));
            if let (Some(meetings), Some(map)) = (shown, timetables.as_object_mut()) {
                map.insert(grid.term.clone(), Value::Array(meetings));
            }
        }
        let prior_sections: Option<Value> = tx
            .query_row(
                "SELECT value FROM lms_courses WHERE id=?1",
                [&lms_course_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
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
                        section
                            .as_object_mut()
                            .unwrap()
                            .insert("bodyHtml".into(), body.clone());
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
            value["id"].as_str().unwrap(),
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
            .map_err(|e| e.to_string())?;
        let old = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE lms_courses SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(|e|e.to_string())?;
        }
    }
    for structure in payload.structures.iter().filter(|s| s.complete) {
        let mut live_assignments = HashSet::new();
        let state = structure
            .state
            .as_ref()
            .expect("validated complete structure");
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
                .map_err(|e| e.to_string())?
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
                item["id"].as_str().unwrap(),
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
                    payload.assign_status.get(&cmid).map(String::as_str),
                )?);
                written.insert(cmid);
                live_assignments.insert(format!("lms-assignment:{cmid}"));
            }
        }
        let mut statement = tx.prepare("SELECT id FROM lms_items WHERE json_extract(value,'$.lmsCourseId')=?1 AND missing_since IS NULL").map_err(|e|e.to_string())?;
        let old = statement
            .query_map([structure.course_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE lms_items SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(|e|e.to_string())?;
        }
        let mut statement = tx
            .prepare("SELECT id FROM assignments WHERE lms_course_id=?1 AND missing_since IS NULL")
            .map_err(|e| e.to_string())?;
        let old = statement
            .query_map([structure.course_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live_assignments.contains(id)) {
            let missing = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE assignments SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND missing_since IS NULL",params![id,missing]).map_err(|e|e.to_string())?;
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
            payload.assign_status.get(&cmid).map(String::as_str),
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
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::LMS_EXTRACT_SCRIPT;

    #[test]
    fn extractor_routes_work_by_loaded_page_and_explicit_scope() {
        assert!(LMS_EXTRACT_SCRIPT
            .contains("const fullScan = globalThis.__ISCT_COMPANION_LMS_FULL__ === true"));
        assert!(LMS_EXTRACT_SCRIPT.contains(
            "const requestedCourseId = explicitCourseId || (!fullScan ? pageCourseId : 0)"
        ));
        assert!(LMS_EXTRACT_SCRIPT.contains("'core_calendar_get_action_events_by_course'"));
        assert!(LMS_EXTRACT_SCRIPT.contains("fullScan && ((year === academicYear"));
    }
}
