use serde_json::Value;

use super::{text, ScheduleError, Term};

pub fn declared_term_credits(rows: &[Value], term: &str) -> Result<Option<f64>, ScheduleError> {
    let term = Term::parse(term)?;
    let year = term.year.to_string();
    let declared: Vec<_> = rows
        .iter()
        .filter(|row| {
            row.get("deletedAt").is_none_or(Value::is_null)
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
