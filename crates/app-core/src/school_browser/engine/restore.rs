//! Restore records for the last visible school page.
use super::*;

#[derive(Deserialize, Serialize)]
pub(super) struct RestoreRecord {
    pub(super) site: SchoolSite,
    pub(super) url: String,
}

impl SchoolBrowserEngine {
    pub fn configure_restore(
        &self,
        enabled: bool,
        path: std::path::PathBuf,
    ) -> Result<bool, SchoolBrowserError> {
        let _restore_io = self.0.restore_io.guard();
        if !enabled {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(SchoolBrowserError::Storage(error.to_string())),
            }
        }
        let current = {
            let mut state = self.0.state.guard();
            state.restore_epoch = state.restore_epoch.saturating_add(1);
            let epoch = state.restore_epoch;
            state.restore = enabled.then_some((path.clone(), epoch));
            state.browsing.and_then(|site| {
                state
                    .views
                    .get(&site)
                    .and_then(|view| restore_record(&view.url).map(|url| (site, url)))
            })
        };
        if enabled {
            if let Some((site, url)) = current {
                let bytes = serde_json::to_vec(&RestoreRecord { site, url })
                    .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
                crate::notes::atomic_write(&path, &bytes)
                    .map_err(|error| SchoolBrowserError::Storage(error.to_string()))?;
            }
        }
        Ok(true)
    }

    pub(super) fn restored_target(&self, site: SchoolSite) -> Option<String> {
        let path = self.0.state.guard().restore.clone()?.0;
        let bytes = std::fs::read(path).ok()?;
        let record: RestoreRecord = serde_json::from_slice(&bytes).ok()?;
        (record.site == site)
            .then(|| restore_record(&record.url))
            .flatten()
            .filter(|url| SchoolSite::for_url(url) == Some(site))
    }
}

impl Inner {
    /// Records the loaded page off the callback thread. The epoch check under
    /// the restore I/O lock drops a late write after restore was disabled.
    pub(super) fn write_restore(
        self: &Arc<Self>,
        path: std::path::PathBuf,
        epoch: u64,
        site: SchoolSite,
        url: String,
    ) {
        let record = RestoreRecord { site, url };
        if let Ok(bytes) = serde_json::to_vec(&record) {
            let owner = self.clone();
            let tasks = self.tasks.clone();
            let _entered = self.runtime.enter();
            tasks.spawn(async move {
                let _ = tokio::task::spawn_blocking(move || {
                    let _io = owner.restore_io.guard();
                    let current = owner.state.with(|state| state.restore.clone());
                    if current == Some((path.clone(), epoch)) {
                        let _ = crate::notes::atomic_write(&path, &bytes);
                    }
                })
                .await;
            });
        }
    }
}
