//! One parsed export with its indexes, and the rules over it: which room or
//! building a room string names, which building and floor the camera shows,
//! and what a tap hits. Buildings, levels and units are export indexes here;
//! a level is the index into its building's `levels`.
use super::{
    export::{Export, Position, Rings},
    geometry::{self, Viewport},
    matching::{alias_key, Mention, Patterns},
    search::{Found, Index},
    CampusMapBounds, CampusMapError, CampusMapLevel, CampusMapPoint, CampusMapPolygon,
};
use std::collections::{HashMap, HashSet};

/// A building comes into focus once it spans this share of the viewport
/// (`Viewport::share`) with the viewport's centre inside its footprint.
/// Starting points, to be tuned on a device.
pub(super) const FOCUS_ENTER_SHARE: f64 = 0.5;
/// The building in focus stays until it spans less than this share or
/// leaves the viewport: the gap keeps a small zoom from flickering.
pub(super) const FOCUS_EXIT_SHARE: f64 = 1.0 / 3.0;
/// A tap this many screen points outside a footprint still picks it, when
/// no footprint holds the tap.
pub(super) const TAP_SLOP_POINTS: f64 = 12.0;

/// Which maps app the card's links open in (the `mapsApp` setting).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MapsApp {
    Apple,
    Google,
}

impl MapsApp {
    /// What an absent or unknown setting reads as: Apple Maps, and on
    /// Android, where Apple has no app, Google Maps.
    #[cfg(not(target_os = "android"))]
    const DEFAULT: Self = Self::Apple;
    #[cfg(target_os = "android")]
    const DEFAULT: Self = Self::Google;

    pub(super) fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("apple") => Self::Apple,
            Some("google") => Self::Google,
            _ => Self::DEFAULT,
        }
    }
}

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
    pub(super) rooms: HashMap<String, usize>,
    /// Entrance id → (building, entrance).
    pub(super) entrances: HashMap<String, (usize, usize)>,
    /// Footprint area per building, square metres.
    areas: Vec<f64>,
    search: Index,
}

/// Buildings whose main entrance is not on 1F, by export id, with the
/// floor's ordinal (1F = 0): the west side of Ookayama is on a slope. The
/// west buildings' from the Science Tokyo accessibility map
/// (ssc.titech.ac.jp/amap), which names their main entrance; W9's and
/// J2・J3's checked by hand. Every other building is taken to be entered
/// on 1F.
const MAIN_ENTRANCE_FLOORS: &[(&str, i32)] = &[
    // 大岡山西2号館 and 西3号館 share one main entrance.
    ("8b4ebd3a-63d4-5cc0-8d7d-e96828ecbd79", 2),
    ("0fd2e106-94b0-55fd-90b0-b01485426ecd", 2),
    // 大岡山西4号館
    ("9df4e073-cfbe-4522-a7f8-ca16a171557f", 2),
    // 大岡山西7号館
    ("8b59ffed-7f7e-417e-b3d3-21b326c26578", 2),
    // 大岡山西講義棟1, 西講義棟2
    ("95e63ef0-8ebc-5502-87ed-1f414753ce93", 2),
    ("dbf173b6-c53e-5d4c-92ca-eea95bac4ae0", 2),
    // 大岡山西8号館 (E) and (W), joined on the main entrance's floor.
    ("7eb7b2c0-8013-530a-8a7d-3efe2da6f375", 2),
    ("7b91641d-f38f-4489-b557-48b20c7fc378", 2),
    // 大岡山西9号館
    ("a10026fc-4e59-55a7-9d89-0193f1749620", 1),
    // J2・J3棟 (Suzukakedai), entered on 2F and 3F only.
    ("5d48b79d-6509-5b23-a0fd-8873883617ce", 1),
];

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

/// A pin at `position` in the chosen maps app: Apple's titled `label`,
/// Google's by its coordinates (its search link takes no title). Both are
/// plain https links, which open the app when it is installed.
pub(super) fn maps_url(app: MapsApp, position: Position, label: &str) -> String {
    let at = format!("{:.7},{:.7}", position[1], position[0]);
    let url = match app {
        MapsApp::Apple => url::Url::parse_with_params(
            "https://maps.apple.com/",
            [("ll", at), ("q", label.to_owned())],
        ),
        MapsApp::Google => url::Url::parse_with_params(
            "https://www.google.com/maps/search/",
            [("api", "1".to_owned()), ("query", at)],
        ),
    };
    url.map(|url| url.to_string()).unwrap_or_default()
}

impl Map {
    /// Indexes the export and checks that every reference resolves, so no
    /// read below meets a dangling id.
    pub(super) fn new(export: Export) -> Result<Self, CampusMapError> {
        let mut buildings = HashMap::new();
        let mut levels = HashMap::new();
        let mut entrances = HashMap::new();
        for (index, building) in export.buildings.iter().enumerate() {
            if !matches!(building.campus.as_str(), "ookayama" | "suzukakedai") {
                return Err(invalid(format!(
                    "building {} has campus {}",
                    building.id, building.campus
                )));
            }
            if buildings.insert(building.id.clone(), index).is_some() {
                return Err(invalid(format!("building {} is listed twice", building.id)));
            }
            for (level_index, level) in building.levels.iter().enumerate() {
                if levels
                    .insert(level.id.clone(), (index, level_index))
                    .is_some()
                {
                    return Err(invalid(format!("level {} is listed twice", level.id)));
                }
            }
        }
        for (index, building) in export.buildings.iter().enumerate() {
            for (number, entrance) in building.entrances.iter().enumerate() {
                if entrances
                    .insert(entrance.id.clone(), (index, number))
                    .is_some()
                {
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
                return Err(invalid(format!(
                    "unit {} has a dangling reference",
                    unit.id
                )));
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
                return Err(invalid(format!(
                    "linework of level {} is invalid",
                    lines.level_id
                )));
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
        let mut search = Index::default();
        for (index, building) in export.buildings.iter().enumerate() {
            for code in &building.codes {
                search.add_code(code, Found::Building(index));
            }
            search.add_alias(&building.name.ja, Found::Building(index));
            if let Some(en) = &building.name.en {
                search.add_alias(en, Found::Building(index));
            }
        }
        for (index, room) in export.rooms.iter().enumerate() {
            search.add_code(&room.code, Found::Room(index));
        }
        for (spelling, targets) in &export.aliases {
            for room in targets {
                if let Some(index) = rooms.get(room) {
                    search.add_alias(spelling, Found::Room(*index));
                }
            }
        }
        let patterns = Patterns::new(
            export
                .buildings
                .iter()
                .flat_map(|building| building.codes.iter().map(String::as_str)),
        )
        .map_err(invalid)?;
        Ok(Self {
            patterns,
            entrances,
            search,
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
        self.level_order(building)
            .into_iter()
            .map(|index| self.level_model(building, index))
            .collect()
    }

    pub(super) fn level_model(&self, building: usize, level: usize) -> CampusMapLevel {
        let level = &self.export.buildings[building].levels[level];
        CampusMapLevel {
            id: level.id.clone(),
            ordinal: level.ordinal,
            name: level.name.clone(),
            has_plan: true,
        }
    }

    /// Every ordinal from a building's lowest floor to its highest, lowest
    /// first, with the level of that ordinal where the export has a plan:
    /// W9's 1F to 7F, 4F to 6F without.
    pub(super) fn stack_levels(&self, building: usize) -> Vec<(i32, Option<usize>)> {
        let levels = &self.export.buildings[building].levels;
        let (Some(lowest), Some(highest)) = (
            levels.iter().map(|level| level.ordinal).min(),
            levels.iter().map(|level| level.ordinal).max(),
        ) else {
            return Vec::new();
        };
        (lowest..=highest)
            .map(|ordinal| {
                (
                    ordinal,
                    levels.iter().position(|level| level.ordinal == ordinal),
                )
            })
            .collect()
    }

    /// The floor that feels like the ground floor: the one the main
    /// entrance is on ([`MAIN_ENTRANCE_FLOORS`]), else 1F; for a building
    /// with no plan of that floor, the floor nearest it, above on a tie.
    pub(super) fn street_level(&self, building: usize) -> Option<usize> {
        let street = self.street_ordinal(building);
        let levels = &self.export.buildings[building].levels;
        (0..levels.len()).min_by_key(|index| {
            let offset = levels[*index].ordinal - street;
            (offset.unsigned_abs(), offset < 0)
        })
    }

    /// The main entrance's floor as an ordinal: the listed one, else 1F
    /// when the plans have it, else the floor with a plan nearest 1F (S6,
    /// East 1, entered on 2F, have plans of nothing lower).
    pub(super) fn street_ordinal(&self, building: usize) -> i32 {
        let record = &self.export.buildings[building];
        if let Some((_, ordinal)) = MAIN_ENTRANCE_FLOORS.iter().find(|(id, _)| *id == record.id) {
            return *ordinal;
        }
        record
            .levels
            .iter()
            .map(|level| level.ordinal)
            .min_by_key(|ordinal| (ordinal.unsigned_abs(), *ordinal < 0))
            .unwrap_or(0)
    }

    pub(super) fn name(&self, building: usize, english: bool) -> String {
        self.export.buildings[building]
            .name
            .pick(english)
            .to_owned()
    }

    /// The smallest unit with a name under `at` on a building's floor: a
    /// room inside a corridor polygon wins over the corridor.
    pub(super) fn unit_at(
        &self,
        building: usize,
        level: usize,
        at: CampusMapPoint,
    ) -> Option<usize> {
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
            .map(|index| {
                (
                    index,
                    geometry::polygon_area(&self.export.units[index].polygon),
                )
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    /// The building whose footprint holds `at`, the smallest where several
    /// do (one drawn inside another); no margin.
    pub(super) fn building_containing(&self, at: Position, with_plans: bool) -> Option<usize> {
        self.export
            .buildings
            .iter()
            .enumerate()
            .filter(|(_, building)| !with_plans || !building.levels.is_empty())
            .filter(|(_, building)| geometry::multipolygon_contains(&building.footprint, at))
            .map(|(index, _)| index)
            .min_by(|a, b| self.areas[*a].total_cmp(&self.areas[*b]))
    }

    /// What a tap on the base map picks: the building holding it, else the
    /// one whose edge is nearest within `slop_m`.
    pub(super) fn building_tapped(&self, at: Position, slop_m: f64) -> Option<usize> {
        self.building_containing(at, false).or_else(|| {
            self.export
                .buildings
                .iter()
                .enumerate()
                .map(|(index, building)| {
                    (index, geometry::distance_to_edges(&building.footprint, at))
                })
                .filter(|(_, distance)| *distance <= slop_m)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(index, _)| index)
        })
    }

    /// The focus rule, over what the person can see: a building with floor
    /// plans comes into focus when the viewport's centre is inside its
    /// footprint and it spans [`FOCUS_ENTER_SHARE`] of the viewport; the one
    /// in focus stays while it spans [`FOCUS_EXIT_SHARE`] and is in view.
    /// Nothing else is weighed. On a building switch the floor follows
    /// [`Map::arrival_level`]. `current` is the building and floor shown
    /// before the move; so is the answer.
    pub(super) fn focus(
        &self,
        view: &Viewport,
        current: Option<(usize, usize)>,
    ) -> Option<(usize, usize)> {
        let current = current
            .filter(|(building, level)| *level < self.export.buildings[*building].levels.len());
        let entering = self.building_containing(view.center, true).filter(|index| {
            view.share(&self.export.buildings[*index].footprint) >= FOCUS_ENTER_SHARE
        });
        let staying = current.filter(|(building, _)| {
            let footprint = &self.export.buildings[*building].footprint;
            view.share(footprint) >= FOCUS_EXIT_SHARE && view.meets(footprint)
        });
        match (entering, staying) {
            (Some(building), Some(shown)) if building == shown.0 => Some(shown),
            (Some(building), _) => self
                .arrival_level(current, building)
                .map(|level| (building, level)),
            (None, staying) => staying,
        }
    }

    /// The floor shown on arriving at `building` from the floor `from`: the
    /// same floor when it is that building's (a floor change stays), else
    /// the one as many floors above (or below) its street floor, else the
    /// street floor ([`Map::street_level`]). No building remembers a floor
    /// of its own.
    pub(super) fn arrival_level(
        &self,
        from: Option<(usize, usize)>,
        building: usize,
    ) -> Option<usize> {
        if let Some((_, level)) = from.filter(|(shown, _)| *shown == building) {
            return Some(level);
        }
        // Another building opens as high above its street as the floor
        // left was above its own: on a slope 3F of one is 1F of another.
        let ordinal = from.map(|(from, level)| {
            self.export.buildings[from].levels[level].ordinal - self.street_ordinal(from)
                + self.street_ordinal(building)
        });
        ordinal
            .and_then(|ordinal| {
                self.export.buildings[building]
                    .levels
                    .iter()
                    .position(|level| level.ordinal == ordinal)
            })
            .or_else(|| self.street_level(building))
    }

    /// The places a search query names, best first.
    pub(super) fn search(&self, query: &str) -> Vec<Found> {
        self.search.search(query)
    }

    /// The floor a room is first drawn on: the lowest with one of its units.
    pub(super) fn room_level(&self, room: usize) -> Option<(usize, usize)> {
        let building = self.room_building(room)?;
        let levels: HashSet<usize> = self.export.rooms[room]
            .unit_ids
            .iter()
            .filter_map(|id| self.units.get(id))
            .filter_map(|unit| self.levels.get(&self.export.units[*unit].level_id))
            .map(|(_, level)| *level)
            .collect();
        self.level_order(building)
            .into_iter()
            .find(|level| levels.contains(level))
            .map(|level| (building, level))
    }

    /// A room as a target, as if a room string named it alone.
    pub(super) fn room_target(&self, room: usize, english: bool) -> Option<Target> {
        self.target(Place::Room { room, level: None }, english)
    }

    /// A building's footprint box, else its centre.
    pub(super) fn building_bounds(&self, building: usize) -> geometry::Rect {
        let record = &self.export.buildings[building];
        geometry::bounds(record.footprint.iter().flatten()).unwrap_or([
            record.center[0],
            record.center[1],
            record.center[0],
            record.center[1],
        ])
    }

    /// The box around every building of a campus: where a page opened on
    /// no room string starts.
    pub(super) fn campus_bounds(&self, campus: &str) -> Option<geometry::Rect> {
        geometry::bounds(
            self.export
                .buildings
                .iter()
                .filter(|building| building.campus == campus)
                .flat_map(|building| building.footprint.iter().flatten()),
        )
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

    pub(super) fn room_building(&self, room: usize) -> Option<usize> {
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
            Some(rooms) => places.extend(
                rooms
                    .into_iter()
                    .map(|room| Place::Room { room, level: None }),
            ),
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
        let level_of = |unit: usize| {
            self.levels
                .get(&self.export.units[unit].level_id)
                .map(|(_, level)| *level)
        };
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
            .find(|level| {
                highlighted
                    .iter()
                    .any(|unit| level_of(*unit) == Some(*level))
            })
            .or(hinted)
            .or_else(|| self.street_level(building));
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
