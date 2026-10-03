use crate::data::versioned::content_hash;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use url::Url;
use uuid::Uuid;

const MAX_PAYLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_RECORDS: usize = 5_000;

/// App-owned, read-only reader for the four approved 教務 pages. It only
/// inspects the current document and returns plain data through the relay.
pub const KYOMU_EXTRACT_SCRIPT: &str = r#"(() => {
  const clean = value => String(value || '').normalize('NFKC').replace(/\s+/g, ' ').trim();
  const text = (root, selector) => clean(root.querySelector(selector)?.textContent);
  const cells = row => [...row.querySelectorAll(':scope > th, :scope > td')].map(cell => clean(cell.textContent));
  const sourceUrl = location.href;
  const allowed = new Map([
    ['/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx','registrations'],
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
  const schedule = raw => clean(raw).split(/\s*[、,]\s*/).map(part => {
    const match = part.match(/([1-4])Q[:：]\s*([月火水木金土日])\s*(\d+)\s*[-–〜~]\s*(\d+)/);
    if (!match) return {raw:part};
    const first = Number(match[3]), last = Number(match[4]);
    return {quarter:match[1]+'Q', day:match[2], periods:Array.from({length:last-first+1},(_,i)=>first+i)};
  }).filter(item => item.raw || item.quarter);
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
      const rawSchedule = [text(block,'.ttYoubi'),text(block,'.ttYoubi2')].filter(Boolean).join(',');
      courses.push({offeringId,title:text(block,'.ttKamokuName a'),code:text(block,'.ttSinkokuNo'),
        instructor:text(block,'.ttKyoin'),schedule:schedule(rawSchedule),requirement:requirement(text(block,'.ttSuisho')),
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
        courses.push({offeringId,title:clean(link?.textContent),code:text(row,'span.resultUnitKamokuCodeSP'),
          instructor:at('授業担当教員'),schedule:schedule(rawSchedule),declaredOnly:rawSchedule === '申告専用',
          requirement:requirement(at('備考')),credits:credit ? {lecture:Number(credit[1]),exercise:Number(credit[2]),lab:Number(credit[3])}:null});
      }
    }
    const savedRaw = text(document,'#ctl00_ContentPlaceHolder1_ctl00_lblStatus3');
    const savedMatch = savedRaw.match(/(\d{4})\/(\d{2})\/(\d{2})\s+(\d{2}):(\d{2}):(\d{2})/);
    const savedAt = savedMatch ? `${savedMatch[1]}-${savedMatch[2]}-${savedMatch[3]}T${savedMatch[4]}:${savedMatch[5]}:${savedMatch[6]}+09:00` : null;
    return {status:document.querySelector('#divSinkokuStatus')?'ok':'login',sourceUrl,kind,
      data:{state:{state:stateMap[rawState]||'unknown',raw:rawState,savedAt},courses}};
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KyomuPayload {
    pub status: String,
    pub source_url: String,
    pub kind: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KyomuStage {
    Ready,
    ReconnectRequired,
}

fn expected_kind(path: &str) -> Option<&'static str> {
    match path {
        "/Titech/Student/%E7%A7%91%E7%9B%AE%E7%94%B3%E5%91%8A/PID1_0.aspx" => Some("registrations"),
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_0.aspx" => Some("grades"),
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_6.aspx" => Some("requirements"),
        "/Titech/Student/%E6%88%90%E7%B8%BE%E9%96%B2%E8%A6%A7/PID2_7.aspx" => Some("gpa"),
        _ => None,
    }
}

pub fn parse_kyomu_payload(raw: &str) -> Result<(KyomuStage, Option<KyomuPayload>), String> {
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err("kyomu payload exceeds the size limit".into());
    }
    let payload: KyomuPayload =
        serde_json::from_str(raw).map_err(|_| "kyomu payload is not valid JSON")?;
    let url = Url::parse(&payload.source_url).map_err(|_| "kyomu source URL is invalid")?;
    if payload.status == "login" {
        let known_login_host = matches!(
            url.host_str(),
            Some("kyomu0.gakumu.titech.ac.jp" | "portal.titech.ac.jp" | "rp.nap.gsic.titech.ac.jp")
        );
        if url.scheme() == "https" && known_login_host {
            return Ok((KyomuStage::ReconnectRequired, None));
        }
    }
    if url.scheme() != "https"
        || url.host_str() != Some("kyomu0.gakumu.titech.ac.jp")
        || expected_kind(url.path()) != Some(payload.kind.as_str())
    {
        return Err("kyomu source page is not approved".into());
    }
    if payload.status != "ok" || !payload.data.is_object() {
        return Err("kyomu extraction was incomplete".into());
    }
    let count = match payload.kind.as_str() {
        "registrations" => payload
            .data
            .pointer("/courses")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        "grades" => payload
            .data
            .pointer("/groups")
            .and_then(Value::as_array)
            .map_or(0, |groups| {
                groups
                    .iter()
                    .filter_map(|g| g.get("rows").and_then(Value::as_array))
                    .map(Vec::len)
                    .sum()
            }),
        "requirements" => payload
            .data
            .pointer("/tables")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        "gpa" => payload
            .data
            .pointer("/terms")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        _ => return Err("kyomu payload kind is invalid".into()),
    };
    if count > MAX_RECORDS {
        return Err("kyomu payload exceeds the record limit".into());
    }
    Ok((KyomuStage::Ready, Some(payload)))
}

fn upsert(
    tx: &Transaction<'_>,
    table: &str,
    id: &str,
    value: &Value,
    extra: &[(&str, rusqlite::types::Value)],
) -> Result<bool, String> {
    let hash = content_hash(value)?;
    let old: Option<(String, u64)> = tx
        .query_row(
            &format!("SELECT hash,rev FROM {table} WHERE id=?1"),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if old.as_ref().is_some_and(|(h, _)| h == &hash) {
        return Ok(false);
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
    ];
    names.extend(extra.iter().map(|(n, _)| *n));
    let timestamp = Utc::now().to_rfc3339();
    let rev = old.map_or(0, |x| x.1) + 1;
    let mut stored = value.clone();
    if let Some(object) = stored.as_object_mut() {
        object.insert("rev".into(), rev.into());
        object.insert("updatedAt".into(), timestamp.clone().into());
        object.insert("updatedBy".into(), "kyomu".into());
        object.insert("source".into(), json!({"provider":"kyomu"}));
    }
    let mut values = vec![
        id.to_string().into(),
        serde_json::to_string(&stored)
            .map_err(|e| e.to_string())?
            .into(),
        hash.into(),
        timestamp.into(),
        String::from("kyomu").into(),
        (rev as i64).into(),
        String::from("kyomu").into(),
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
        let Some(entry) = current.as_mut() else { continue };
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
            if let Some(state) = payload.data.get("state") {
                let id = "kyomu-registration-state";
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
                let id = format!("kyomu-registration:{offering}");
                ids.insert(id.clone());
                let mut value = course.clone();
                if let Some(o) = value.as_object_mut() {
                    o.insert("id".into(), id.clone().into());
                }
                changed += usize::from(upsert(&tx, "registrations", &id, &value, &[])?);
            }
        }
        "grades" => {
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
                    let mut value = row.clone();
                    if let Some(o) = value.as_object_mut() {
                        o.insert("id".into(), id.clone().into());
                        o.insert("category".into(), json!({"ja":category}));
                    }
                    changed += usize::from(upsert(&tx, "grades", &id, &value, &[])?);
                }
            }
        }
        "requirements" => {
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
        }
        "gpa" => {
            for summary in gpa_summaries(&payload.data) {
                let id = summary["id"].as_str().unwrap_or_default().to_string();
                let scope = summary["scope"].as_str().unwrap_or_default().to_string();
                changed += usize::from(upsert(
                    &tx,
                    "gpa_summary",
                    &id,
                    &summary,
                    &[("scope", scope.into())],
                )?);
            }
            for (index, row) in payload
                .data
                .get("terms")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let id = format!("kyomu-gpa-term:{index}");
                let value = json!({"id":id,"cells":row});
                changed += usize::from(upsert(&tx, "gpa_by_term", &id, &value, &[])?);
            }
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
