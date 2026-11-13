//! The campus map file as Isct-Data published it: one cache row holding the
//! `.gz` bytes and the hash the index declared for them. The reference read
//! writes it in the transaction that marks its key fresh; a cache reset drops
//! the table with the rest, and the file is downloaded again.
use super::*;

/// The one row's key: the format the file has (`campus-map/v1`).
const FORMAT: i64 = crate::campus_map::FORMAT_VERSION as i64;

/// Stores the file, replacing the previous one. Returns 1 when it changed.
pub(crate) fn put_campus_map(
    transaction: &Transaction<'_>,
    hash: &str,
    gz: &[u8],
    units: usize,
) -> Result<usize, DataError> {
    let stored: Option<String> = transaction
        .query_row(
            "SELECT hash FROM campus_map WHERE format=?1",
            [FORMAT],
            |row| row.get(0),
        )
        .optional()?;
    if stored.as_deref() == Some(hash) {
        return Ok(0);
    }
    transaction.execute(
        "INSERT INTO campus_map(format,hash,data,units) VALUES(?1,?2,?3,?4)
         ON CONFLICT(format) DO UPDATE SET hash=excluded.hash, data=excluded.data,
           units=excluded.units",
        params![FORMAT, hash, gz, i64::try_from(units).unwrap_or(i64::MAX)],
    )?;
    Ok(1)
}

/// Units the stored file holds; 0 without one.
pub(crate) fn campus_map_units(connection: &Connection) -> Result<usize, DataError> {
    let units: Option<i64> = connection
        .query_row(
            "SELECT units FROM campus_map WHERE format=?1",
            [FORMAT],
            |row| row.get(0),
        )
        .optional()?;
    Ok(units.map_or(0, |units| usize::try_from(units).unwrap_or(0)))
}

impl DataStore {
    /// The stored file's hash, which says whether a parsed copy is current.
    pub(crate) fn campus_map_hash(&self) -> Result<Option<String>, DataError> {
        Ok(self
            .open()?
            .query_row(
                "SELECT hash FROM campus_map WHERE format=?1",
                [FORMAT],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// The stored file: its hash and `.gz` bytes.
    pub(crate) fn campus_map_file(&self) -> Result<Option<(String, Vec<u8>)>, DataError> {
        Ok(self
            .open()?
            .query_row(
                "SELECT hash, data FROM campus_map WHERE format=?1",
                [FORMAT],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }
}
