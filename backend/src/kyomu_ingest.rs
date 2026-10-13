use crate::data::{resolve_course_id, versioned::content_hash};
use crate::school_parser::kyomu::KyomuPayload;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// App-owned, read-only reader for the four approved 教務 pages. It only
/// inspects the current document and returns plain data through the relay.
pub const KYOMU_EXTRACT_SCRIPT: &str = r#"(() => {
  const clean = value => String(value || '').normalize('NFKC').replace(/\s+/g, ' ').trim();
  const text = (root, selector) => clean(root.querySelector(selector)?.textContent);
  const cells = row => [...row.querySelectorAll(':scope > th, :scope > td')].map(cell => clean(cell.textContent));
  const sourceUrl = location.href;
  const allowed = new Map([
    ['/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx','registrations'],
    ['/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_1.aspx','declared'],
    ['/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx','grades'],
    ['/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx','requirements'],
    ['/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx','gpa']
  ]);
  const kind = allowed.get(location.pathname);
  const login = location.hostname !== 'kyomu0.gakumu.titech.ac.jp' || !kind
    || Boolean(document.querySelector('input[type=password], form[action*="login" i]'));
  if (login) return {status:'login', sourceUrl, kind:kind || 'unknown', data:{}};
  const requirement = raw => {
    const match = clean(raw).match(/^([A-Z])\s*\((◎|○|選択)\)$/);
    return match ? {category:match[1], mark:match[2] === '◎' ? 'required'
      : match[2] === '○' ? 'recommended' : 'elective'} : {raw:clean(raw)};
  };
  // One meeting per <br> line: `3Q：火3-4`, `3～4Q：金7-8` (once per quarter).
  const lines = el => {
    if (!el) return [];
    const copy = el.cloneNode(true);
    copy.querySelectorAll('br').forEach(br => br.replaceWith('\n'));
    return copy.textContent.split('\n').map(clean).filter(Boolean);
  };
  const schedule = rows => rows.flatMap(part => {
    const match = part.match(/^([1-4])(?:\s*[~〜-]\s*([1-4]))?Q[:：]\s*([月火水木金土日])\s*(\d+)\s*[-–〜~]\s*(\d+)/);
    if (!match) return [{raw:part}];
    const from = Number(match[1]), to = Number(match[2] || match[1]);
    const first = Number(match[4]), last = Number(match[5]);
    const periods = Array.from({length:last-first+1},(_,i)=>first+i);
    return Array.from({length:to-from+1},(_,i)=>({quarter:(from+i)+'Q', day:match[3], periods}));
  });
  if (kind === 'registrations') {
    const rawState = text(document,'#ctl00_ContentPlaceHolder1_ctl00_lblStatus2');
    const stateMap = {'一時保存':'saved','要エラーチェック':'needs-check','本登録可能':'registrable',
      '承認待ち':'needs-approval','要承認申請':'needs-approval','承認待ち / 要承認申請':'needs-approval',
      '承認待ち/要承認申請':'needs-approval','本登録不可':'rejected',
      '本登録':'registered','本登録済':'registered'};
    const seen = new Set(), courses = [];
    for (const block of document.querySelectorAll('table#timetable [data-jwc]')) {
      const offeringId = clean(block.getAttribute('data-jwc'));
      if (!/^\d{9}$/.test(offeringId) || seen.has(offeringId)) continue;
      seen.add(offeringId);
      const classes = [...block.classList, ...(block.closest('td')?.classList || [])];
      const courseState = classes.includes('timetableError') ? 'rejected'
        : classes.some(c => c === 'timetableAlert1' || c === 'timetableAlert2' || c === 'timetableAlert3' || c === 'timetableAlert22') ? 'needs-approval'
        : classes.includes('timetableUnchecked') ? 'needs-check' : 'registrable';
      // .ttYoubi2 is .ttYoubi without the rooms.
      const times = lines(block.querySelector('.ttYoubi2') || block.querySelector('.ttYoubi'));
      courses.push({offeringId,title:text(block,'.ttKamokuName a'),code:text(block,'.ttSinkokuNo'),
        instructor:text(block,'.ttKyoin'),schedule:schedule(times),requirement:requirement(text(block,'.ttSuisho')),
        courseState,declaredOnly:false});
    }
    for (const table of document.querySelectorAll('table[id^="kamokuUnit_"][id$="_gvKamoku"]')) {
      const headers = [...table.querySelectorAll('tr:first-child th')].map(th => clean(th.textContent));
      for (const row of [...table.querySelectorAll('tr')].slice(1)) {
        if (clean(row.textContent).includes('対象データはありません')) continue;
        const link = row.querySelector('a.kamokuGridKamokuName');
        const offeringId = new URL(link?.href || location.href).searchParams.get('jwc') || '';
        if (!/^\d{9}$/.test(offeringId) || seen.has(offeringId)) continue;
        seen.add(offeringId);
        const values = cells(row), at = label => values[headers.findIndex(h => h.includes(label))] || '';
        const creditRaw = text(row,'span.resultUnitKamokuTaniSP');
        const credit = creditRaw.match(/\[\s*([\d.]+)-([\d.]+)-([\d.]+)\s*\]/);
        const rawSchedule = text(row,'td.tdYoubi');
        const times = lines(row.querySelector('td.tdYoubi'));
        courses.push({offeringId,title:clean(link?.textContent),code:text(row,'span.resultUnitKamokuCodeSP'),
          instructor:at('授業担当教員'),schedule:schedule(times),declaredOnly:rawSchedule === '申告専用',
          requirement:requirement(at('備考')),credits:credit ? {lecture:Number(credit[1]),exercise:Number(credit[2]),lab:Number(credit[3])}:null});
      }
    }
    const savedRaw = text(document,'#ctl00_ContentPlaceHolder1_ctl00_lblStatus3');
    const savedMatch = savedRaw.match(/(\d{4})\/(\d{2})\/(\d{2})\s+(\d{2}):(\d{2}):(\d{2})/);
    const savedAt = savedMatch ? `${savedMatch[1]}-${savedMatch[2]}-${savedMatch[3]}T${savedMatch[4]}:${savedMatch[5]}:${savedMatch[6]}+09:00` : null;
    return {status:document.querySelector('#divSinkokuStatus')?'ok':'login',sourceUrl,kind,
      data:{state:{state:stateMap[rawState]||'unknown',raw:rawState,savedAt},courses}};
  }
  if (kind === 'declared') {
    // 科目から: every declared course with its quarter and credit split. Each
    // category table states its own total (#…_{unit}_spanSum); that total is
    // authoritative (e.g. two 0-0-1 PE rows that total 1.0).
    const courses = [];
    for (const table of document.querySelectorAll('table[id^="kamokuUnit_"][id$="_gvKamoku"]')) {
      const unit = table.id.replace(/_gvKamoku$/, '');
      const sumText = text(document, '#ctl00_ContentPlaceHolder1_' + unit + '_spanSum');
      const sum = Number((sumText.match(/([\d.]+)/) || [])[1]);
      for (const row of table.querySelectorAll('tr')) {
        const link = row.querySelector('a.kamokuGridKamokuName');
        if (!link) continue;
        const offeringId = new URL(link.href, location.href).searchParams.get('jwc') || '';
        if (!/^\d{9}$/.test(offeringId)) continue;
        const range = text(row, 'td.tdQ').match(/^([1-4])(?:\s*-\s*([1-4]))?$/);
        const credit = text(row, 'span.resultUnitKamokuTaniSP').match(/\[\s*([\d.]+)-([\d.]+)-([\d.]+)\s*\]/);
        courses.push({offeringId, unit, unitTotal:Number.isFinite(sum) ? sum : null,
          code:text(row, 'span.resultUnitKamokuCodeSP'), title:clean(link.textContent),
          quarters:range ? Array.from({length:Number(range[2] || range[1]) - Number(range[1]) + 1}, (_, i) => Number(range[1]) + i) : [],
          credits:credit ? {lecture:Number(credit[1]), exercise:Number(credit[2]), lab:Number(credit[3])} : null});
      }
    }
    return {status:document.querySelector('table[id^="kamokuUnit_"]') ? 'ok' : 'login', sourceUrl, kind, data:{courses}};
  }
  if (kind === 'grades') {
    const groups = [], nodes = document.querySelectorAll('h3[id$="_h3"], table[id$="_grid"]');
    let category = '';
    for (const node of nodes) {
      if (node.matches('h3')) { category = clean(node.textContent); continue; }
      const rows = [];
      for (const row of node.querySelectorAll('tr')) {
        const title = text(row,'td.resultUnitKamokuName');
        if (!title) continue;
        rows.push({title,code:text(row,'td.resultUnitSuisho + td.mediaPC') || text(row,'span.resultUnitKamokuCodeSP'),
          recommended:text(row,'td.resultUnitSuisho'),credits:text(row,'td.resultUnitTani'),result:text(row,'td.resultUnitSeiseki'),
          quarter:[...row.querySelectorAll('[class*="tdQuarter"]')].map(x=>clean(x.textContent)).join(' '),completed:text(row,'td.resultUnitYG span.mediaPC')});
      }
      if (category && rows.length) groups.push({category,rows});
    }
    return {status:'ok',sourceUrl,kind,data:{groups}};
  }
  if (kind === 'requirements') {
    const tables = [...document.querySelectorAll('div.tableSet02Tbl table.tblYouken')].map(table => {
      const rows = [...table.querySelectorAll('tr')].map(cells).filter(row => row.length);
      return {headers:rows[0] || [],rows:rows.slice(1).map(row=>({label:row[0]||'',values:row.slice(1)}))};
    });
    return {status:tables.length===2?'ok':'error',sourceUrl,kind,data:{tables}};
  }
  const categories = [...document.querySelectorAll('h3[id$="_h3"]')].map(h => {
    const match = h.id.match(/resultUnit_(\d+)_h3$/); return match ? {code:match[1],name:clean(h.textContent)} : null;
  }).filter(Boolean);
  const overall = [...document.querySelectorAll('table.tblGPASummary.mediaPC')].map(table=>[...table.querySelectorAll('tr')].map(cells));
  const terms = [...document.querySelectorAll('table.tblQ tr')].map(cells).filter(row=>row.length);
  return {status:'ok',sourceUrl,kind,data:{categories,overall,terms}};
})()"#;

fn upsert(
    tx: &Transaction<'_>,
    table: &str,
    id: &str,
    value: &Value,
    extra: &[(&str, rusqlite::types::Value)],
) -> Result<bool, String> {
    let hash = content_hash(value)?;
    let old: Option<(String, u64, Option<String>)> = tx
        .query_row(
            &format!("SELECT hash,rev,missing_since FROM {table} WHERE id=?1"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let timestamp = Utc::now().to_rfc3339();
    if old.as_ref().is_some_and(|(h, _, _)| h == &hash) {
        let changed = old
            .as_ref()
            .is_some_and(|(_, _, missing)| missing.is_some());
        tx.execute(&format!("UPDATE {table} SET last_seen_at=?2,missing_since=NULL,deleted_at=NULL,
            value=json_remove(json_set(value,'$.lastSeenAt',?2),'$.missingSince','$.deletedAt') WHERE id=?1"), params![id,timestamp])
            .map_err(|e|e.to_string())?;
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
    names.extend(extra.iter().map(|(n, _)| *n));
    let rev = old.map_or(0, |x| x.1) + 1;
    let mut stored = value.clone();
    if let Some(object) = stored.as_object_mut() {
        object.insert("rev".into(), rev.into());
        object.insert("updatedAt".into(), timestamp.clone().into());
        object.insert("updatedBy".into(), "kyomu".into());
        object.insert("source".into(), json!({"provider":"kyomu"}));
        object.insert(
            "sourceKey".into(),
            format!("kyomu:default:{table}:{id}").into(),
        );
        object.insert("lastSeenAt".into(), timestamp.clone().into());
        object.remove("missingSince");
        object.remove("deletedAt");
    }
    let mut values = vec![
        id.to_string().into(),
        serde_json::to_string(&stored)
            .map_err(|e| e.to_string())?
            .into(),
        hash.into(),
        timestamp.clone().into(),
        String::from("kyomu").into(),
        (rev as i64).into(),
        String::from("kyomu").into(),
        rusqlite::types::Value::Null,
        timestamp.clone().into(),
        rusqlite::types::Value::Null,
    ];
    values.extend(extra.iter().map(|(_, v)| v.clone()));
    let marks = (1..=names.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let updates = names
        .iter()
        .skip(1)
        .map(|n| format!("{n}=excluded.{n}"))
        .collect::<Vec<_>>()
        .join(",");
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

fn mark_missing(
    tx: &Transaction<'_>,
    table: &str,
    prefix: &str,
    live: &HashSet<String>,
) -> Result<usize, String> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT id FROM {table} WHERE id LIKE ?1 AND missing_since IS NULL"
        ))
        .map_err(|e| e.to_string())?;
    let pattern = format!("{prefix}%");
    let stale = statement
        .query_map([pattern], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    let missing = Utc::now().to_rfc3339();
    let mut changed = 0;
    for id in stale.into_iter().filter(|id| !live.contains(id)) {
        changed += tx.execute(
            &format!("UPDATE {table} SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1"),
            params![id, missing],
        ).map_err(|e| e.to_string())?;
    }
    Ok(changed)
}

fn number(raw: &str) -> Option<f64> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    cleaned.parse().ok()
}

/// `table.tblGPASummary.mediaPC` under 通算: row 1 names the groups
/// (全ての科目 / 教養科目群 / 専門科目群), row 2 the columns (GPA, GPT,
/// 修得済単位, 合格率%), row 3 the values in the same order.
pub fn gpa_summaries(data: &Value) -> Vec<Value> {
    let Some(table) = data
        .get("overall")
        .and_then(Value::as_array)
        .and_then(|tables| tables.first())
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let row = |index: usize| -> Vec<String> {
        table
            .get(index)
            .and_then(Value::as_array)
            .map(|cells| {
                cells
                    .iter()
                    .map(|cell| cell.as_str().unwrap_or("").replace(' ', ""))
                    .collect()
            })
            .unwrap_or_default()
    };
    let (labels, values) = (row(1), row(2));
    if labels.is_empty() || labels.len() != values.len() {
        return Vec::new();
    }
    let fetched = Utc::now().to_rfc3339();
    let mut scopes: Vec<Value> = Vec::new();
    let mut current: Option<serde_json::Map<String, Value>> = None;
    let order = ["all", "liberal-arts", "major"];
    for (label, value) in labels.iter().zip(values.iter()) {
        // Every group starts with its GPA column.
        if label.eq_ignore_ascii_case("GPA") {
            if let Some(done) = current.take() {
                scopes.push(Value::Object(done));
            }
            let scope = order.get(scopes.len()).copied().unwrap_or("other");
            let mut entry = serde_json::Map::new();
            entry.insert("id".into(), format!("kyomu-gpa:{scope}").into());
            entry.insert("scope".into(), scope.into());
            entry.insert("earnedCredits".into(), 0.into());
            entry.insert("fetchedAt".into(), fetched.clone().into());
            current = Some(entry);
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        let key = if label.eq_ignore_ascii_case("GPA") {
            "gpa"
        } else if label.eq_ignore_ascii_case("GPT") {
            "gpt"
        } else if label.contains("単位") {
            "earnedCredits"
        } else if label.contains("合格率") {
            "passRate"
        } else {
            continue;
        };
        if let Some(number) = number(value) {
            entry.insert(key.into(), number.into());
        }
    }
    if let Some(done) = current.take() {
        scopes.push(Value::Object(done));
    }
    scopes
        .into_iter()
        .filter(|scope| scope["scope"] != "other")
        .collect()
}

/// One row per declared course with the credits 教務Web counts: the row's
/// split, scaled so each category adds up to the total the page states.
pub fn declared_credits(data: &Value) -> Vec<Value> {
    let courses = data
        .get("courses")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let raw = |course: &Value| {
        ["lecture", "exercise", "lab"]
            .iter()
            .filter_map(|key| {
                course
                    .pointer(&format!("/credits/{key}"))
                    .and_then(Value::as_f64)
            })
            .sum::<f64>()
    };
    let mut unit_sums: HashMap<String, f64> = HashMap::new();
    for course in &courses {
        let unit = course
            .get("unit")
            .and_then(Value::as_str)
            .unwrap_or_default();
        *unit_sums.entry(unit.to_string()).or_default() += raw(course);
    }
    courses
        .iter()
        .filter_map(|course| {
            let offering = course.get("offeringId")?.as_str()?;
            let unit = course
                .get("unit")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let listed = raw(course);
            let sum = unit_sums.get(unit).copied().unwrap_or(0.0);
            let effective = match course.get("unitTotal").and_then(Value::as_f64) {
                Some(total) if sum > 0.0 => listed * total / sum,
                _ => listed,
            };
            let mut row = course.clone();
            let object = row.as_object_mut()?;
            object.insert("id".into(), format!("kyomu-declared:{offering}").into());
            object.insert("kind".into(), "declared".into());
            // Two decimals: halves and quarters stay exact, float noise does not.
            object.insert(
                "effectiveCredits".into(),
                ((effective * 100.0).round() / 100.0).into(),
            );
            Some(row)
        })
        .collect()
}

pub fn apply_kyomu_payload(
    connection: &mut Connection,
    payload: &KyomuPayload,
) -> Result<usize, String> {
    let tx = connection.transaction().map_err(|e| e.to_string())?;
    let run = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO import_runs(id,provider,scope,state,started_at) VALUES(?1,'kyomu',?2,'running',?3)",params![run,payload.kind,Utc::now().to_rfc3339()]).map_err(|e|e.to_string())?;
    let mut changed = 0;
    match payload.kind.as_str() {
        "registrations" => {
            let mut state_ids = HashSet::new();
            if let Some(state) = payload.data.get("state") {
                let id = "kyomu-registration-state";
                state_ids.insert(id.to_string());
                let mut value = state.clone();
                if let Some(o) = value.as_object_mut() {
                    o.insert("id".into(), id.into());
                }
                changed += usize::from(upsert(&tx, "registration_states", id, &value, &[])?);
            }
            let mut ids = HashSet::new();
            for course in payload
                .data
                .get("courses")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(offering) = course.get("offeringId").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(Ok(year)) = offering.get(..4).map(str::parse::<i32>) {
                    resolve_course_id(&tx, "kyomu", offering, year, Some(offering))?;
                }
                let id = format!("kyomu-registration:{offering}");
                ids.insert(id.clone());
                let mut value = course.clone();
                if let Some(o) = value.as_object_mut() {
                    o.insert("id".into(), id.clone().into());
                }
                changed += usize::from(upsert(&tx, "registrations", &id, &value, &[])?);
            }
            changed += mark_missing(&tx, "registrations", "kyomu-registration:", &ids)?;
            changed += mark_missing(
                &tx,
                "registration_states",
                "kyomu-registration-state",
                &state_ids,
            )?;
        }
        "declared" => {
            let rows = declared_credits(&payload.data);
            let live: HashSet<String> = rows
                .iter()
                .filter_map(|row| row["id"].as_str().map(str::to_string))
                .collect();
            for row in &rows {
                let id = row["id"].as_str().unwrap_or_default();
                changed += usize::from(upsert(&tx, "registrations", id, row, &[])?);
            }
            // The page lists every declaration; withdrawn ones leave it.
            let mut statement = tx
                .prepare("SELECT id FROM registrations WHERE id LIKE 'kyomu-declared:%' AND missing_since IS NULL")
                .map_err(|e| e.to_string())?;
            let stale = statement
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(statement);
            for id in stale.into_iter().filter(|id| !live.contains(id)) {
                let missing = Utc::now().to_rfc3339();
                changed += tx
                    .execute(
                        "UPDATE registrations SET missing_since=?2,rev=rev+1,value=json_set(value,'$.missingSince',?2,'$.rev',rev+1,'$.updatedAt',?2) WHERE id=?1",
                        params![id, missing],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
        "grades" => {
            let mut live = HashSet::new();
            for group in payload
                .data
                .get("groups")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let category = group.get("category").and_then(Value::as_str).unwrap_or("");
                for (index, row) in group
                    .get("rows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let code = row.get("code").and_then(Value::as_str).unwrap_or("");
                    let completed = row.get("completed").and_then(Value::as_str).unwrap_or("");
                    let id = format!("kyomu-grade:{code}:{completed}:{index}");
                    live.insert(id.clone());
                    let mut value = row.clone();
                    if let Some(o) = value.as_object_mut() {
                        o.insert("id".into(), id.clone().into());
                        o.insert("category".into(), json!({"ja":category}));
                    }
                    changed += usize::from(upsert(&tx, "grades", &id, &value, &[])?);
                }
            }
            changed += mark_missing(&tx, "grades", "kyomu-grade:", &live)?;
        }
        "requirements" => {
            let mut live = HashSet::new();
            for (table_index, table) in payload
                .data
                .get("tables")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let headers = table
                    .get("headers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for row in table
                    .get("rows")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let label = row.get("label").and_then(Value::as_str).unwrap_or("");
                    for (column, value) in row
                        .get("values")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        let category = headers
                            .get(column + 1)
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let id = format!("kyomu-requirement:{table_index}:{category}:{label}");
                        live.insert(id.clone());
                        let record = json!({"id":id,"table":table_index,"category":category,"label":label,"value":value});
                        changed += usize::from(upsert(
                            &tx,
                            "requirement_progress",
                            record["id"].as_str().unwrap(),
                            &record,
                            &[],
                        )?);
                    }
                }
            }
            changed += mark_missing(&tx, "requirement_progress", "kyomu-requirement:", &live)?;
        }
        "gpa" => {
            let mut summary_ids = HashSet::new();
            for summary in gpa_summaries(&payload.data) {
                let id = summary["id"].as_str().unwrap_or_default().to_string();
                summary_ids.insert(id.clone());
                let scope = summary["scope"].as_str().unwrap_or_default().to_string();
                changed += usize::from(upsert(
                    &tx,
                    "gpa_summary",
                    &id,
                    &summary,
                    &[("scope", scope.into())],
                )?);
            }
            let mut term_ids = HashSet::new();
            for (index, row) in payload
                .data
                .get("terms")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let id = format!("kyomu-gpa-term:{index}");
                term_ids.insert(id.clone());
                let value = json!({"id":id,"cells":row});
                changed += usize::from(upsert(&tx, "gpa_by_term", &id, &value, &[])?);
            }
            changed += mark_missing(&tx, "gpa_summary", "kyomu-gpa:", &summary_ids)?;
            changed += mark_missing(&tx, "gpa_by_term", "kyomu-gpa-term:", &term_ids)?;
        }
        _ => return Err("unsupported kyomu payload".into()),
    }
    tx.execute(
        "UPDATE import_runs SET state='complete',finished_at=?2,message=?3 WHERE id=?1",
        params![
            run,
            Utc::now().to_rfc3339(),
            format!("{changed} rows changed")
        ],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}
