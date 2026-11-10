//! One parsed export with its indexes, and the rules over it: which room or
//! building a room string names, which building and floor the camera shows,
//! and what a tap hits. Buildings, levels and units are export indexes here;
//! a level is the index into its building's `levels`.
use super::{
    export::{Export, Position, Rings},
    geometry,
    matching::{alias_key, Mention, Patterns},
    CampusMapBounds, CampusMapCamera, CampusMapError, CampusMapLevel, CampusMapPoint,
    CampusMapPolygon,
};
use std::collections::{HashMap, HashSet};

/// Below this zoom level no building is in focus: the whole campus shows.
const FOCUS_ZOOM: f64 = 17.0;
/// The camera centre this close outside a footprint still picks it.
const FOCUS_MARGIN_M: f64 = 10.0;

pub(super) struct Map {
    pub(super) export: Export,
    patterns: Patterns,
    pub(super) buildings: HashMap<String, usize>,
    /// Level id → (building, level).
    pub(super) levels: HashMap<String, (usize, usize)>,
    pub(super) units: HashMap<String, usize>,
    pub(super) units_by_level: HashMap<String, Vec<usize>>,
    pub(super) amenities_by_level: HashMap<String, Vec<usize>>,
    pub(super) linework_by_level: HashMap<String, Vec<usize>>,
    rooms: HashMap<String, usize>,
    /// Footprint area per building, square metres.
    areas: Vec<f64>,
}

/// How much of a room string the map knows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Match {
    /// The room has units on a floor plan, which are highlighted.
    Room,
    /// Only the building is known (or the room has no unit drawn).
    Building,
}

/// One room or building a room string names: where the map page opens.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Target {
    /// The room's code, or the words of the string when only the building
    /// is known.
    pub(super) label: String,
    pub(super) matched: Match,
    pub(super) building: usize,
    pub(super) building_name: String,
    /// The floor shown on entry: the highlighted room's; `None` for a
    /// building without floor plans.
    pub(super) level: Option<usize>,
    /// Every unit of the room on the floors it is on.
    pub(super) units: Vec<usize>,
    /// Where the camera goes on entry: the room on its floor, else the
    /// building.
    pub(super) focus: CampusMapBounds,
}

/// A place a room string names, before it becomes a target.
enum Place {
    Room {
        room: usize,
        level: Option<String>,
    },
    Building {
        building: usize,
        label: String,
        level: Option<String>,
    },
}

fn invalid(what: impl std::fmt::Display) -> CampusMapError {
    CampusMapError::Invalid(what.to_string())
}

pub(super) fn point(position: Position) -> CampusMapPoint {
    CampusMapPoint {
        latitude: position[1],
        longitude: position[0],
    }
}

pub(super) fn polygon(rings: &Rings) -> CampusMapPolygon {
    CampusMapPolygon {
        rings: rings
            .iter()
            .map(|ring| ring.iter().copied().map(point).collect())
            .collect(),
    }
}

pub(super) fn rect_bounds(rect: geometry::Rect) -> CampusMapBounds {
    CampusMapBounds {
        west: rect[0],
        south: rect[1],
        east: rect[2],
        north: rect[3],
    }
}

/// A pin in Apple Maps at `position`, titled `label`.
pub(super) fn maps_url(position: Position, label: &str) -> String {
    url::Url::parse_with_params(
        "https://maps.apple.com/",
        [
            ("ll", format!("{:.7},{:.7}", position[1], position[0])),
            ("q", label.to_owned()),
        ],
    )
    .map(|url| url.to_string())
    .unwrap_or_default()
}

/// The zoom level a camera shows: 256-point tiles across `width_points`.
pub(super) fn zoom(camera: &CampusMapCamera) -> f64 {
    let span = camera.bounds.east - camera.bounds.west;
    let ratio = 360.0 * camera.width_points / (span * 256.0);
    if span > 0.0 && ratio.is_finite() && ratio > 0.0 {
        ratio.log2()
    } else {
        0.0
    }
}

impl Map {
    /// Indexes the export and checks that every reference resolves, so no
    /// read below meets a dangling id.
    pub(super) fn new(export: Export) -> Result<Self, CampusMapError> {
        let mut buildings = HashMap::new();
        let mut levels = HashMap::new();
        let mut entrances = HashSet::new();
        for (index, building) in export.buildings.iter().enumerate() {
            if !matches!(building.campus.as_str(), "ookayama" | "suzukakedai") {
                return Err(invalid(format!("building {} has campus {}", building.id, building.campus)));
            }
            if buildings.insert(building.id.clone(), index).is_some() {
                return Err(invalid(format!("building {} is listed twice", building.id)));
            }
            for (level_index, level) in building.levels.iter().enumerate() {
                if levels.insert(level.id.clone(), (index, level_index)).is_some() {
                    return Err(invalid(format!("level {} is listed twice", level.id)));
                }
            }
        }
        for building in &export.buildings {
            for entrance in &building.entrances {
                if !entrances.insert(entrance.id.as_str()) {
                    return Err(invalid(format!("entrance {} is listed twice", entrance.id)));
                }
                if entrance
                    .level_id
                    .as_ref()
                    .is_some_and(|level| !levels.contains_key(level))
                {
                    return Err(invalid(format!("entrance {} has no level", entrance.id)));
                }
            }
        }
        let mut rooms = HashMap::new();
        for (index, room) in export.rooms.iter().enumerate() {
            if !buildings.contains_key(&room.building_id) {
                return Err(invalid(format!("room {} has no building", room.id)));
            }
            if rooms.insert(room.id.clone(), index).is_some() {
                return Err(invalid(format!("room {} is listed twice", room.id)));
            }
        }
        let mut units = HashMap::new();
        let mut units_by_level: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, unit) in export.units.iter().enumerate() {
            if units.insert(unit.id.clone(), index).is_some() {
                return Err(invalid(format!("unit {} is listed twice", unit.id)));
            }
            if !levels.contains_key(&unit.level_id)
                || unit
                    .room_id
                    .as_ref()
                    .is_some_and(|room| !rooms.contains_key(room))
            {
                return Err(invalid(format!("unit {} has a dangling reference", unit.id)));
            }
            units_by_level
                .entry(unit.level_id.clone())
                .or_default()
                .push(index);
        }
        for room in &export.rooms {
            if room.unit_ids.iter().any(|unit| !units.contains_key(unit)) {
                return Err(invalid(format!("room {} names a missing unit", room.id)));
            }
        }
        let mut amenities_by_level: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, amenity) in export.amenities.iter().enumerate() {
            if !levels.contains_key(&amenity.level_id) {
                return Err(invalid(format!("amenity {} has no level", amenity.id)));
            }
            amenities_by_level
                .entry(amenity.level_id.clone())
                .or_default()
                .push(index);
        }
        let mut linework_by_level: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, lines) in export.linework.iter().enumerate() {
            if !levels.contains_key(&lines.level_id) {
                return Err(invalid(format!("linework of level {} is invalid", lines.level_id)));
            }
            linework_by_level
                .entry(lines.level_id.clone())
                .or_default()
                .push(index);
        }
        for (spelling, targets) in &export.aliases {
            if targets.is_empty() || targets.iter().any(|room| !rooms.contains_key(room)) {
                return Err(invalid(format!("alias {spelling} names a missing room")));
            }
        }
        let areas = export
            .buildings
            .iter()
            .map(|building| geometry::multipolygon_area(&building.footprint))
            .collect();
        Ok(Self {
            patterns: Patterns::new().map_err(invalid)?,
            export,
            buildings,
            levels,
            units,
            units_by_level,
            amenities_by_level,
            linework_by_level,
            rooms,
            areas,
        })
    }

    /// Units the file publishes: the rows a suspicious shrink counts.
    pub(super) fn unit_count(&self) -> usize {
        self.export.units.len()
    }

    /// A building's levels, lowest first.
    fn level_order(&self, building: usize) -> Vec<usize> {
        let levels = &self.export.buildings[building].levels;
        let mut order: Vec<usize> = (0..levels.len()).collect();
        order.sort_by_key(|index| levels[*index].ordinal);
        order
    }

    pub(super) fn level_models(&self, building: usize) -> Vec<CampusMapLevel> {
        let levels = &self.export.buildings[building].levels;
        self.level_order(building)
            .into_iter()
            .map(|index| CampusMapLevel {
                id: levels[index].id.clone(),
                ordinal: levels[index].ordinal,
                name: levels[index].name.clone(),
            })
            .collect()
    }

    /// The ground floor, else the floor nearest the ground, above it on a
    /// tie.
    pub(super) fn ground_level(&self, building: usize) -> Option<usize> {
        let levels = &self.export.buildings[building].levels;
        (0..levels.len()).min_by_key(|index| {
            let ordinal = levels[*index].ordinal;
            (ordinal.unsigned_abs(), ordinal < 0)
        })
    }

    pub(super) fn name(&self, building: usize, english: bool) -> String {
        self.export.buildings[building]
            .name
            .pick(english)
            .to_owned()
    }

    /// The smallest unit with a name under `at` on a building's floor: a
    /// room inside a corridor polygon wins over the corridor.
    pub(super) fn unit_at(&self, building: usize, level: usize, at: CampusMapPoint) -> Option<usize> {
        let level_id = &self.export.buildings[building].levels[level].id;
        let at = [at.longitude, at.latitude];
        self.units_by_level
            .get(level_id)?
            .iter()
            .copied()
            .filter(|index| {
                let unit = &self.export.units[*index];
                (unit.label.is_some() || unit.room_id.is_some())
                    && geometry::polygon_contains(&unit.polygon, at)
            })
            .map(|index| (index, geometry::polygon_area(&self.export.units[index].polygon)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// The building with floor plans whose footprint, grown by
    /// [`FOCUS_MARGIN_M`], holds `at`; the smallest where several do.
    fn building_at(&self, at: Position) -> Option<usize> {
        self.export
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, building)| !building.levels.is_empty())
            .filter(|(_, building)| {
                geometry::multipolygon_contains(&building.footprint, at)
                    || geometry::distance_to_edges(&building.footprint, at) <= FOCUS_MARGIN_M
            })
            .map(|(index, _)| index)
            .min_by(|a, b| self.areas[*a].total_cmp(&self.areas[*b]))
    }

    /// The focus rule: below zoom 17 nothing; the building under the camera
    /// centre (smallest first); else the current one while it is still in
    /// view. A move the app made keeps what is shown. On a building switch
    /// the floor keeps its ordinal when the new building has it, else the
    /// ground floor, else the floor nearest the ground. `current` is the
    /// building and floor shown before the move; so is the answer.
    pub(super) fn focus(
        &self,
        camera: &CampusMapCamera,
        current: Option<(usize, usize)>,
    ) -> Option<(usize, usize)> {
        let current = current.filter(|(building, level)| {
            *level < self.export.buildings[*building].levels.len()
        });
        let center = [camera.center.longitude, camera.center.latitude];
        let view = [
            camera.bounds.west,
            camera.bounds.south,
            camera.bounds.east,
            camera.bounds.north,
        ];
        if !camera.by_user {
            return current;
        }
        if zoom(camera) < FOCUS_ZOOM {
            return None;
        }
        let building = self.building_at(center).or_else(|| {
            current.map(|(building, _)| building).filter(|index| {
                geometry::multipolygon_meets_rect(&self.export.buildings[*index].footprint, view)
            })
        })?;
        if let Some((shown, level)) = current.filter(|(shown, _)| *shown == building) {
            return Some((shown, level));
        }
        let ordinal = current.map(|(from, level)| self.export.buildings[from].levels[level].ordinal);
        ordinal
            .and_then(|ordinal| {
                self.export.buildings[building]
                    .levels
                    .iter()
                    .position(|level| level.ordinal == ordinal)
            })
            .or_else(|| self.ground_level(building))
            .map(|level| (building, level))
    }

    /// The rooms an alias spelling names, in the export's order.
    fn alias(&self, spelling: &str) -> Option<Vec<usize>> {
        let rooms: Vec<usize> = self
            .export
            .aliases
            .get(&alias_key(spelling))?
            .iter()
            .filter_map(|id| self.rooms.get(id).copied())
            .collect();
        (!rooms.is_empty()).then_some(rooms)
    }

    fn room_building(&self, room: usize) -> Option<usize> {
        self.buildings
            .get(&self.export.rooms[room].building_id)
            .copied()
    }

    /// One mention: a room by its alias (the parsed code first, then the
    /// mention's words), else its building by code, else by name.
    fn resolve(&self, mention: &Mention, places: &mut Vec<Place>) {
        let rooms = mention
            .room
            .as_deref()
            .and_then(|room| self.alias(room))
            .or_else(|| self.alias(&mention.text));
        if let Some(rooms) = rooms {
            places.extend(rooms.into_iter().map(|room| Place::Room {
                room,
                level: mention.level.clone(),
            }));
            return;
        }
        let label = mention.room.clone().unwrap_or_else(|| mention.text.clone());
        let building = |building: usize| Place::Building {
            building,
            label: label.clone(),
            level: mention.level.clone(),
        };
        if let Some(code) = &mention.building {
            let coded: Vec<usize> = (0..self.export.buildings.len())
                .filter(|index| self.export.buildings[*index].codes.contains(code))
                .collect();
            if !coded.is_empty() {
                places.extend(coded.into_iter().map(building));
                return;
            }
        }
        let words = alias_key(&mention.text);
        for (index, candidate) in self.export.buildings.iter().enumerate() {
            let named = std::iter::once(candidate.name.ja.as_str())
                .chain(candidate.name.en.as_deref())
                .map(alias_key)
                .any(|name| name.chars().count() >= 2 && words.contains(&name));
            if named {
                places.push(building(index));
            }
        }
    }

    /// What a room string points at, in the string's order. A spelling the
    /// alias table knows whole wins; otherwise each mention the parser finds.
    /// A building already shown for a room gets no building-only row.
    pub(super) fn locate(&self, raw: &str, english: bool) -> Vec<Target> {
        let mut places = Vec::new();
        match self.alias(raw) {
            Some(rooms) => places.extend(rooms.into_iter().map(|room| Place::Room { room, level: None })),
            None => {
                for mention in self.patterns.mentions(raw) {
                    self.resolve(&mention, &mut places);
                }
            }
        }
        let with_room: HashSet<usize> = places
            .iter()
            .filter_map(|place| match place {
                Place::Room { room, .. } => self.room_building(*room),
                Place::Building { .. } => None,
            })
            .collect();
        let mut rooms = HashSet::new();
        let mut buildings = HashSet::new();
        places.retain(|place| match place {
            Place::Room { room, .. } => rooms.insert(*room),
            Place::Building { building, .. } => {
                !with_room.contains(building) && buildings.insert(*building)
            }
        });
        places
            .into_iter()
            .filter_map(|place| self.target(place, english))
            .collect()
    }

    /// The level named `name` in `building`, if any.
    fn named_level(&self, building: usize, name: Option<&str>) -> Option<usize> {
        let name = name?;
        self.export.buildings[building]
            .levels
            .iter()
            .position(|level| level.name == name)
    }

    fn target(&self, place: Place, english: bool) -> Option<Target> {
        let (building, label, units, hint) = match place {
            Place::Room { room, level } => {
                let room = &self.export.rooms[room];
                let building = *self.buildings.get(&room.building_id)?;
                let units: Vec<usize> = room
                    .unit_ids
                    .iter()
                    .filter_map(|id| self.units.get(id).copied())
                    .collect();
                (building, room.code.clone(), units, level)
            }
            Place::Building {
                building,
                label,
                level,
            } => (building, label, Vec::new(), level),
        };
        let level_of = |unit: usize| self.levels.get(&self.export.units[unit].level_id).map(|(_, level)| *level);
        let hinted = self.named_level(building, hint.as_deref());
        // A floor the string names narrows a room spread over several.
        let on_hint: Vec<usize> = units
            .iter()
            .copied()
            .filter(|unit| hinted.is_some() && level_of(*unit) == hinted)
            .collect();
        let highlighted = if on_hint.is_empty() { units } else { on_hint };
        // On entry the highlighted room's floor wins: the lowest it is on.
        let level = self
            .level_order(building)
            .into_iter()
            .find(|level| highlighted.iter().any(|unit| level_of(*unit) == Some(*level)))
            .or(hinted)
            .or_else(|| self.ground_level(building));
        let shown: Vec<&Rings> = highlighted
            .iter()
            .filter(|unit| level.is_some() && level_of(**unit) == level)
            .map(|unit| &self.export.units[*unit].polygon)
            .collect();
        let footprint = &self.export.buildings[building].footprint;
        let center = self.export.buildings[building].center;
        let focus = geometry::bounds(shown.into_iter().flatten())
            .or_else(|| geometry::bounds(footprint.iter().flatten()))
            .unwrap_or([center[0], center[1], center[0], center[1]]);
        Some(Target {
            label,
            matched: if highlighted.is_empty() {
                Match::Building
            } else {
                Match::Room
            },
            building,
            building_name: self.name(building, english),
            level,
            units: highlighted,
            focus: rect_bounds(focus),
        })
    }
}
