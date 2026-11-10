//! One parsed export with its indexes, and the rules over it: which room or
//! building a room string names, which building and floor the camera shows,
//! and what a tap hits.
use super::{
    export::{Export, Position, Rings},
    geometry,
    matching::{alias_key, Mention, Patterns},
    CampusMapAmenity, CampusMapBounds, CampusMapBuilding, CampusMapCamera, CampusMapEntrance,
    CampusMapError, CampusMapFloor, CampusMapFocus, CampusMapLevel, CampusMapLines,
    CampusMapMatch, CampusMapOverview, CampusMapPoint, CampusMapPolygon, CampusMapTarget,
    CampusMapUnit,
};
use std::collections::{HashMap, HashSet};

/// Below this zoom level no building is in focus: the whole campus shows.
const FOCUS_ZOOM: f64 = 17.0;
/// The camera centre this close outside a footprint still picks it.
const FOCUS_MARGIN_M: f64 = 10.0;

pub(super) struct Map {
    export: Export,
    patterns: Patterns,
    buildings: HashMap<String, usize>,
    /// Level id → (building, level), indexes into the export.
    levels: HashMap<String, (usize, usize)>,
    units: HashMap<String, usize>,
    units_by_level: HashMap<String, Vec<usize>>,
    amenities_by_level: HashMap<String, Vec<usize>>,
    linework_by_level: HashMap<String, Vec<usize>>,
    rooms: HashMap<String, usize>,
    /// Footprint area per building, square metres.
    areas: Vec<f64>,
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

fn point(position: Position) -> CampusMapPoint {
    CampusMapPoint {
        latitude: position[1],
        longitude: position[0],
    }
}

fn polygon(rings: &Rings) -> CampusMapPolygon {
    CampusMapPolygon {
        rings: rings
            .iter()
            .map(|ring| ring.iter().copied().map(point).collect())
            .collect(),
    }
}

fn rect_bounds(rect: geometry::Rect) -> CampusMapBounds {
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
fn zoom(camera: &CampusMapCamera) -> f64 {
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
            if !levels.contains_key(&lines.level_id) || !lines.width_m.is_finite() {
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

    fn level_models(&self, building: usize) -> Vec<CampusMapLevel> {
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
    fn ground_level(&self, building: usize) -> Option<usize> {
        let levels = &self.export.buildings[building].levels;
        (0..levels.len()).min_by_key(|index| {
            let ordinal = levels[*index].ordinal;
            (ordinal.unsigned_abs(), ordinal < 0)
        })
    }

    fn name(&self, building: usize, english: bool) -> String {
        self.export.buildings[building]
            .name
            .pick(english)
            .to_owned()
    }

    fn building_model(&self, index: usize, english: bool) -> CampusMapBuilding {
        let building = &self.export.buildings[index];
        let name = self.name(index, english);
        CampusMapBuilding {
            id: building.id.clone(),
            codes: building.codes.clone(),
            center: point(building.center),
            footprint: building.footprint.iter().map(polygon).collect(),
            levels: self.level_models(index),
            entrances: building
                .entrances
                .iter()
                .map(|entrance| {
                    let entrance_name = entrance
                        .name
                        .as_ref()
                        .map(|label| label.pick(english).to_owned());
                    let title = match &entrance_name {
                        Some(label) => format!("{name} {label}"),
                        None => name.clone(),
                    };
                    CampusMapEntrance {
                        id: entrance.id.clone(),
                        name: entrance_name,
                        point: point(entrance.point),
                        level_id: entrance.level_id.clone(),
                        maps_url: maps_url(entrance.point, &title),
                    }
                })
                .collect(),
            maps_url: maps_url(building.center, &name),
            name,
        }
    }

    pub(super) fn overview(&self, english: bool) -> CampusMapOverview {
        CampusMapOverview {
            buildings: (0..self.export.buildings.len())
                .map(|index| self.building_model(index, english))
                .collect(),
            attribution: self.export.attribution.clone(),
        }
    }

    fn unit_model(&self, index: usize) -> CampusMapUnit {
        let unit = &self.export.units[index];
        CampusMapUnit {
            id: unit.id.clone(),
            category: unit.category,
            label: unit.label.clone(),
            center: point(unit.center),
            polygon: polygon(&unit.polygon),
            room_id: unit.room_id.clone(),
        }
    }

    pub(super) fn floor(&self, level_id: &str) -> Option<CampusMapFloor> {
        let (building, level) = *self.levels.get(level_id)?;
        let building = &self.export.buildings[building];
        let on_level = |index: &HashMap<String, Vec<usize>>| -> Vec<usize> {
            index.get(level_id).cloned().unwrap_or_default()
        };
        Some(CampusMapFloor {
            level_id: level_id.to_owned(),
            building_id: building.id.clone(),
            outline: building.levels[level].outline.iter().map(polygon).collect(),
            units: on_level(&self.units_by_level)
                .into_iter()
                .map(|index| self.unit_model(index))
                .collect(),
            lines: on_level(&self.linework_by_level)
                .into_iter()
                .map(|index| {
                    let lines = &self.export.linework[index];
                    CampusMapLines {
                        tone: lines.tone,
                        width_m: lines.width_m,
                        lines: lines
                            .lines
                            .iter()
                            .map(|line| line.iter().copied().map(point).collect())
                            .collect(),
                    }
                })
                .collect(),
            amenities: on_level(&self.amenities_by_level)
                .into_iter()
                .map(|index| {
                    let amenity = &self.export.amenities[index];
                    CampusMapAmenity {
                        id: amenity.id.clone(),
                        category: amenity.category,
                        point: point(amenity.point),
                    }
                })
                .collect(),
        })
    }

    /// The smallest unit with a name under `at`: a room inside a corridor
    /// polygon wins over the corridor.
    pub(super) fn unit_at(&self, level_id: &str, at: CampusMapPoint) -> Option<CampusMapUnit> {
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
            .map(|(index, _)| self.unit_model(index))
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
    /// ground floor, else the floor nearest the ground.
    pub(super) fn focus(&self, camera: &CampusMapCamera) -> CampusMapFocus {
        let current = camera
            .building_id
            .as_deref()
            .and_then(|id| self.buildings.get(id).copied())
            .filter(|index| !self.export.buildings[*index].levels.is_empty());
        let current_level = camera
            .level_id
            .as_deref()
            .and_then(|id| self.levels.get(id).copied())
            .filter(|(building, _)| Some(*building) == current)
            .map(|(_, level)| level);
        let center = [camera.center.longitude, camera.center.latitude];
        let view = [
            camera.bounds.west,
            camera.bounds.south,
            camera.bounds.east,
            camera.bounds.north,
        ];
        let building = if !camera.by_user {
            current
        } else if zoom(camera) < FOCUS_ZOOM {
            None
        } else {
            self.building_at(center).or_else(|| {
                current.filter(|index| {
                    geometry::multipolygon_meets_rect(&self.export.buildings[*index].footprint, view)
                })
            })
        };
        let level = building.and_then(|index| {
            if Some(index) == current {
                return current_level.or_else(|| self.ground_level(index));
            }
            let ordinal = current
                .zip(current_level)
                .map(|(from, level)| self.export.buildings[from].levels[level].ordinal);
            ordinal
                .and_then(|ordinal| {
                    self.export.buildings[index]
                        .levels
                        .iter()
                        .position(|level| level.ordinal == ordinal)
                })
                .or_else(|| self.ground_level(index))
        });
        CampusMapFocus {
            building_id: building.map(|index| self.export.buildings[index].id.clone()),
            levels: building
                .map(|index| self.level_models(index))
                .unwrap_or_default(),
            level_id: building
                .zip(level)
                .map(|(index, level)| self.export.buildings[index].levels[level].id.clone()),
        }
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
    pub(super) fn locate(&self, raw: &str, english: bool) -> Vec<CampusMapTarget> {
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

    fn target(&self, place: Place, english: bool) -> Option<CampusMapTarget> {
        let (building, label, room_id, units, hint) = match place {
            Place::Room { room, level } => {
                let room = &self.export.rooms[room];
                let building = *self.buildings.get(&room.building_id)?;
                let units: Vec<usize> = room
                    .unit_ids
                    .iter()
                    .filter_map(|id| self.units.get(id).copied())
                    .collect();
                (building, room.code.clone(), Some(room.id.clone()), units, level)
            }
            Place::Building {
                building,
                label,
                level,
            } => (building, label, None, Vec::new(), level),
        };
        let levels = &self.export.buildings[building].levels;
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
        Some(CampusMapTarget {
            label,
            matched: if highlighted.is_empty() {
                CampusMapMatch::Building
            } else {
                CampusMapMatch::Room
            },
            room_id,
            building_id: self.export.buildings[building].id.clone(),
            building_name: self.name(building, english),
            levels: self.level_models(building),
            level_id: level.map(|level| levels[level].id.clone()),
            highlighted_unit_ids: highlighted
                .iter()
                .map(|unit| self.export.units[*unit].id.clone())
                .collect(),
            focus: rect_bounds(focus),
        })
    }
}
