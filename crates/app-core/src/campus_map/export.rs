//! The Isct-Data campus map export, format 2, as published: gzip of one JSON
//! object. Only this format is read; any other `formatVersion` is refused
//! before its fields are looked at.
use super::{CampusMapError, FORMAT_VERSION, MAX_JSON_BYTES};
use crate::locale::Localized;
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
    /// The badge colour class (slotledger `approach/campus-map-icons`).
    pub(super) fn tone(self) -> super::CampusMapAmenityTone {
        use super::CampusMapAmenityTone as Tone;
        match self {
            Self::Aed => Tone::Aed,
            Self::Restroom
            | Self::RestroomMale
            | Self::RestroomFemale
            | Self::RestroomWheelchair => Tone::Restroom,
            Self::Lounge
            | Self::LoungeWomen
            | Self::LoungeInternational
            | Self::RestRoom
            | Self::CalmDown
            | Self::StudyRoom => Tone::Stay,
            Self::Elevator | Self::Stairs => Tone::Passage,
        }
    }

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
    pub(crate) format_version: u32,
    pub(crate) attribution: Vec<String>,
    pub(crate) buildings: Vec<Building>,
    pub units: Vec<Unit>,
    pub(crate) amenities: Vec<Amenity>,
    pub(crate) linework: Vec<Linework>,
    pub(crate) rooms: Vec<Room>,
    pub(crate) aliases: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Building {
    pub id: String,
    pub campus: String,
    pub name: Localized,
    pub(crate) codes: Vec<String>,
    pub center: Position,
    /// MultiPolygon coordinates.
    pub(crate) footprint: Vec<Rings>,
    pub(crate) levels: Vec<Level>,
    /// The floor the main entrance is on, one of `levels`; null only for a
    /// building with no floors.
    pub(crate) street_level_id: Option<String>,
    pub(crate) entrances: Vec<Entrance>,
    /// The maintainers marked the building's data complete: the map asks
    /// for no more photos of it. Not in the published format 2, which is
    /// frozen for the builds that read it (Isct-Data
    /// `docs/campus-map-export.md`, "Versioning"); it arrives with the next
    /// format, and this build reads it when present.
    #[serde(default)]
    pub complete: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Level {
    pub id: String,
    pub(crate) ordinal: i32,
    pub name: String,
    pub outline: Vec<Rings>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Entrance {
    pub id: String,
    pub point: Position,
    #[serde(default)]
    pub name: Option<Localized>,
    #[serde(default)]
    pub(crate) level_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Unit {
    pub id: String,
    pub(crate) level_id: String,
    pub(crate) category: UnitCategory,
    #[serde(default)]
    pub(crate) room_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    pub center: Position,
    pub(crate) polygon: Rings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Amenity {
    pub id: String,
    pub(crate) level_id: String,
    pub(crate) category: AmenityCategory,
    pub point: Position,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Linework {
    pub(crate) level_id: String,
    pub(crate) tone: LineTone,
    /// The line's physical width, in metres.
    pub(crate) width_m: f64,
    /// MultiLineString coordinates.
    pub lines: Vec<Vec<Position>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Room {
    pub id: String,
    pub code: String,
    pub building_id: String,
    pub(crate) unit_ids: Vec<String>,
}

/// Only the version, read before the strict parse so a later format is
/// refused as a format, not as malformed v2.
#[derive(Deserialize)]
struct Version {
    #[serde(rename = "formatVersion")]
    format_version: Option<serde_json::Value>,
}

/// Unpacks the file (at most [`MAX_JSON_BYTES`]) and parses format 2.
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

/// Parses the unpacked JSON of format 2.
fn parse(json: &[u8]) -> Result<Export, CampusMapError> {
    let version: Version =
        serde_json::from_slice(json).map_err(|error| CampusMapError::Invalid(error.to_string()))?;
    match version.format_version {
        Some(serde_json::Value::Number(number))
            if number.as_u64() == Some(u64::from(FORMAT_VERSION)) => {}
        Some(other) => return Err(CampusMapError::Format(other.to_string())),
        None => return Err(CampusMapError::Format("none".into())),
    }
    serde_json::from_slice(json).map_err(|error| CampusMapError::Invalid(error.to_string()))
}
