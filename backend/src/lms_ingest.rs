use crate::data::versioned::content_hash;
use chrono::{Datelike, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use url::Url;
use uuid::Uuid;

const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_COURSES: usize = 500;
const MAX_STRUCTURES: usize = 500;
const MAX_ITEMS: usize = 5_000;
const MAX_EVENTS: usize = 5_000;

/// Read-only Moodle transport. Every request passes through `call`, whose
/// closed allowlist is also checked before the request is constructed.
pub const LMS_EXTRACT_SCRIPT: &str = concat!(r#"(async () => {
  const enrichCourseContent = "#, include_str!("lms_course_content.js"), r#";
  const forumContent = "#, include_str!("lms_forum_content.js"), r#";
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
    const actionEvents = [];
    let aftereventid;
    for (let page = 0; page < 100; page += 1) {
      const args = {timesortfrom:0, limitnum:50};
      if (aftereventid !== undefined) args.aftereventid = aftereventid;
      const data = await call('core_calendar_get_action_events_by_timesort', args);
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
    const upcoming = await call('core_calendar_get_calendar_upcoming_view',
      {courseid:1, categoryid:0});
    const datedEvents = (Array.isArray(upcoming?.events) ? upcoming.events.slice(0, 5000) : []).map(event => ({
      ...pick(event, ['id','instance','modulename','name','timesort','eventtype','url']), course:{id:event.course?.id}}));
    const now = new Date();
    const academicYear = now.getMonth() < 2 ? now.getFullYear() - 1 : now.getFullYear();
    const category = now.getMonth() < 2 ? 4 : now.getMonth() < 5 ? 1
      : now.getMonth() < 8 ? 2 : now.getMonth() < 11 ? 3 : 4;
    const wanted = new Set([category, category === 4 ? 1 : category + 1]);
    // The category name is the course's quarter ("1"…"4").
    const structures = [];
    let contentBudget = 3000000;
    const boundedBody = text => { const body = text.slice(0, Math.min(100000, contentBudget)); contentBudget -= body.length; return body; };
    for (const course of courses) {
      const year = Number(String(course.shortname || '').match(/\[(\d{4})\]\s*$/)?.[1]);
      const quarter = Number(course.coursecategory);
      const nextYear = category === 4 ? academicYear + 1 : academicYear;
      const shouldLoad = (year === academicYear && wanted.has(quarter))
        || (category === 4 && year === nextYear && quarter === 1);
      if (!shouldLoad) continue;
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
      }
    }
    // Bounded read-only module details, using the same signed-in session.
    const detailCms = structures.flatMap(item => item.state?.cm || [])
      .filter(cm => cm.uservisible !== false && ['resource','folder','url','page','forum','assign'].includes(cm.module)).slice(0, 60);
    let forumBudget = 16;
    const readDetail = async cm => {
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
          }).filter(Boolean))].slice(0, Math.min(8, forumBudget));
          forumBudget -= discussions.length;
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
        if (cm.module === 'url') {
          const link = page.querySelector('#region-main a[href^="https://"]');
          if (link) cm.resourceUrl = link.href;
        }
      } catch { /* Keep cached detail on a later incomplete read. */ }
    };
    for (let index = 0; index < detailCms.length; index += 4)
      await Promise.all(detailCms.slice(index, index + 4).map(readDetail));
    // Submission state as the assignment page shows it (a plain GET, read
    // through the classes Moodle puts on the status table), for the
    // assignments of the loaded quarters.
    const assignIds = structures.flatMap(item => (item.state?.cm || [])
      .filter(cm => cm.module === 'assign').map(cm => Number(cm.id))).filter(id => id > 0).slice(0, 60);
    const assignStatus = [];
    const readStatus = async cmid => {
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
      } catch {}
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
})()"#);

/// Moodle sends ids as numbers in some functions and numeric strings in
/// others (`core_courseformat_get_state`).
fn id_of(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn category_number<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(number) => number.as_u64().unwrap_or(0),
        Value::String(text) => text.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsCourse {
    pub id: u64,
    #[serde(default)]
    pub fullname: String,
    #[serde(default)]
    pub shortname: String,
    #[serde(default)]
    pub idnumber: String,
    /// The category *name*, which Science Tokyo LMS sets to the quarter
    /// (`"1"`…`"4"`; `"他"` for others). Moodle sends it as a string.
    #[serde(default, deserialize_with = "category_number")]
    pub coursecategory: u64,
    #[serde(default)]
    pub startdate: i64,
    #[serde(default)]
    pub enddate: i64,
    #[serde(default)]
    pub viewurl: String,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsStructure {
    pub course_id: u64,
    pub complete: bool,
    pub state: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LmsPayload {
    pub status: String,
    pub source_url: String,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub courses_complete: bool,
    #[serde(default)]
    pub courses: Vec<LmsCourse>,
    #[serde(default)]
    pub structures: Vec<LmsStructure>,
    #[serde(default)]
    pub action_events: Vec<Value>,
    #[serde(default)]
    pub dated_events: Vec<Value>,
    /// Submission state read from each assignment page (`cmid` → state).
    #[serde(default)]
    pub assign_status: HashMap<u64, String>,
    /// The dashboard timetable grid, aggregated per course.
    #[serde(default)]
    pub timetable: Option<LmsTimetable>,
    /// Records dropped by validation; the rest of the payload is still used.
    #[serde(default, skip_deserializing)]
    pub skipped: usize,
}

/// The payload as sent, before each record is checked on its own.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLmsPayload {
    status: String,
    source_url: String,
    #[serde(default)]
    language: String,
    #[serde(default)]
    courses_complete: bool,
    #[serde(default)]
    courses: Vec<Value>,
    #[serde(default)]
    structures: Vec<Value>,
    #[serde(default)]
    action_events: Vec<Value>,
    #[serde(default)]
    dated_events: Vec<Value>,
    #[serde(default)]
    timetable: Option<Value>,
    #[serde(default)]
    assign_status: Vec<Value>,
}

/// What the dashboard timetable says for one term: weekly meetings per LMS
/// course, and the courses it lists without a slot (集中講義など).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LmsTimetable {
    pub term: String,
    pub meetings: HashMap<u64, Vec<Value>>,
    pub unscheduled: Vec<u64>,
}

/// Turns grid cells (course, day, period) into weekly meetings with
/// contiguous period ranges, e.g. 月3 + 月4 → `{day:1, periods:[3,4]}`.
pub fn aggregate_slots(cells: &[(u64, u8, u8)]) -> HashMap<u64, Vec<Value>> {
    let mut by_course: HashMap<u64, std::collections::BTreeMap<u8, Vec<u8>>> = HashMap::new();
    for &(course, day, period) in cells {
        by_course.entry(course).or_default().entry(day).or_default().push(period);
    }
    by_course
        .into_iter()
        .map(|(course, days)| {
            let mut meetings = Vec::new();
            for (day, mut periods) in days {
                periods.sort_unstable();
                periods.dedup();
                let mut start = periods[0];
                let mut end = start;
                for &period in &periods[1..] {
                    if period == end + 1 {
                        end = period;
                    } else {
                        meetings.push(json!({"day": day, "periods": [start, end]}));
                        start = period;
                        end = period;
                    }
                }
                meetings.push(json!({"day": day, "periods": [start, end]}));
            }
            (course, meetings)
        })
        .collect()
}

/// Validates the grid read from the page; anything off drops the whole grid
/// (it is a cross-check, never required).
fn parse_timetable(raw: &Value, courses: &HashSet<u64>) -> Option<LmsTimetable> {
    let year = raw.get("year")?.as_u64().filter(|year| (2000..2100).contains(year))?;
    let quarter = raw.get("quarter")?.as_str()?.trim().parse::<u8>().ok().filter(|q| (1..=4).contains(q))?;
    let slots = raw.get("slots")?.as_array()?;
    if slots.len() > 500 {
        return None;
    }
    let mut cells = Vec::new();
    for slot in slots {
        let course = id_of(slot.get("courseId"))?;
        let day = slot.get("day")?.as_u64().filter(|day| (1..=7).contains(day))? as u8;
        let period = slot.get("period")?.as_u64().filter(|p| (1..=12).contains(p))? as u8;
        if courses.contains(&course) {
            cells.push((course, day, period));
        }
    }
    let unscheduled = raw
        .get("unscheduled")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|id| id_of(Some(id))).filter(|id| courses.contains(id)).take(200).collect())
        .unwrap_or_default();
    Some(LmsTimetable {
        term: format!("{year}-{quarter}Q"),
        meetings: aggregate_slots(&cells),
        unscheduled,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LmsStage {
    Ready,
    ReconnectRequired,
}

fn lms_url(raw: &str) -> bool {
    Url::parse(raw).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("lms.s.isct.ac.jp")
            && url.username().is_empty()
            && url.password().is_none()
    })
}

/// Moodle sends names HTML-escaped (`A &amp; B`). Decodes the named
/// entities it uses and numeric references; anything else stays as written.
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let entity = &rest[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            }?;
            Some((character, end))
        });
        match decoded {
            Some((character, end)) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Decodes the named string fields of a JSON record in place.
fn decode_fields(value: &mut Value, keys: &[&str]) {
    for key in keys {
        if let Some(Value::String(text)) = value.get_mut(*key) {
            *text = decode_entities(text);
        }
    }
}

fn valid_cm(cm: &Value) -> bool {
    id_of(cm.get("id")).is_some()
        && id_of(cm.get("sectionid")).is_some()
        && cm.get("module").and_then(Value::as_str).is_some()
        && cm.get("name").and_then(Value::as_str).is_some()
        && !cm
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| !url.is_empty() && !lms_url(url))
}

fn valid_event(event: &Value) -> bool {
    id_of(event.get("instance")).is_some()
        && id_of(event.get("course").and_then(|course| course.get("id"))).is_some()
        && event.get("timesort").and_then(Value::as_i64).is_some()
}

/// Checks the envelope strictly and every record on its own: a record Moodle
/// shapes unexpectedly is dropped (and counted) instead of failing the whole
/// pull. A dropped course makes the course list incomplete, so nothing is
/// tombstoned from it.
pub fn parse_lms_payload(raw: &str) -> Result<(LmsStage, Option<LmsPayload>), String> {
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err("LMS payload exceeds the size limit".into());
    }
    let raw: RawLmsPayload =
        serde_json::from_str(raw).map_err(|_| "LMS payload is not valid JSON")?;
    if !lms_url(&raw.source_url) {
        return Err("LMS payload has an invalid source URL".into());
    }
    if raw.status == "login" {
        return Ok((LmsStage::ReconnectRequired, None));
    }
    if raw.status != "ok" {
        return Err("LMS extraction was incomplete".into());
    }
    if raw.courses.len() > MAX_COURSES
        || raw.structures.len() > MAX_STRUCTURES
        || raw.action_events.len() > MAX_EVENTS
        || raw.dated_events.len() > MAX_EVENTS
    {
        return Err("LMS payload exceeds a collection limit".into());
    }
    let mut skipped = 0;
    let mut skipped_courses = false;
    let mut course_ids = HashSet::new();
    let mut courses = Vec::new();
    for value in raw.courses {
        let course = serde_json::from_value::<LmsCourse>(value).ok().filter(|course| {
            course.id != 0
                && !course_ids.contains(&course.id)
                && !course.fullname.trim().is_empty()
                && course.fullname.len() <= 1_000
                && course.shortname.len() <= 255
                && lms_url(&course.viewurl)
        });
        match course {
            Some(mut course) => {
                course.fullname = decode_entities(&course.fullname);
                course_ids.insert(course.id);
                courses.push(course);
            }
            None => {
                skipped += 1;
                skipped_courses = true;
            }
        }
    }
    let mut item_count = 0;
    let mut structures = Vec::new();
    for value in raw.structures {
        let Ok(mut structure) = serde_json::from_value::<LmsStructure>(value) else {
            skipped += 1;
            continue;
        };
        if !course_ids.contains(&structure.course_id) {
            skipped += 1;
            continue;
        }
        if structure.complete {
            let cms = structure
                .state
                .as_ref()
                .and_then(|state| state.get("cm"))
                .and_then(Value::as_array)
                .cloned();
            let Some(cms) = cms else {
                // Without a module list the structure is unusable, but the
                // course's existing items must not be tombstoned either.
                structure.complete = false;
                structure.state = None;
                skipped += 1;
                structures.push(structure);
                continue;
            };
            let kept: Vec<Value> = cms
                .into_iter()
                .filter(valid_cm)
                .map(|mut cm| {
                    decode_fields(&mut cm, &["name"]);
                    cm
                })
                .collect();
            let total = structure.state.as_ref().and_then(|s| s.get("cm")).and_then(Value::as_array).map_or(0, Vec::len);
            if kept.len() < total {
                // A partial module list would tombstone the dropped items.
                skipped += total - kept.len();
                structure.complete = false;
            }
            item_count += kept.len();
            if let Some(state) = structure.state.as_mut().and_then(Value::as_object_mut) {
                state.insert("cm".into(), Value::Array(kept));
                if let Some(Value::Array(sections)) = state.get_mut("section") {
                    for section in sections {
                        decode_fields(section, &["title"]);
                        // Moodle serializes these IDs as strings, unlike material sectionId.
                        if let Some(id) = id_of(section.get("id")) {
                            section["id"] = json!(id);
                        }
                    }
                }
            }
        }
        structures.push(structure);
    }
    if item_count > MAX_ITEMS {
        return Err("LMS structure exceeds the item limit".into());
    }
    let mut events = |list: Vec<Value>| {
        let total = list.len();
        let kept: Vec<Value> = list
            .into_iter()
            .filter(valid_event)
            .map(|mut event| {
                decode_fields(&mut event, &["name", "activityname"]);
                event
            })
            .collect();
        skipped += total - kept.len();
        kept
    };
    let action_events = events(raw.action_events);
    let dated_events = events(raw.dated_events);
    let timetable = raw.timetable.as_ref().and_then(|grid| parse_timetable(grid, &course_ids));
    let assign_status = raw
        .assign_status
        .iter()
        .take(200)
        .filter_map(|row| {
            let state = row.get("submission")?.as_str()?;
            if !matches!(state, "open" | "submitted" | "graded") {
                return None;
            }
            Some((id_of(row.get("cmid"))?, state.to_string()))
        })
        .collect();
    Ok((
        LmsStage::Ready,
        Some(LmsPayload {
            status: raw.status,
            source_url: raw.source_url,
            language: raw.language,
            courses_complete: raw.courses_complete && !skipped_courses,
            courses,
            structures,
            action_events,
            dated_events,
            timetable,
            assign_status,
            skipped,
        }),
    ))
}

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
    let existing: Option<String> = tx
        .query_row(
            &format!("SELECT hash FROM {table} WHERE id=?1"),
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if existing.as_deref() == Some(&hash) {
        return Ok(false);
    }
    let timestamp = Utc::now().to_rfc3339();
    let mut names = vec![
        "id",
        "value",
        "hash",
        "updated_at",
        "updated_by",
        "rev",
        "source",
        "deleted_at",
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
    }
    let json = serde_json::to_string(&stored).map_err(|e| e.to_string())?;
    let mut values = vec![
        id.to_string().into(),
        json.into(),
        hash.into(),
        timestamp.into(),
        String::from("lms").into(),
        (rev as i64).into(),
        String::from("lms").into(),
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
    let prior: Option<String> = tx
        .query_row(
            "SELECT submission FROM assignments WHERE lms_cmid=?1 AND deleted_at IS NULL",
            [cmid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    // The assignment page is authoritative; the event list is only a hint.
    let submission = if let Some(state) = page_state {
        state
    } else if listed_open {
        "open"
    } else if prior.as_deref() == Some("open") && due.is_some_and(|d| d > Utc::now().timestamp()) {
        "submitted"
    } else {
        prior.as_deref().filter(|p| *p == "submitted").unwrap_or("unknown")
    };
    let due_at = due.and_then(|d| chrono::DateTime::from_timestamp(d, 0)).map(|d| d.to_rfc3339());
    let assignment = json!({"id":format!("lms-assignment:{cmid}"),"kind":"assignment","courseId":source.course,
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
                due_at.map(Into::into).unwrap_or(rusqlite::types::Value::Null),
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
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
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
        .filter(|v| v.get("eventtype").and_then(Value::as_str) == Some("due"))
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
            .query_row("SELECT value FROM lms_courses WHERE id=?1", [&lms_course_id], |row| row.get::<_, String>(0))
            .optional()
            .map_err(|e| e.to_string())?
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|stored| stored.get("timetables").cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        if let Some(grid) = &payload.timetable {
            let shown = grid.meetings.get(&course.id).cloned().or_else(|| {
                grid.unscheduled.contains(&course.id).then(Vec::new)
            });
            if let (Some(meetings), Some(map)) = (shown, timetables.as_object_mut()) {
                map.insert(grid.term.clone(), Value::Array(meetings));
            }
        }
        let prior_sections: Option<Value> = tx.query_row(
            "SELECT value FROM lms_courses WHERE id=?1", [&lms_course_id], |row| row.get::<_, String>(0)
        ).optional().map_err(|e| e.to_string())?
          .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
          .and_then(|stored| stored.get("sections").cloned());
        let mut sections = payload.structures.iter().find(|s| s.course_id == course.id && s.complete)
            .and_then(|s| s.state.as_ref()).and_then(|s| s.get("section")).cloned()
            .or_else(|| prior_sections.clone()).unwrap_or_else(|| json!([]));
        if let Some(current) = sections.as_array_mut() {
            for section in current {
                if section.get("bodyHtml").is_none() {
                    if let Some(body) = prior_sections.as_ref().and_then(Value::as_array)
                        .and_then(|old| old.iter().find(|s| id_of(s.get("id")) == id_of(section.get("id"))))
                        .and_then(|s| s.get("bodyHtml")) {
                        section.as_object_mut().unwrap().insert("bodyHtml".into(), body.clone());
                    }
                }
            }
        }
        let value = json!({"id":lms_course_id,"lmsId":course.id,"offeringId":offering,
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
        // The timetable itself changes only through the app's Pull review;
        // materials and assignments attach to a local course already linked
        // to the same syllabus offering, else to a stable LMS course id.
        let offering_id = value.get("offeringId").and_then(Value::as_str);
        let linked = offering_id.and_then(|id| {
            tx.query_row(
                "SELECT id FROM timetable_courses WHERE deleted_at IS NULL
                 AND json_extract(value,'$.offeringId')=?1 ORDER BY updated_at DESC LIMIT 1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
        });
        let academic_year = year.unwrap_or_else(|| Utc::now().year() as u32);
        let course_id = linked.unwrap_or_else(|| {
            offering_id
                .map(|id| format!("course:{academic_year}:{id}"))
                .unwrap_or_else(|| format!("course:lms:{}", course.id))
        });
        course_ids.insert(course.id, course_id);
    }
    if payload.courses_complete {
        let live: HashSet<String> = payload
            .courses
            .iter()
            .map(|c| format!("lms-course:{}", c.id))
            .collect();
        let mut statement = tx
            .prepare("SELECT id FROM lms_courses WHERE deleted_at IS NULL")
            .map_err(|e| e.to_string())?;
        let old = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let deleted = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE lms_courses SET deleted_at=?2,rev=rev+1,value=json_set(value,'$.deletedAt',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND deleted_at IS NULL",params![id,deleted]).map_err(|e|e.to_string())?;
        }
    }
    for structure in payload.structures.iter().filter(|s| s.complete) {
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
                .unwrap_or_else(|| format!("course:lms:{}", structure.course_id));
            let name = cm.get("name").and_then(Value::as_str).unwrap_or("");
            let previous: Option<Value> = tx.query_row("SELECT value FROM lms_items WHERE id=?1", [&item_id], |r| r.get::<_, String>(0))
                .optional().map_err(|e| e.to_string())?.and_then(|raw| serde_json::from_str(&raw).ok());
            let body = cm.get("bodyHtml").or_else(|| previous.as_ref().and_then(|v| v.get("bodyHtml")));
            let resource_url = cm.get("resourceUrl").or_else(|| previous.as_ref().and_then(|v| v.get("resourceUrl")));
            let item = json!({"id":item_id,"kind":"material","courseId":linked_course,"cmid":cmid,"lmsCourseId":structure.course_id,"sectionNumber":section.1,
                "sectionId":id_of(cm.get("sectionid")),"bodyHtml":body,"resourceUrl":resource_url,"sectionTitle":section.0,"module":module,"name":cm.get("name").and_then(Value::as_str).unwrap_or(""),
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
                    &AssignmentSource { cmid, lms_course: structure.course_id, course: &linked_course, title, url },
                    due_of(cmid),
                    action_ids.contains(&cmid),
                    payload.assign_status.get(&cmid).map(String::as_str),
                )?);
                written.insert(cmid);
            }
        }
        let mut statement = tx.prepare("SELECT id FROM lms_items WHERE json_extract(value,'$.lmsCourseId')=?1 AND deleted_at IS NULL").map_err(|e|e.to_string())?;
        let old = statement
            .query_map([structure.course_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(statement);
        for id in old.into_iter().filter(|id| !live.contains(id)) {
            let deleted = Utc::now().to_rfc3339();
            changed += tx.execute("UPDATE lms_items SET deleted_at=?2,rev=rev+1,value=json_set(value,'$.deletedAt',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1 AND deleted_at IS NULL",params![id,deleted]).map_err(|e|e.to_string())?;
        }
    }
    // Open work in courses whose structure was not loaded (other quarters).
    for event in &payload.action_events {
        let Some(cmid) = id_of(event.get("instance")) else { continue };
        let module = event.get("modulename").and_then(Value::as_str).unwrap_or("");
        let url = event.get("url").and_then(Value::as_str).unwrap_or("");
        if written.contains(&cmid) || !matches!(module, "assign" | "quiz") || (!url.is_empty() && !lms_url(url)) {
            continue;
        }
        let Some(lms_course) = id_of(event.get("course").and_then(|course| course.get("id"))) else { continue };
        let course = course_ids
            .get(&lms_course)
            .cloned()
            .unwrap_or_else(|| format!("course:lms:{lms_course}"));
        let title = event
            .get("activityname")
            .or_else(|| event.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        changed += usize::from(write_assignment(
            &tx,
            &AssignmentSource { cmid, lms_course, course: &course, title, url },
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
