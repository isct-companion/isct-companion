//! The Isct-Data campus map export, format 1, as published: gzip of one JSON
//! object. Only this format is read; any other `formatVersion` is refused
//! before its fields are looked at.
use super::{CampusMapError, FORMAT_VERSION, MAX_JSON_BYTES};
use serde::Deserialize;
use std::{collections::BTreeMap, io::Read};

/// A unit's kind, the export's full list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UnitCategory {
    Classroom,
    Room,
    Corridor,
    ElevatorLobby,
    Restroom,
    Stairs,
    Lounge,
    Porch,
}

/// An amenity's kind, the export's full list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
pub(super) enum AmenityCategory {
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

impl AmenityCategory {
    /// The key shells name an amenity's icon and text by: the iOS asset
    /// `CampusMap/<key>` and the catalog key `campusMap.amenity.<key>`.
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Elevator => "elevator",
            Self::Stairs => "stairs",
            Self::Restroom => "restroom",
            Self::RestroomMale => "restroom-male",
            Self::RestroomFemale => "restroom-female",
            Self::RestroomWheelchair => "restroom-accessible",
            Self::Aed => "aed",
            Self::Lounge => "lounge",
            Self::LoungeWomen => "lounge-women",
            Self::LoungeInternational => "lounge-international",
            Self::RestRoom => "rest-room",
            Self::CalmDown => "calm-down",
            Self::StudyRoom => "study-room",
        }
    }
}

/// A drawn line's role on the source plans.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum LineTone {
    /// Black on the source plan: walls.
    Wall,
    /// Grey on the source plan: fittings and hairlines.
    Detail,
}

/// `[lng, lat]`.
pub(super) type Position = [f64; 2];
/// GeoJSON Polygon coordinates: the outer ring, then holes.
pub(super) type Rings = Vec<Vec<Position>>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Export {
    #[allow(dead_code)]
    pub format_version: u32,
    pub attribution: Vec<String>,
    pub buildings: Vec<Building>,
    pub units: Vec<Unit>,
    pub amenities: Vec<Amenity>,
    pub linework: Vec<Linework>,
    pub rooms: Vec<Room>,
    pub aliases: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Name {
    pub ja: String,
    #[serde(default)]
    pub en: Option<String>,
}

impl Name {
    pub(super) fn pick(&self, english: bool) -> &str {
        match (&self.en, english) {
            (Some(en), true) if !en.trim().is_empty() => en,
            _ => &self.ja,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Building {
    pub id: String,
    pub campus: String,
    pub name: Name,
    pub codes: Vec<String>,
    pub center: Position,
    /// MultiPolygon coordinates.
    pub footprint: Vec<Rings>,
    pub levels: Vec<Level>,
    pub entrances: Vec<Entrance>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Level {
    pub id: String,
    pub ordinal: i32,
    pub name: String,
    pub outline: Vec<Rings>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Entrance {
    pub id: String,
    pub point: Position,
    #[serde(default)]
    pub name: Option<Name>,
    #[serde(default)]
    pub level_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Unit {
    pub id: String,
    pub level_id: String,
    pub category: UnitCategory,
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    pub center: Position,
    pub polygon: Rings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Amenity {
    pub id: String,
    pub level_id: String,
    pub category: AmenityCategory,
    pub point: Position,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Linework {
    pub level_id: String,
    pub tone: LineTone,
    /// The line's physical width, in metres.
    pub width_m: f64,
    /// MultiLineString coordinates.
    pub lines: Vec<Vec<Position>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Room {
    pub id: String,
    pub code: String,
    pub building_id: String,
    pub unit_ids: Vec<String>,
}

/// Only the version, read before the strict parse so a later format is
/// refused as a format, not as malformed v1.
#[derive(Deserialize)]
struct Version {
    #[serde(rename = "formatVersion")]
    format_version: Option<serde_json::Value>,
}

/// Unpacks the file (at most [`MAX_JSON_BYTES`]) and parses format 1.
pub(super) fn read(gz: &[u8]) -> Result<Export, CampusMapError> {
    let mut json = Vec::new();
    flate2::read::GzDecoder::new(gz)
        .take(MAX_JSON_BYTES as u64 + 1)
        .read_to_end(&mut json)
        .map_err(|error| CampusMapError::Invalid(format!("not gzip: {error}")))?;
    if json.len() > MAX_JSON_BYTES {
        return Err(CampusMapError::Invalid(
            "the unpacked file exceeds the size limit".into(),
        ));
    }
    parse(&json)
}

/// Parses the unpacked JSON of format 1.
fn parse(json: &[u8]) -> Result<Export, CampusMapError> {
    let version: Version = serde_json::from_slice(json)
        .map_err(|error| CampusMapError::Invalid(error.to_string()))?;
    match version.format_version {
        Some(serde_json::Value::Number(number))
            if number.as_u64() == Some(u64::from(FORMAT_VERSION)) => {}
        Some(other) => return Err(CampusMapError::Format(other.to_string())),
        None => return Err(CampusMapError::Format("none".into())),
    }
    serde_json::from_slice(json).map_err(|error| CampusMapError::Invalid(error.to_string()))
}
