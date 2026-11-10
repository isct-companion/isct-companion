//! Registration guide shown inside 教務Web. The app hands over a queue of
//! planned courses; the page shows them one at a time and highlights where the
//! person clicks. It never clicks, fills, submits or posts back by itself.

use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_ITEMS: usize = 40;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GuideItem {
    pub offering_id: String,
    pub code: String,
    pub title: String,
    /// 1 = Monday … 5 = Friday; absent for intensive / declaration-only.
    pub day: Option<u8>,
    /// Human-readable slot, e.g. `3Q 月3-4`.
    pub slot: String,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum GuideError {
    #[error("Too many courses for the registration guide.")]
    TooManyCourses,
    #[error("A registration guide item is invalid.")]
    InvalidItem,
}

fn bounded(value: &str, max: usize) -> bool {
    value.chars().count() <= max && !value.chars().any(char::is_control)
}

/// Rejects anything that is not a plain, bounded course row.
pub fn validate(items: Vec<GuideItem>) -> Result<Vec<GuideItem>, GuideError> {
    if items.len() > MAX_ITEMS {
        return Err(GuideError::TooManyCourses);
    }
    for item in &items {
        let id_ok = item.offering_id.is_empty()
            || (item.offering_id.len() == 9
                && item.offering_id.bytes().all(|b| b.is_ascii_digit()));
        if !id_ok
            || !bounded(&item.code, 32)
            || !bounded(&item.title, 200)
            || !bounded(&item.slot, 60)
            || item.day.is_some_and(|day| !(1..=7).contains(&day))
        {
            return Err(GuideError::InvalidItem);
        }
    }
    Ok(items)
}

/// The script evaluated on 教務Web pages while a guide is active.
pub fn script(items: &[GuideItem]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let queue = serde_json::to_string(items).ok()?;
    Some(GUIDE_SCRIPT.replace("__QUEUE__", &queue))
}

const GUIDE_SCRIPT: &str = r#"(() => {
  if (location.hostname !== 'kyomu0.gakumu.titech.ac.jp') return;
  const queue = __QUEUE__;
  const ID = 'isct-companion-guide';
  document.getElementById(ID)?.remove();
  document.getElementById(ID + '-style')?.remove();
  const style = document.createElement('style');
  style.id = ID + '-style';
  style.textContent = `
    #${ID}{position:fixed;right:16px;bottom:16px;z-index:2147483647;width:300px;max-height:70vh;display:flex;flex-direction:column;
      font:13px/1.45 -apple-system,BlinkMacSystemFont,"Hiragino Sans",sans-serif;color:#1d2433;background:#fff;
      border:1px solid #d5dae3;border-radius:12px;box-shadow:0 12px 32px rgba(20,30,50,.22);overflow:hidden}
    #${ID} header{display:flex;align-items:center;gap:8px;padding:10px 12px;background:#2f4f7f;color:#fff}
    #${ID} header b{flex:1;font-size:13px}
    #${ID} button{font:inherit;border:1px solid #c9d0db;background:#fff;color:#1d2433;border-radius:7px;padding:3px 9px;cursor:pointer}
    #${ID} header button{border-color:transparent;background:rgba(255,255,255,.16);color:#fff}
    #${ID} .now{padding:12px;border-bottom:1px solid #eef0f4}
    #${ID} .now .code{font:600 12px ui-monospace,Menlo,monospace;color:#2f4f7f}
    #${ID} .now .title{font-size:15px;font-weight:650;margin:2px 0}
    #${ID} .now .slot{color:#5b6475}
    #${ID} .hint{margin-top:8px;padding:7px 9px;border-radius:8px;background:#f2f6fc;color:#2f4f7f}
    #${ID} .hint.warn{background:#fff4e5;color:#8a4b00}
    #${ID} .row{display:flex;gap:6px;margin-top:10px}
    #${ID} .row .grow{flex:1}
    #${ID} ol{margin:0;padding:6px 0;overflow:auto;list-style:none}
    #${ID} li{display:flex;gap:8px;padding:4px 12px;cursor:pointer;color:#5b6475}
    #${ID} li.current{background:#eef3fb;color:#1d2433;font-weight:600}
    #${ID} li.done{color:#2e7d4f}
    #${ID} li i{font-style:normal;width:14px}
    #${ID}.min ol,#${ID}.min .now{display:none}
    .isct-companion-guide-target{outline:3px solid #2f7de1 !important;outline-offset:2px;border-radius:4px;animation:isct-companion-pulse 1.4s ease-in-out infinite}
    .isct-companion-guide-danger{outline:3px dashed #e07b00 !important;outline-offset:2px}
    .isct-companion-guide-match{background:#e7f6ec !important;outline:2px solid #2e9d5b !important}
    @keyframes isct-companion-pulse{50%{outline-color:rgba(47,125,225,.25)}}`;
  document.documentElement.appendChild(style);

  const key = 'isct-companion-guide-index';
  const read = () => { try { return Number(sessionStorage.getItem(key)) || 0; } catch { return 0; } };
  const write = (value) => { try { sessionStorage.setItem(key, String(value)); } catch {} };
  const days = ['', '月', '火', '水', '木', '金', '土', '日'];
  const onTimetable = location.pathname.endsWith('/PID1_0.aspx');
  const listed = new Set();
  for (const block of document.querySelectorAll('table#timetable [data-jwc]')) listed.add(block.getAttribute('data-jwc'));
  for (const link of document.querySelectorAll('a[href*="jwc="]')) {
    const match = link.getAttribute('href').match(/jwc=(\d{9})/);
    if (match && (onTimetable || link.closest('table#timetable, table[id^="kamokuUnit_"]'))) listed.add(match[1]);
  }
  const done = (item) => onTimetable && Boolean(item.offeringId) && listed.has(item.offeringId);
  let index = Math.min(read(), queue.length - 1);
  if (onTimetable && done(queue[index])) {
    const next = queue.findIndex((item) => !done(item));
    if (next >= 0) index = next;
  }

  const panel = document.createElement('section');
  panel.id = ID;
  const h = (tag, props = {}, ...children) => {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children);
    return node;
  };
  const clearMarks = () => {
    for (const cls of ['isct-companion-guide-target', 'isct-companion-guide-danger', 'isct-companion-guide-match'])
      for (const node of document.querySelectorAll('.' + cls)) node.classList.remove(cls);
  };
  const statusText = (document.querySelector('#ctl00_ContentPlaceHolder1_ctl00_lblStatus2')?.textContent || '').trim();
  const locked = /本登録済|本登録$/.test(statusText);

  const render = () => {
    write(index);
    clearMarks();
    const item = queue[index];
    const remaining = queue.filter((entry) => !done(entry)).length;
    const hints = [];
    if (onTimetable && remaining === 0) {
      hints.push(['', 'All queued courses are on 教務Web. Run the error check, then 本登録.']);
    } else if (onTimetable && done(item)) {
      hints.push(['', 'Already on 教務Web ✓']);
    } else if (onTimetable) {
      if (locked) {
        hints.push(['warn', 'Registration is final (本登録済). Adding needs 本登録解除 first, then 本登録 again before the deadline.']);
        const release = [...document.querySelectorAll('input[type=submit], button')].find((node) => /本登録解除/.test(node.value || node.textContent || ''));
        release?.classList.add('isct-companion-guide-danger');
      }
      const add = item.day ? document.querySelector(`a[href*="btnAdd__${item.day}'"]`) : null;
      if (add) {
        add.classList.add('isct-companion-guide-target');
        add.scrollIntoView({ block: 'center', behavior: 'smooth' });
        hints.push(['', `Click ${days[item.day]} in the header, then search ${item.code}.`]);
      } else {
        const byCourse = document.querySelector('a[href$="PID1_1.aspx"]');
        byCourse?.classList.add('isct-companion-guide-target');
        hints.push(['', `Open 科目から and search ${item.code}.`]);
      }
    } else {
      // Search / add pages: point at the code wherever it appears.
      const cells = [...document.querySelectorAll('td, span, a')].filter(
        (node) => node.children.length === 0 && (node.textContent || '').includes(item.code));
      const rows = new Set(cells.map((node) => node.closest('tr') || node));
      for (const row of rows) row.classList.add('isct-companion-guide-match');
      const first = [...rows][0];
      if (first) {
        first.scrollIntoView({ block: 'center', behavior: 'smooth' });
        hints.push(['', `Found ${item.code}. Add it with the row's own button.`]);
      } else {
        const field = [...document.querySelectorAll('input[type=text], input:not([type])')].find((node) => node.offsetParent);
        field?.classList.add('isct-companion-guide-target');
        hints.push(['', `Search for ${item.code}${field ? ' in the highlighted field' : ''}.`]);
      }
    }
    const now = h('div', { className: 'now' },
      h('div', { className: 'code', textContent: item.code }),
      h('div', { className: 'title', textContent: item.title }),
      h('div', { className: 'slot', textContent: item.slot }),
      ...hints.map(([kind, text]) => h('div', { className: 'hint' + (kind ? ' ' + kind : ''), textContent: text })),
      h('div', { className: 'row' },
        h('button', { type: 'button', textContent: '‹', title: 'Previous', disabled: index === 0, onclick: () => { index -= 1; render(); } }),
        h('button', { type: 'button', className: 'grow', textContent: 'Copy code', onclick: (event) => {
          navigator.clipboard?.writeText(item.code).then(() => { event.target.textContent = 'Copied ✓'; }, () => {});
        } }),
        h('button', { type: 'button', textContent: '›', title: 'Next', disabled: index >= queue.length - 1, onclick: () => { index += 1; render(); } })));
    const list = h('ol', {}, ...queue.map((entry, position) => h('li', {
      className: (position === index ? 'current ' : '') + (done(entry) ? 'done' : ''),
      onclick: () => { index = position; render(); },
    }, h('i', { textContent: done(entry) ? '✓' : String(position + 1) }), h('span', { textContent: `${entry.code} ${entry.title}` }))));
    const minimized = panel.classList.contains('min');
    panel.replaceChildren(
      h('header', {},
        h('b', { textContent: `Registration queue · ${queue.length - remaining}/${queue.length}` }),
        h('button', { type: 'button', textContent: minimized ? '▴' : '▾', title: 'Minimize', onclick: () => { panel.classList.toggle('min'); render(); } })),
      now, list);
  };
  render();
  document.body.appendChild(panel);
})()"#;

#[cfg(test)]
#[path = "tests/kyomu_guide.rs"]
mod tests;
