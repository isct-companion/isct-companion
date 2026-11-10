//! The campus map: Isct-Data's export (format 1, `campus-map/v1`), which room
//! or building a course's room string points at, which building and floor the
//! map shows for where the camera is, and the floor read models shells draw
//! with their own map API. Core downloads and stores the file
//! (`reference_refresh`, the `campus_map` cache table); until it is stored
//! every read here answers nothing, and shells show no map button.
//!
//! Every `pub` serde type in this file crosses the FFI as a record
//! (`app-ffi/build.rs`); the export's own shape stays in `export`.
mod export;
mod geometry;
mod map;
mod matching;

use crate::data::{DataError, DataStore};
use map::Map;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, PoisonError};

/// The format this app reads, and its `index.json` key (`campusMap["1"]`).
pub(crate) const FORMAT_VERSION: u32 = 1;
pub(crate) const FORMAT_KEY: &str = "1";
/// The `.gz` download is refused beyond this; the export aims at 0.5 MB.
pub(crate) const MAX_COMPRESSED_BYTES: usize = 8 * 1024 * 1024;
/// The decompressed JSON is refused beyond this (contract v1).
pub(crate) const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;

/// WGS84 degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapPoint {
    pub latitude: f64,
    pub longitude: f64,
}

/// A box in WGS84 degrees.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapBounds {
    pub south: f64,
    pub west: f64,
    pub north: f64,
    pub east: f64,
}

/// One outer ring, then its holes; each ring is closed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapPolygon {
    pub rings: Vec<Vec<CampusMapPoint>>,
}

/// A unit's kind, the export's full list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampusMapUnitCategory {
    Classroom,
    Room,
    Corridor,
    ElevatorLobby,
    Restroom,
    Stairs,
    Lounge,
    Porch,
}

/// An amenity's kind, the export's full list. Each has its own icon.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CampusMapAmenityCategory {
    #[serde(rename = "elevator")]
    Elevator,
    #[serde(rename = "stairs")]
    Stairs,
    #[serde(rename = "restroom")]
    Restroom,
    #[serde(rename = "restroom.male")]
    RestroomMale,
    #[serde(rename = "restroom.female")]
    RestroomFemale,
    #[serde(rename = "restroom.wheelchair")]
    RestroomWheelchair,
    #[serde(rename = "aed")]
    Aed,
    #[serde(rename = "lounge")]
    Lounge,
    #[serde(rename = "lounge.women")]
    LoungeWomen,
    #[serde(rename = "lounge.international")]
    LoungeInternational,
    #[serde(rename = "rest_room")]
    RestRoom,
    #[serde(rename = "calm_down")]
    CalmDown,
    #[serde(rename = "study_room")]
    StudyRoom,
}

/// A drawn line's role on the source plans.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CampusMapLineTone {
    /// Black on the source plan: walls.
    Wall,
    /// Grey on the source plan: fittings and hairlines.
    Detail,
}

/// A way into a building, and the link that shows it in a maps app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapEntrance {
    pub id: String,
    /// In the content language, when the export names it.
    pub name: Option<String>,
    pub point: CampusMapPoint,
    pub level_id: Option<String>,
    /// `https://maps.apple.com/?ll=…&q=…` for this entrance.
    pub maps_url: String,
}

/// One floor, lowest first in a building's list (B1 = -1, 1F = 0).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapLevel {
    pub id: String,
    pub ordinal: i32,
    /// `B1`, `2F`.
    pub name: String,
}

/// A building's footprint, drawn on every zoom; the floors of one with
/// `levels` open once the camera focuses on it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapBuilding {
    pub id: String,
    /// In the content language.
    pub name: String,
    /// Room-code prefixes of its rooms (`W9`).
    pub codes: Vec<String>,
    pub center: CampusMapPoint,
    pub footprint: Vec<CampusMapPolygon>,
    /// Lowest first; empty for a building without floor plans, which the
    /// camera never focuses on.
    pub levels: Vec<CampusMapLevel>,
    /// Every way in, all drawn while the building is focused.
    pub entrances: Vec<CampusMapEntrance>,
    /// The building's centre in a maps app, for a building without entrances.
    pub maps_url: String,
}

/// Every building, read once per map page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapOverview {
    pub buildings: Vec<CampusMapBuilding>,
    /// The data's credit lines (OpenStreetMap, ODbL), shown on the map.
    pub attribution: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapUnit {
    pub id: String,
    pub category: CampusMapUnitCategory,
    /// A room's code, else the unit's own name.
    pub label: Option<String>,
    pub center: CampusMapPoint,
    pub polygon: CampusMapPolygon,
    pub room_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapLines {
    pub tone: CampusMapLineTone,
    /// Metres; 0 means as thin as the display can draw.
    pub width_m: f64,
    pub lines: Vec<Vec<CampusMapPoint>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapAmenity {
    pub id: String,
    pub category: CampusMapAmenityCategory,
    pub point: CampusMapPoint,
}

/// Everything drawn for one floor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapFloor {
    pub level_id: String,
    pub building_id: String,
    pub outline: Vec<CampusMapPolygon>,
    pub units: Vec<CampusMapUnit>,
    pub lines: Vec<CampusMapLines>,
    pub amenities: Vec<CampusMapAmenity>,
}

/// How much of a room string the map knows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CampusMapMatch {
    /// The room has units on a floor plan, which are highlighted.
    Room,
    /// Only the building is known (or the room has no unit drawn).
    Building,
}

/// One room or building a room string names: where the map page opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapTarget {
    /// The room's code, or the words of the string when only the building
    /// is known.
    pub label: String,
    pub matched: CampusMapMatch,
    pub room_id: Option<String>,
    pub building_id: String,
    pub building_name: String,
    pub levels: Vec<CampusMapLevel>,
    /// The floor shown on entry: the highlighted room's; `None` for a
    /// building without floor plans.
    pub level_id: Option<String>,
    /// Every unit of the room on the floors it is on.
    pub highlighted_unit_ids: Vec<String>,
    /// Where the camera goes on entry: the room on its floor, else the
    /// building.
    pub focus: CampusMapBounds,
}

/// Where the camera is after a move, and what the page shows now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapCamera {
    pub center: CampusMapPoint,
    pub bounds: CampusMapBounds,
    /// The map's width in points; with the bounds it gives the zoom level.
    pub width_points: f64,
    /// A gesture moved the camera; a move the app made never re-picks.
    pub by_user: bool,
    pub building_id: Option<String>,
    pub level_id: Option<String>,
}

/// The building in focus, its floors and the floor shown; all `None` and
/// empty when the camera is too far out or over no building with plans.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapFocus {
    pub building_id: Option<String>,
    pub levels: Vec<CampusMapLevel>,
    pub level_id: Option<String>,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum CampusMapError {
    /// Another format than the one this app reads; it is never stored.
    #[error("The campus map file has format {0}; this app reads format 1.")]
    Format(String),
    #[error("The campus map file is invalid: {0}")]
    Invalid(String),
    #[error("No campus map floor has the id {0}.")]
    UnknownLevel(String),
    #[error("{0}")]
    Storage(String),
}

impl From<DataError> for CampusMapError {
    fn from(error: DataError) -> Self {
        Self::Storage(error.to_string())
    }
}

/// A downloaded export, checked before it is stored: the `.gz` bytes the
/// index hash covers, and its units, the rows a suspicious shrink counts.
#[derive(Clone)]
pub struct CampusMapFile {
    pub(crate) gz: Vec<u8>,
    pub(crate) hash: String,
    pub(crate) units: usize,
}

impl std::fmt::Debug for CampusMapFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CampusMapFile")
            .field("bytes", &self.gz.len())
            .field("hash", &self.hash)
            .field("units", &self.units)
            .finish()
    }
}

impl CampusMapFile {
    /// Unpacks and checks the file as a read would use it; a file this app
    /// cannot read is never stored.
    pub(crate) fn read(gz: Vec<u8>, hash: String) -> Result<Self, CampusMapError> {
        let map = Map::new(export::read(&gz)?)?;
        Ok(Self {
            units: map.unit_count(),
            gz,
            hash,
        })
    }
}

/// The map reads of one store. The parsed file is kept while the stored
/// file's hash stays the same.
pub struct CampusMapService {
    store: DataStore,
    loaded: Mutex<Option<(String, Arc<Map>)>>,
}

impl CampusMapService {
    pub fn new(store: DataStore) -> Self {
        Self {
            store,
            loaded: Mutex::new(None),
        }
    }

    /// The stored file, parsed; `None` until one is stored.
    fn map(&self) -> Result<Option<Arc<Map>>, CampusMapError> {
        let mut loaded = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(hash) = self.store.campus_map_hash()? else {
            *loaded = None;
            return Ok(None);
        };
        if let Some((cached, map)) = loaded.as_ref() {
            if *cached == hash {
                return Ok(Some(map.clone()));
            }
        }
        let Some((hash, gz)) = self.store.campus_map_file()? else {
            *loaded = None;
            return Ok(None);
        };
        let map = Arc::new(Map::new(export::read(&gz)?)?);
        *loaded = Some((hash, map.clone()));
        Ok(Some(map))
    }

    /// Names in English when the content language is English.
    fn english(&self) -> Result<bool, CampusMapError> {
        let workspace = self.store.load_fields(&[])?;
        Ok(crate::schedule::content_language(&workspace) == "en")
    }

    /// Every building; empty until the file is stored.
    pub fn overview(&self) -> Result<CampusMapOverview, CampusMapError> {
        let Some(map) = self.map()? else {
            return Ok(CampusMapOverview {
                buildings: Vec::new(),
                attribution: Vec::new(),
            });
        };
        Ok(map.overview(self.english()?))
    }

    /// What a course's room string points at, in the string's order; every
    /// candidate of an ambiguous spelling. Empty when the map knows none of
    /// it, or no file is stored: then there is no map button.
    pub fn locate(&self, room: &str) -> Result<Vec<CampusMapTarget>, CampusMapError> {
        let Some(map) = self.map()? else {
            return Ok(Vec::new());
        };
        Ok(map.locate(room, self.english()?))
    }

    /// The floor plan of one level.
    pub fn floor(&self, level_id: &str) -> Result<CampusMapFloor, CampusMapError> {
        self.map()?
            .and_then(|map| map.floor(level_id))
            .ok_or_else(|| CampusMapError::UnknownLevel(level_id.to_owned()))
    }

    /// Which building and floor the page shows after a camera move.
    pub fn focus(&self, camera: &CampusMapCamera) -> Result<CampusMapFocus, CampusMapError> {
        Ok(match self.map()? {
            Some(map) => map.focus(camera),
            None => CampusMapFocus {
                building_id: None,
                levels: Vec::new(),
                level_id: None,
            },
        })
    }

    /// The room or named unit under a tap on `level_id`; the smallest one
    /// when units nest.
    pub fn unit_at(
        &self,
        level_id: &str,
        point: CampusMapPoint,
    ) -> Result<Option<CampusMapUnit>, CampusMapError> {
        Ok(self.map()?.and_then(|map| map.unit_at(level_id, point)))
    }
}

/// Also the hand-built export the reference tests store.
#[cfg(test)]
#[path = "../tests/campus_map.rs"]
pub(crate) mod tests;
