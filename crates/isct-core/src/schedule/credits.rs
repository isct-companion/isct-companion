use serde_json::Value;

use super::{text, ScheduleError, Term};

pub fn declared_term_credits(rows: &[Value], term: &str) -> Result<Option<f64>, ScheduleError> {
    let term = Term::parse(term)?;
    let year = term.year.to_string();
    let declared: Vec<_> = rows
        .iter()
        .filter(|row| {
            row.get("deletedAt").is_none_or(Value::is_null)
                && row.get("missingSince").is_none_or(Value::is_null)
                && row["quarters"].as_array().is_some_and(|q| {
                    q.iter()
                        .any(|v| v.as_u64() == Some(u64::from(term.quarter)))
                })
                && text(row, "kind") == Some("declared")
                && text(row, "offeringId").is_some_and(|id| id.starts_with(&year))
        })
        .collect();
    if declared.is_empty() {
        return Ok(None);
    }
    let total = declared.iter().fold(0.0, |sum, row| {
        let quarters = row
            .get("quarters")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if quarters
            .iter()
            .any(|value| value.as_u64() == Some(u64::from(term.quarter)))
            && !quarters.is_empty()
        {
            sum + row
                .get("effectiveCredits")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                / quarters.len() as f64
        } else {
            sum
        }
    });
    Ok(Some((total * 100.0).round() / 100.0))
}

/// Published results are the historical credit ledger. Quarter spans share
/// the effective total, as they do on the current declaration page.
pub(crate) fn earned_credits_by_term(rows: &[Value]) -> std::collections::BTreeMap<String, f64> {
    let mut totals = std::collections::BTreeMap::new();
    for row in rows.iter().filter(|r| {
        r.get("deletedAt").is_none_or(Value::is_null)
            && r.get("missingSince").is_none_or(Value::is_null)
    }) {
        let completed = text(row, "completed").unwrap_or("");
        let Some(year) = completed
            .get(..4)
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let quarter_text = text(row, "quarter").unwrap_or("");
        let mut quarters: std::collections::BTreeSet<_> = quarter_text
            .chars()
            .filter_map(|c| c.to_digit(10))
            .filter(|q| (1..=4).contains(q))
            .collect();
        if quarter_text.contains(['-', '〜', '～', '–']) && quarters.len() == 2 {
            if let (Some(&first), Some(&last)) = (quarters.first(), quarters.last()) {
                quarters.extend(first..=last);
            }
        }
        if quarters.is_empty() {
            continue;
        }
        let result = text(row, "result").unwrap_or("").trim();
        let numeric = result.parse::<f64>().ok().filter(|v| v.is_finite());
        let passed = numeric.is_some_and(|v| v >= 60.0)
            || matches!(
                result,
                "合格" | "認定" | "P" | "Pass" | "認" | "A" | "B" | "C" | "S"
            );
        // Pending/unknown results do not establish an earned-credit ledger.
        if !passed && numeric.is_none() && !matches!(result, "不合格" | "不可" | "F" | "Fail")
        {
            continue;
        }
        let credits = row["effectiveCredits"].as_f64().or_else(|| {
            let raw = text(row, "credits")?;
            raw.trim()
                .trim_matches(['[', ']'])
                .split('-')
                .map(|part| part.trim().parse::<f64>())
                .collect::<Result<Vec<_>, _>>()
                .ok()
                .map(|parts| parts.iter().sum())
        });
        let Some(credits) = credits.filter(|v| v.is_finite() && *v >= 0.0) else {
            continue;
        };
        for quarter in &quarters {
            *totals.entry(format!("{year}-{quarter}Q")).or_insert(0.0) += if passed {
                credits / quarters.len() as f64
            } else {
                0.0
            };
        }
    }
    for total in totals.values_mut() {
        *total = (*total * 100.0).round() / 100.0;
    }
    totals
}
