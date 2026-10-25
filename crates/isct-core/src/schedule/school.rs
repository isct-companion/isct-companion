//! School records read models evaluate beside the workspace, which does not
//! carry them.
use crate::contracts::ClassFormat;
use serde::Serialize;
#[cfg(test)]
use serde_json::Value;
use std::collections::BTreeMap;

/// What a timetable course shows of its catalog offering.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct CatalogEntry {
    pub code: String,
    pub format: ClassFormat,
}

/// School records schedule read models evaluate with the workspace. Core
/// reads them in the transaction that reads the workspace, so the two always
/// agree, and keeps them out of the workspace the shells hold.
#[derive(Clone, Debug, Default)]
pub struct SchoolReads {
    /// The catalog offerings the workspace's courses name, by offering id.
    pub(crate) catalog: BTreeMap<String, CatalogEntry>,
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
        }
    }
}
