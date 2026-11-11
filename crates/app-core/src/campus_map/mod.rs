//! The campus map: Isct-Data's export (format 1, `campus-map/v1`), which room
//! or building a course's room string points at, and the map page itself.
//! Core downloads and stores the file (`reference_refresh`, the `campus_map`
//! cache table); until it is stored every read here answers nothing, and
//! shells show no map button.
//!
//! The page is Core's: shells open it, on a room string or on none, forward what the
//! person does as [`CampusMapIntent`]s and draw the [`CampusMapScene`] each
//! answer carries with their own map API, by role. Which building and floor
//! show, what is selected, what the card says, which marks are drawn and
//! where the camera goes are all decided here (`page`).
//!
//! Every `pub` serde type in this file crosses the FFI as a record
//! (`app-ffi/build.rs`); the export's own shape stays in `export`.
mod export;
mod geometry;
mod map;
mod matching;
mod page;
mod search;

use crate::data::{DataError, DataStore};
use map::Map;
use page::Page;
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

/// One floor of a building (B1 = -1, 1F = 0).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapLevel {
    pub id: String,
    pub ordinal: i32,
    /// `B1`, `2F`.
    pub name: String,
}

/// Where the camera settled after a move: the part of the map the person
/// can see, the screen less the search bar, the card, the level picker and
/// the safe area, as its corners on the ground (a rotated map gives a
/// rotated box).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapCamera {
    pub top_left: CampusMapPoint,
    pub top_right: CampusMapPoint,
    pub bottom_right: CampusMapPoint,
    pub bottom_left: CampusMapPoint,
    /// The visible part's width in screen points.
    pub width_points: f64,
    /// The visible part's height in screen points.
    pub height_points: f64,
    /// Not a move the app made: the shell made none (`setVisibleMapRect`)
    /// that was still under way. A move the app made never re-picks the
    /// building.
    pub by_user: bool,
}

/// A place the person picked, by its stable id in the export.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CampusMapPlace {
    Building {
        id: String,
    },
    Room {
        id: String,
    },
    /// A floor plan's unit, as its room mark names it.
    Unit {
        id: String,
    },
    Entrance {
        id: String,
    },
}

/// What the person did on the map page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CampusMapIntent {
    /// Picked one of the card's candidates, by its index there.
    Choose { index: u32 },
    /// The camera settled, after a gesture or a framing. `framing` is the
    /// key of the last framing the shell applied: a report from before the
    /// latest framing Core asked for changes nothing.
    Camera {
        camera: CampusMapCamera,
        framing: u64,
    },
    /// Picked a floor in the level picker.
    Level { id: String },
    /// Tapped the map anywhere but a mark, on the floor drawn then (`None`
    /// with no floor drawn): a unit of that floor, else a building. A tap on
    /// a floor the page no longer shows changes nothing.
    Tap {
        point: CampusMapPoint,
        level_id: Option<String>,
    },
    /// Picked a place: a search result, a room or entrance mark.
    Select { place: CampusMapPlace },
    /// Closed the card: nothing is selected.
    Clear,
}

/// What a layer of the scene is, which decides its colours: shells map each
/// role to their theme.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CampusMapLayerRole {
    /// Every building but the emphasized one, drawn on every zoom under the
    /// floor (above roads, below labels).
    Footprint,
    /// The building in focus, else the chosen room's building.
    FocusedFootprint,
    /// The shown floor's outline; this and every role below it are drawn
    /// above the base map's labels.
    LevelOutline,
    /// Classrooms, filled; other units have no fill.
    Classroom,
    /// Grey on the source plan: fittings and hairlines, under the walls.
    DetailLine,
    /// Black on the source plan: walls.
    WallLine,
    /// The selected room, over the plan.
    Highlight,
}

/// One drawn layer: polygons, or lines. A line layer has one physical width.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapLayer {
    /// Stable for the same content; shells key their drawing on it.
    pub id: String,
    pub role: CampusMapLayerRole,
    pub polygons: Vec<CampusMapPolygon>,
    pub lines: Vec<Vec<CampusMapPoint>>,
    /// Metres; zero for a polygon layer.
    pub width_m: f64,
}

/// A mark's kind and what it shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CampusMapMarkKind {
    /// A room's code or a unit's name; `highlighted` for the selected room,
    /// drawn larger and in the accent. Picking it selects `unit_id`.
    Room {
        unit_id: String,
        label: String,
        highlighted: bool,
    },
    /// An amenity's icon. `key` is stable lowercase (`restroom-male`):
    /// shells name the icon and the spoken name after it.
    Amenity { key: String },
    /// A way into the building, which the person can tap.
    Entrance {
        entrance_id: String,
        name: Option<String>,
        selected: bool,
    },
}

/// A point mark over the map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapMark {
    /// Unique within a scene.
    pub id: String,
    pub point: CampusMapPoint,
    pub kind: CampusMapMarkKind,
}

/// An "Open in Maps" link of the card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapLink {
    /// An entrance's link, else the building's centre.
    pub entrance: bool,
    /// The entrance's name, when the export names it.
    pub name: Option<String>,
    /// A pin in the maps app the settings name: `https://maps.apple.com/?ll=…&q=…`
    /// or `https://www.google.com/maps/search/?api=1&query=…`.
    pub url: String,
}

/// One room of an ambiguous spelling.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapCandidate {
    pub label: String,
    pub building_name: String,
    pub chosen: bool,
}

/// The card describes the selection and nothing else, so a camera move,
/// a floor picked or a zoom never changes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapCard {
    /// The selected room, unit, entrance or building; `None` for an entrance
    /// the export does not name.
    pub title: Option<String>,
    /// The selection's building, never the camera's.
    pub building_name: String,
    /// `ookayama` or `suzukakedai`; shells name it.
    pub campus: String,
    /// The selection's floor, when it has one.
    pub level_name: Option<String>,
    /// The chosen room is not on the floor plans: only its building shows.
    pub building_only: bool,
    /// The selection's building has no floor plans at all.
    pub no_floor_plans: bool,
    /// One per entrance of the selection's building, the tapped one first;
    /// the building's centre when it has none. No route is ever drawn.
    pub links: Vec<CampusMapLink>,
    /// Every room an ambiguous spelling names; empty for one.
    pub candidates: Vec<CampusMapCandidate>,
}

/// A box Core wants in view. A `key` larger than the last one the shell
/// applied moves the camera; the same key never does again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapFraming {
    pub key: u64,
    pub bounds: CampusMapBounds,
}

/// Everything the map page draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapScene {
    /// The page every intent names; an intent naming an older page, which
    /// a later `open` replaced, changes nothing.
    pub page: u64,
    /// Rises with every change: a scene older than the one drawn is dropped.
    pub revision: u64,
    /// Bottom to top: the footprints, then the floor shown, if any.
    pub layers: Vec<CampusMapLayer>,
    /// Bottom to top. Shell map APIs resolve collisions between these marks.
    pub marks: Vec<CampusMapMark>,
    /// The focused building's floors, top floor first; empty when no floor
    /// is shown.
    pub levels: Vec<CampusMapLevel>,
    pub level_id: Option<String>,
    /// The floor the selected room is on, while the picker lists its
    /// building.
    pub marked_level_id: Option<String>,
    /// `None` while nothing is selected: on a page opened on no room
    /// string, or one the map knows nothing of.
    pub card: Option<CampusMapCard>,
    pub framing: Option<CampusMapFraming>,
    /// The data's credit lines (OpenStreetMap, ODbL), shown on the map.
    pub attribution: Vec<String>,
}

/// One search answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapSearchResult {
    /// What selecting it sends.
    pub place: CampusMapPlace,
    /// The room's code, or the building's name.
    pub title: String,
    pub building_name: String,
    /// `ookayama` or `suzukakedai`.
    pub campus: String,
    /// The floor the room is on, when it is on the plans.
    pub level_name: Option<String>,
    /// Found, but not on the floor plans: a building without plans, or a
    /// room with no unit drawn. Selecting it shows the building only.
    pub no_floor_plan: bool,
}

/// The answers to one query. `query` is the text asked about, so a shell
/// drops answers to text the person has since changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CampusMapSearch {
    pub query: String,
    pub results: Vec<CampusMapSearchResult>,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum CampusMapError {
    /// Another format than the one this app reads; it is never stored.
    #[error("The campus map file has format {0}; this app reads format 1.")]
    Format(String),
    #[error("The campus map file is invalid: {0}")]
    Invalid(String),
    #[error("No campus map file is stored yet.")]
    Unavailable,
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

/// The map reads of one store and its one page: the page is a full-screen
/// cover, so opening one replaces any other, and a page the shell left
/// without closing is dropped by the next `open`. An open takes its id when
/// it is asked for and installs its page only if no later open or close came
/// meanwhile. The parsed file is kept
/// while the stored file's hash stays the same; the page keeps the file it
/// was opened on.
pub struct CampusMapService {
    store: DataStore,
    loaded: Mutex<Option<(String, Arc<Map>)>>,
    page: Mutex<OpenPage>,
}

/// The latest open or close and the page it installed; ids only rise, so
/// an older one is stale.
#[derive(Default)]
struct OpenPage {
    last: u64,
    page: Option<Page>,
}

impl CampusMapService {
    pub fn new(store: DataStore) -> Self {
        Self {
            store,
            loaded: Mutex::new(None),
            page: Mutex::new(OpenPage::default()),
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

    /// Names in English when the content language is English, and the maps
    /// app the card's links open in.
    fn preferences(&self) -> Result<page::Preferences, CampusMapError> {
        let workspace = self.store.load_fields(&[])?;
        Ok(page::Preferences {
            english: crate::schedule::content_language(&workspace) == "en",
            maps: map::MapsApp::from_setting(
                workspace
                    .pointer("/settings/mapsApp")
                    .and_then(serde_json::Value::as_str),
            ),
        })
    }

    /// With a room string: whether the map knows any room or building it
    /// names. Without one: whether a map is stored at all. `false` until a
    /// file is stored; shells show a map button only for `true`.
    pub fn has(&self, room: Option<&str>) -> Result<bool, CampusMapError> {
        Ok(self.map()?.is_some_and(|map| match room {
            Some(room) => !map.locate(room, false).is_empty(),
            None => true,
        }))
    }

    /// The places a search box's text names, best first.
    pub fn search(&self, query: &str) -> Result<CampusMapSearch, CampusMapError> {
        let map = self.map()?.ok_or(CampusMapError::Unavailable)?;
        let english = self.preferences()?.english;
        Ok(CampusMapSearch {
            query: query.to_owned(),
            results: page::search(&map, english, query),
        })
    }

    /// Takes the id of a new page, dropping the current one. Called when
    /// the shell asks to open, before the file is loaded, so opens and
    /// closes take effect in the order the shell made them.
    pub fn reserve(&self) -> u64 {
        let mut open = self.page.lock().unwrap_or_else(PoisonError::into_inner);
        open.last += 1;
        open.page = None;
        open.last
    }

    /// Opens the reserved page on a room string, its first target chosen and
    /// the camera framed on it; on none, nothing selected and the main
    /// campus in view. A page a later open or close superseded is answered
    /// but never installed.
    pub fn open(&self, page: u64, room: Option<&str>) -> Result<CampusMapScene, CampusMapError> {
        let map = self.map()?.ok_or(CampusMapError::Unavailable)?;
        let mut opened = Page::open(map, self.preferences()?, room);
        let scene = opened.scene(page).unwrap_or_else(|| opened.render(page));
        let mut open = self.page.lock().unwrap_or_else(PoisonError::into_inner);
        if open.last == page {
            open.page = Some(opened);
        }
        Ok(scene)
    }

    /// Applies one intent to the page. `None` when nothing drawn changed,
    /// which most camera moves are, and for a page that is no longer open.
    pub fn send(&self, page: u64, intent: CampusMapIntent) -> Option<CampusMapScene> {
        let mut open = self.page.lock().unwrap_or_else(PoisonError::into_inner);
        if open.last != page {
            return None;
        }
        let current = open.page.as_mut()?;
        current.apply(intent);
        current.scene(page)
    }

    /// Drops the page when the shell no longer shows it, and any open still
    /// loading.
    pub fn close(&self) {
        let mut open = self.page.lock().unwrap_or_else(PoisonError::into_inner);
        open.last += 1;
        open.page = None;
    }
}
