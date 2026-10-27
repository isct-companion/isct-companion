//! School records read models evaluate beside the workspace, which does not
//! carry them.
use crate::contracts::ClassFormat;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a timetable course shows of its catalog offering.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct CatalogEntry {
    pub code: String,
    pub format: ClassFormat,
}

/// The published exam timetable: its visible rows that name a term
/// (`academicYear` and `quarter`), in id order, and where each term's are.
#[derive(Debug, Default)]
pub(crate) struct ExamTimetable {
    rows: Vec<Value>,
    by_term: BTreeMap<String, Vec<usize>>,
}

impl ExamTimetable {
    pub(crate) fn new(rows: Vec<Value>) -> Self {
        let mut timetable = Self::default();
        for row in rows.into_iter().filter(super::visible) {
            let Some(term) = super::exams::exam_term(&row) else {
                continue;
            };
            timetable
                .by_term
                .entry(term)
                .or_default()
                .push(timetable.rows.len());
            timetable.rows.push(row);
        }
        timetable
    }

    pub(crate) fn rows(&self) -> &[Value] {
        &self.rows
    }

    /// The terms with an exam, in order.
    pub(crate) fn terms(&self) -> impl Iterator<Item = &str> {
        self.by_term.keys().map(String::as_str)
    }

    /// `term`'s rows, in id order.
    pub(crate) fn of<'a>(&'a self, term: &str) -> impl Iterator<Item = &'a Value> {
        self.by_term
            .get(term)
            .into_iter()
            .flatten()
            .map(|index| &self.rows[*index])
    }
}

/// School records schedule read models evaluate with the workspace. Core
/// reads them in the transaction that reads the workspace, so the two always
/// agree, and keeps them out of the workspace the shells hold.
#[derive(Clone, Debug, Default)]
pub struct SchoolReads {
    /// The catalog offerings the workspace's courses name, by offering id.
    pub(crate) catalog: BTreeMap<String, CatalogEntry>,
    /// Shared by every snapshot of one cache.db version: no user intent
    /// changes it.
    pub(crate) exams: Arc<ExamTimetable>,
}

impl SchoolReads {
    pub(crate) fn offering(&self, id: &str) -> Option<&CatalogEntry> {
        self.catalog.get(id)
    }

    /// The school records a test workspace lists under the fields the
    /// workspace once carried them in.
    #[cfg(test)]
    pub(crate) fn fixture(workspace: &Value) -> Self {
        let text = |row: &Value, key: &str| row[key].as_str().unwrap_or_default().to_owned();
        Self {
            catalog: super::array(workspace, "offerings")
                .iter()
                .map(|offering| {
                    let format = serde_json::from_value(offering["format"].clone())
                        .unwrap_or(ClassFormat { kinds: Vec::new() });
                    (
                        text(offering, "id"),
                        CatalogEntry {
                            code: text(offering, "code"),
                            format,
                        },
                    )
                })
                .collect(),
            exams: Arc::new(ExamTimetable::new(
                super::array(workspace, "exams").to_vec(),
            )),
        }
    }
}
