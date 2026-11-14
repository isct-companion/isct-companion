//! One open map page: what the person chose and tapped, which building and
//! floor the camera shows, and the scene drawn from them. Shells keep none of
//! it; they forward intents and draw each scene by role.
//!
//! What is selected (the card) and what floor is shown are kept apart: the
//! camera, a floor picked or the card folded change the floor at most, never
//! the selection. Only another selection, or closing the card, replaces it.
use super::{
    export::{LineTone, UnitCategory},
    geometry::{self, Viewport},
    map::{maps_url, point, polygon, rect_bounds, Map, MapsApp, Match, Target, TAP_SLOP_POINTS},
    search::Found,
    CampusMapCamera, CampusMapCandidate, CampusMapCard, CampusMapFraming, CampusMapIntent,
    CampusMapLayer, CampusMapLayerRole, CampusMapLevel, CampusMapLink, CampusMapMark,
    CampusMapMarkKind, CampusMapPlace, CampusMapPoint, CampusMapScene, CampusMapSearchResult,
    CampusMapStackEntrance, CampusMapStackFloor,
};
use std::sync::Arc;

/// However far in, a framed room keeps some of its surroundings in view.
const LEAST_FRAMED_SIDE_M: f64 = 60.0;
/// A page opened on no room string starts over this campus.
const HOME_CAMPUS: &str = "ookayama";
/// Metres between floors in the stack: about twice as built, so the plan
/// of each floor shows between the ones above and below it.
const STACK_SPACING_M: f64 = 8.0;

/// What a page reads from the settings when it opens.
#[derive(Clone, Copy, Debug)]
pub(super) struct Preferences {
    pub(super) english: bool,
    pub(super) maps: MapsApp,
    /// The shell draws the stack, and a room on the plans opens in it.
    /// Only Android's does.
    pub(super) stacks: bool,
}

/// What the card describes. Each keeps the building (and floor) it was
/// picked on, wherever the camera goes after.
#[derive(Clone, Debug, PartialEq)]
enum Selection {
    None,
    /// One of the room string's targets, by index.
    Target(usize),
    /// A room picked by search.
    Room(Target),
    /// A building tapped or picked by search.
    Building(usize),
    /// A unit tapped on the floor shown then.
    Unit {
        unit: usize,
        building: usize,
        level: usize,
    },
    /// An entrance tapped on the map.
    Entrance {
        building: usize,
        entrance: usize,
    },
}

/// Everything a scene is drawn from: an unchanged one draws the same scene.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    floor: Option<(usize, usize)>,
    selection: Selection,
    framing: u64,
    stacked: bool,
}

pub(super) struct Page {
    map: Arc<Map>,
    preferences: Preferences,
    targets: Vec<Target>,
    /// The building whose floor is shown, and that floor.
    floor: Option<(usize, usize)>,
    selection: Selection,
    /// The last room selected, from the room string or a search: it stays
    /// lit while the person looks at another building or entrance, so the
    /// way back to it shows it still. Another room, a unit or closing the
    /// card replaces it.
    room: Option<Target>,
    framing: Option<CampusMapFraming>,
    /// The floor to show once the latest framing lands (the app's own camera
    /// report for it); `Some(None)` shows none.
    arrival: Option<Option<(usize, usize)>>,
    /// What the person saw at the last camera report the page took.
    view: Option<Viewport>,
    /// The floors are shown as a stack.
    stacked: bool,
    revision: u64,
    shown: Option<Shown>,
}

impl Page {
    /// A page on a room string, its first target chosen; on none, or one
    /// the map knows nothing of, nothing selected over the main campus.
    pub(super) fn open(map: Arc<Map>, preferences: Preferences, room: Option<&str>) -> Self {
        let targets = room.map_or_else(Vec::new, |room| map.locate(room, preferences.english));
        let mut page = Self {
            map,
            preferences,
            targets,
            floor: None,
            selection: Selection::None,
            room: None,
            framing: None,
            arrival: None,
            view: None,
            stacked: false,
            revision: 0,
            shown: None,
        };
        if page.targets.is_empty() {
            if let Some(campus) = page.map.campus_bounds(HOME_CAMPUS) {
                page.frame(campus, None);
            }
        } else {
            page.choose(0);
            page.keep_room();
            // A room string that names a room on the plans (W9-324, not just
            // W9) opens as the stack, the room standing out on its floor.
            if page.preferences.stacks
                && page
                    .targets
                    .first()
                    .is_some_and(|target| target.matched == Match::Room && target.level.is_some())
            {
                page.stack(true);
            }
        }
        page
    }

    pub(super) fn apply(&mut self, intent: CampusMapIntent) {
        match intent {
            CampusMapIntent::Choose { index } => self.choose(index as usize),
            CampusMapIntent::Camera { camera, framing } => self.camera(&camera, framing),
            CampusMapIntent::Level { id } => self.pick_level(&id),
            CampusMapIntent::Tap { point, level_id } => self.tap(point, level_id.as_deref()),
            CampusMapIntent::Select { place } => self.select(&place),
            CampusMapIntent::Clear => self.selection = Selection::None,
            CampusMapIntent::Stack { on } => self.stack(on),
        }
        self.keep_room();
    }

    /// Remembers the room selected; a building or an entrance picked since
    /// keeps the one before.
    fn keep_room(&mut self) {
        match &self.selection {
            Selection::Target(_) | Selection::Room(_) => {
                self.room = self
                    .selected_target()
                    .filter(|target| target.matched == Match::Room)
                    .cloned();
            }
            Selection::None | Selection::Unit { .. } => self.room = None,
            Selection::Building(_) | Selection::Entrance { .. } => {}
        }
    }

    /// The room lit: the one selected, else the one kept from before.
    fn kept_room(&self) -> Option<&Target> {
        match &self.selection {
            Selection::Target(_) | Selection::Room(_) => self.selected_target(),
            _ => self.room.as_ref(),
        }
        .filter(|target| target.matched == Match::Room)
    }

    /// The scene, when anything drawn changed since the last one.
    pub(super) fn scene(&mut self, page: u64) -> Option<CampusMapScene> {
        let shown = Shown {
            floor: self.floor,
            selection: self.selection.clone(),
            framing: self.framing.as_ref().map_or(0, |framing| framing.key),
            stacked: self.stacked,
        };
        if self.shown.as_ref() == Some(&shown) {
            return None;
        }
        self.shown = Some(shown);
        self.revision += 1;
        Some(self.render(page))
    }

    // MARK: Intents

    /// Asks the shell to bring `rect` into view; once that move lands, the
    /// floor becomes `arrival`. Its key outdates every camera report from
    /// before it.
    fn frame(&mut self, rect: geometry::Rect, arrival: Option<(usize, usize)>) {
        let key = self.framing.as_ref().map_or(0, |framing| framing.key) + 1;
        self.framing = Some(CampusMapFraming {
            key,
            bounds: rect_bounds(geometry::at_least(rect, LEAST_FRAMED_SIDE_M)),
        });
        self.arrival = Some(arrival);
    }

    fn frame_target(&mut self, target: &Target) {
        let focus = [
            target.focus.west,
            target.focus.south,
            target.focus.east,
            target.focus.north,
        ];
        self.frame(focus, target.level.map(|level| (target.building, level)));
    }

    /// Selects a target of the room string and frames its room.
    fn choose(&mut self, index: usize) {
        let Some(target) = self.targets.get(index).cloned() else {
            return;
        };
        self.selection = Selection::Target(index);
        self.frame_target(&target);
    }

    /// Every explicit building pick frames its bounds once, including a tap
    /// on the building already in focus. Its floor changes only on landing.
    fn pick_building(&mut self, building: usize) {
        self.selection = Selection::Building(building);
        // Back at the kept room's building, its floor.
        let arrival = self
            .room_floor(building)
            .or_else(|| self.map.arrival_level(self.floor, building))
            .map(|level| (building, level));
        self.frame(self.map.building_bounds(building), arrival);
    }

    /// One way in for every pick: a search result, a mark, a building.
    fn select(&mut self, place: &CampusMapPlace) {
        let map = self.map.clone();
        match place {
            CampusMapPlace::Building { id } => {
                if let Some(building) = map.buildings.get(id).copied() {
                    self.pick_building(building);
                }
            }
            CampusMapPlace::Room { id } => {
                let Some(target) = map
                    .rooms
                    .get(id)
                    .and_then(|room| map.room_target(*room, self.preferences.english))
                else {
                    return;
                };
                self.frame_target(&target);
                self.selection = Selection::Room(target);
            }
            CampusMapPlace::Unit { id } => {
                // A unit mark is drawn only on the floor shown; one from a
                // floor since left changes nothing.
                let Some(unit) = map.units.get(id).copied() else {
                    return;
                };
                let floor = map.levels.get(&map.export.units[unit].level_id).copied();
                if floor.is_some() && floor == self.floor {
                    if let Some((building, level)) = floor {
                        self.selection = Selection::Unit {
                            unit,
                            building,
                            level,
                        };
                    }
                }
            }
            CampusMapPlace::Entrance { id } => {
                if let Some((building, entrance)) = map.entrances.get(id).copied() {
                    self.selection = Selection::Entrance { building, entrance };
                }
            }
        }
    }

    /// A camera report from before the latest framing (the shell had not
    /// moved the camera yet) changes nothing: it would pull the floor back
    /// to where the camera was. The app's own move lands the framing's
    /// floor; a person's move re-picks the building by the focus rule.
    fn camera(&mut self, camera: &CampusMapCamera, framing: u64) {
        if framing < self.framing.as_ref().map_or(0, |latest| latest.key) {
            return;
        }
        let corner = |at: &CampusMapPoint| [at.longitude, at.latitude];
        let Some(view) = Viewport::new(
            [
                corner(&camera.top_left),
                corner(&camera.top_right),
                corner(&camera.bottom_right),
                corner(&camera.bottom_left),
            ],
            camera.width_points,
        ) else {
            return;
        };
        if camera.by_user {
            self.arrival = None;
            // A stack stays on its building however the person turns it:
            // seen at an angle, the camera's centre is no guide.
            if !self.stacked {
                self.floor = self.map.focus(&view, self.floor);
            }
        } else if let Some(arrival) = self.arrival.take() {
            self.floor = arrival;
        }
        self.view = Some(view);
    }

    /// A floor of the building shown; a stale pick of another building's
    /// floor changes nothing. A floor picked while a framing to the same
    /// building is under way wins over the floor it would land on.
    fn pick_level(&mut self, id: &str) {
        let Some((building, level)) = self.map.levels.get(id).copied() else {
            return;
        };
        if self.floor.is_some_and(|(shown, _)| shown == building) {
            self.floor = Some((building, level));
            if matches!(self.arrival, Some(Some((landing, _))) if landing == building) {
                self.arrival = None;
            }
        }
    }

    /// A tap on a named unit of the floor shown selects it. Elsewhere, a
    /// tap on a building's footprint (or within [`TAP_SLOP_POINTS`] of one,
    /// in metres at the camera's scale) selects the building. A tap on
    /// nothing changes nothing. `level_id` is the floor the shell drew under
    /// the finger: a tap on a floor the page has since left changes nothing.
    fn tap(&mut self, at: CampusMapPoint, level_id: Option<&str>) {
        let tapped = level_id.and_then(|id| self.map.levels.get(id).copied());
        if tapped != self.floor {
            return;
        }
        if let Some((building, level)) = self.floor {
            if let Some(unit) = self.map.unit_at(building, level, at) {
                let id = self.map.export.units[unit].id.clone();
                self.select(&CampusMapPlace::Unit { id });
                return;
            }
        }
        let slop = self
            .view
            .as_ref()
            .map_or(0.0, |view| TAP_SLOP_POINTS * view.metres_per_point);
        if let Some(building) = self.map.building_tapped([at.longitude, at.latitude], slop) {
            let id = self.map.export.buildings[building].id.clone();
            self.select(&CampusMapPlace::Building { id });
        }
    }

    /// Stacks the floors of the building in view (else the selection's),
    /// framed whole; back on, one floor from above.
    fn stack(&mut self, on: bool) {
        if !on {
            self.stacked = false;
            // Back on the flat map the selected room stays in view: its
            // floor comes forward, as it did in the stack.
            if let Some((building, _)) = self.floor {
                if let Some(level) = self.room_floor(building) {
                    self.floor = Some((building, level));
                }
            }
            return;
        }
        let Some(building) = self
            .floor
            .map(|(building, _)| building)
            .or_else(|| self.selected_building())
            .filter(|building| !self.map.export.buildings[*building].levels.is_empty())
        else {
            return;
        };
        // The selected room's floor comes forward, so the room stands out
        // in the stack; else the floor already shown, else the ground.
        let room_floor = self.room_floor(building);
        let floor = room_floor
            .map(|level| (building, level))
            .or_else(|| self.floor.filter(|(shown, _)| *shown == building))
            .or_else(|| {
                self.map
                    .street_level(building)
                    .map(|level| (building, level))
            });
        self.stacked = true;
        // The stack shows its floor at once; the framing lands on the same.
        if floor.is_some() {
            self.floor = floor;
        }
        self.frame(self.map.building_bounds(building), floor);
    }

    /// A floor's height in the stack, by its ordinal: its rank above the
    /// building's street floor, which lies on the ground; a floor below the
    /// street is under it, at a negative height.
    fn base_m(&self, building: usize, ordinal: i32) -> f64 {
        f64::from(ordinal - self.map.street_ordinal(building)) * STACK_SPACING_M
    }

    /// The floor of `building` the selected room is on, if the selection is
    /// a room there.
    fn room_floor(&self, building: usize) -> Option<usize> {
        match &self.selection {
            Selection::Unit {
                building: owner,
                level,
                ..
            } if *owner == building => Some(*level),
            _ => self
                .kept_room()
                .filter(|target| target.building == building)
                .and_then(|target| target.level),
        }
    }

    /// Every floor of the stacked building, lowest first.
    fn stack_floors(&self) -> Vec<CampusMapStackFloor> {
        let Some((building, shown)) = self.floor.filter(|_| self.stacked) else {
            return Vec::new();
        };
        let map = &self.map;
        let lit = self.highlighted();
        let record = &map.export.buildings[building];
        map.stack_levels(building)
            .into_iter()
            .map(|(ordinal, level)| {
                let Some(level) = level else {
                    // A floor between two with plans: only where it is.
                    return CampusMapStackFloor {
                        level_id: gap_level_id(&record.id, ordinal),
                        name: level_name(ordinal),
                        base_m: self.base_m(building, ordinal),
                        shown: false,
                        has_plan: false,
                        outline: record.footprint.iter().map(polygon).collect(),
                        rooms: Vec::new(),
                        walkways: Vec::new(),
                        highlight: Vec::new(),
                        entrances: Vec::new(),
                    };
                };
                let source = &record.levels[level];
                let mut floor = CampusMapStackFloor {
                    level_id: source.id.clone(),
                    name: source.name.clone(),
                    base_m: self.base_m(building, ordinal),
                    shown: level == shown,
                    has_plan: true,
                    outline: source.outline.iter().map(polygon).collect(),
                    rooms: Vec::new(),
                    walkways: Vec::new(),
                    highlight: Vec::new(),
                    entrances: record
                        .entrances
                        .iter()
                        .enumerate()
                        // Only the floor picked shows its entrances: the
                        // stack is read one floor at a time.
                        .filter(|(_, entrance)| {
                            level == shown
                                && entrance.level_id.as_deref() == Some(source.id.as_str())
                        })
                        .map(|(index, entrance)| CampusMapStackEntrance {
                            entrance_id: entrance.id.clone(),
                            name: entrance
                                .name
                                .as_ref()
                                .map(|name| name.pick(self.preferences.english).to_owned()),
                            point: point(entrance.point),
                            selected: self.selection
                                == Selection::Entrance {
                                    building,
                                    entrance: index,
                                },
                        })
                        .collect(),
                };
                for unit in map.units_by_level.get(&source.id).into_iter().flatten() {
                    let record = &map.export.units[*unit];
                    let shape = polygon(&record.polygon);
                    if lit.contains(unit) {
                        floor.highlight.push(shape);
                    } else if matches!(
                        record.category,
                        UnitCategory::Corridor
                            | UnitCategory::ElevatorLobby
                            | UnitCategory::Stairs
                            | UnitCategory::Lounge
                            | UnitCategory::Porch
                    ) {
                        floor.walkways.push(shape);
                    } else {
                        floor.rooms.push(shape);
                    }
                }
                floor
            })
            .collect()
    }

    // MARK: Scene

    /// The room string's target or the searched room the card shows.
    fn selected_target(&self) -> Option<&Target> {
        match &self.selection {
            Selection::Target(index) => self.targets.get(*index),
            Selection::Room(target) => Some(target),
            _ => None,
        }
    }

    /// The building the card describes, never the camera's.
    fn selected_building(&self) -> Option<usize> {
        match &self.selection {
            Selection::None => None,
            Selection::Target(_) | Selection::Room(_) => {
                self.selected_target().map(|target| target.building)
            }
            Selection::Building(building)
            | Selection::Unit { building, .. }
            | Selection::Entrance { building, .. } => Some(*building),
        }
    }

    /// Units drawn highlighted, on whichever floor they are.
    fn highlighted(&self) -> Vec<usize> {
        match &self.selection {
            Selection::Unit { unit, .. } => vec![*unit],
            Selection::Target(_) | Selection::Room(_) => self
                .selected_target()
                .map_or_else(Vec::new, |target| target.units.clone()),
            _ => self
                .kept_room()
                .map_or_else(Vec::new, |target| target.units.clone()),
        }
    }

    /// The building whose entrances are drawn: the one in focus, else the
    /// selection's.
    fn entrance_building(&self) -> Option<usize> {
        self.floor
            .map(|(building, _)| building)
            .or_else(|| self.selected_building())
    }

    /// The floor the selected room is on, while the picker lists its
    /// building.
    fn marked_level(&self) -> Option<usize> {
        let (shown, _) = self.floor?;
        self.room_floor(shown)
    }

    pub(super) fn render(&self, page: u64) -> CampusMapScene {
        let map = &self.map;
        let floor = self.floor;
        let levels = floor.map_or_else(Vec::new, |(building, _)| {
            let mut levels = if self.stacked {
                map.stack_levels(building)
                    .into_iter()
                    .map(|(ordinal, level)| match level {
                        Some(level) => map.level_model(building, level),
                        None => CampusMapLevel {
                            id: gap_level_id(&map.export.buildings[building].id, ordinal),
                            ordinal,
                            name: level_name(ordinal),
                            has_plan: false,
                        },
                    })
                    .collect()
            } else {
                map.level_models(building)
            };
            levels.reverse();
            levels
        });
        let level_id =
            floor.map(|(building, level)| map.export.buildings[building].levels[level].id.clone());
        let marked_level_id = floor
            .zip(self.marked_level())
            .map(|((building, _), level)| map.export.buildings[building].levels[level].id.clone());
        let mut layers = self.footprints();
        if !self.stacked {
            layers.extend(self.floor_layers());
        }
        CampusMapScene {
            page,
            revision: self.revision,
            layers,
            marks: self.marks(),
            levels,
            level_id,
            marked_level_id,
            card: self.card(),
            framing: self.framing.clone(),
            attribution: map.export.attribution.clone(),
            stacked: self.stacked && floor.is_some(),
            stack: self.stack_floors(),
        }
    }

    fn layer(id: impl Into<String>, role: CampusMapLayerRole) -> CampusMapLayer {
        CampusMapLayer {
            id: id.into(),
            role,
            polygons: Vec::new(),
            lines: Vec::new(),
            width_m: 0.0,
        }
    }

    /// Every footprint; the building in focus, else the selection's,
    /// stands out.
    fn footprints(&self) -> Vec<CampusMapLayer> {
        let emphasized = self.entrance_building();
        let focus_id =
            emphasized.map_or("none", |index| self.map.export.buildings[index].id.as_str());
        let mut plain = Self::layer(
            format!("footprints/{focus_id}"),
            CampusMapLayerRole::Footprint,
        );
        let mut focused = Self::layer(
            format!("focused-footprint/{focus_id}"),
            CampusMapLayerRole::FocusedFootprint,
        );
        for (index, building) in self.map.export.buildings.iter().enumerate() {
            let layer = if Some(index) == emphasized {
                &mut focused
            } else {
                &mut plain
            };
            layer
                .polygons
                .extend(building.footprint.iter().map(polygon));
        }
        [plain, focused]
            .into_iter()
            .filter(|layer| !layer.polygons.is_empty())
            .collect()
    }

    /// The floor shown: its outline, classrooms, detail lines under wall
    /// lines, then the highlighted room over them.
    fn floor_layers(&self) -> Vec<CampusMapLayer> {
        let Some((building, level)) = self.floor else {
            return Vec::new();
        };
        let map = &self.map;
        let lit = self.highlighted();
        let level = &map.export.buildings[building].levels[level];
        let on_level = |index: &std::collections::HashMap<String, Vec<usize>>| {
            index.get(&level.id).cloned().unwrap_or_default()
        };
        let units = on_level(&map.units_by_level);
        let mut outline = Self::layer(
            format!("outline/{}", level.id),
            CampusMapLayerRole::LevelOutline,
        );
        outline.polygons = level.outline.iter().map(polygon).collect();
        let highlighted_id = units
            .iter()
            .filter(|unit| lit.contains(unit))
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let hidden_classrooms_id = units
            .iter()
            .filter(|unit| {
                lit.contains(unit) && map.export.units[**unit].category == UnitCategory::Classroom
            })
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut classrooms = Self::layer(
            format!("classrooms/{}/{}", level.id, hidden_classrooms_id),
            CampusMapLayerRole::Classroom,
        );
        let mut highlight = Self::layer(
            format!("highlight/{}/{}", level.id, highlighted_id),
            CampusMapLayerRole::Highlight,
        );
        for unit in &units {
            let shape = polygon(&map.export.units[*unit].polygon);
            if lit.contains(unit) {
                highlight.polygons.push(shape);
            } else if map.export.units[*unit].category == UnitCategory::Classroom {
                classrooms.polygons.push(shape);
            }
        }
        let linework = on_level(&map.linework_by_level);
        let mut line_layers: Vec<CampusMapLayer> = Vec::new();
        for tone in [LineTone::Detail, LineTone::Wall] {
            let role = match tone {
                LineTone::Detail => CampusMapLayerRole::DetailLine,
                LineTone::Wall => CampusMapLayerRole::WallLine,
            };
            for index in linework.iter().copied() {
                let source = &map.export.linework[index];
                if source.tone != tone {
                    continue;
                }
                let width = source.width_m;
                let existing = line_layers.iter().position(|layer| {
                    layer.role == role && layer.width_m.to_bits() == width.to_bits()
                });
                let layer = match existing {
                    Some(index) => &mut line_layers[index],
                    None => {
                        let name = match tone {
                            LineTone::Detail => "detail-lines",
                            LineTone::Wall => "wall-lines",
                        };
                        let mut layer = Self::layer(
                            format!("{name}/{}/{:016x}", level.id, width.to_bits()),
                            role,
                        );
                        layer.width_m = width;
                        let index = line_layers.len();
                        line_layers.push(layer);
                        &mut line_layers[index]
                    }
                };
                layer.lines.extend(
                    source
                        .lines
                        .iter()
                        .map(|line| line.iter().copied().map(point).collect::<Vec<_>>()),
                );
            }
        }
        [outline, classrooms]
            .into_iter()
            .chain(line_layers)
            .chain([highlight])
            .filter(|layer| {
                layer.role == CampusMapLayerRole::LevelOutline
                    || !layer.polygons.is_empty()
                    || !layer.lines.is_empty()
            })
            .collect()
    }

    /// Room labels and amenities on the floor shown, then the entrances: on
    /// the floor shown, or all of them for a building without plans.
    fn marks(&self) -> Vec<CampusMapMark> {
        let map = &self.map;
        let mut plain = Vec::new();
        let mut amenities = Vec::new();
        let mut highlighted = Vec::new();
        let selected = self.highlighted();
        // A stack lies above the ground, where marks are drawn: it carries
        // its entrances on their own floors, and the ground has no marks.
        if self.stacked && self.floor.is_some() {
            return Vec::new();
        }
        let level_id = self
            .floor
            .map(|(building, level)| map.export.buildings[building].levels[level].id.as_str());
        if let Some(level_id) = level_id {
            for index in map.units_by_level.get(level_id).into_iter().flatten() {
                let unit = &map.export.units[*index];
                let Some(label) = &unit.label else {
                    continue;
                };
                let lit = selected.contains(index);
                let mark = CampusMapMark {
                    id: format!("unit/{}", unit.id),
                    point: point(unit.center),
                    kind: CampusMapMarkKind::Room {
                        unit_id: unit.id.clone(),
                        label: label.clone(),
                        highlighted: lit,
                    },
                };
                if lit {
                    highlighted.push(mark);
                } else {
                    plain.push(mark);
                }
            }
            for index in map.amenities_by_level.get(level_id).into_iter().flatten() {
                let amenity = &map.export.amenities[*index];
                amenities.push(CampusMapMark {
                    id: format!("amenity/{}", amenity.id),
                    point: point(amenity.point),
                    kind: CampusMapMarkKind::Amenity {
                        key: amenity.category.key().to_owned(),
                    },
                });
            }
        }
        let mut entrances = Vec::new();
        if let Some(building) = self.entrance_building() {
            for (index, entrance) in map.export.buildings[building].entrances.iter().enumerate() {
                if level_id.is_some() && entrance.level_id.as_deref() != level_id {
                    continue;
                }
                entrances.push(CampusMapMark {
                    id: format!("entrance/{}", entrance.id),
                    point: point(entrance.point),
                    kind: CampusMapMarkKind::Entrance {
                        entrance_id: entrance.id.clone(),
                        name: entrance
                            .name
                            .as_ref()
                            .map(|name| name.pick(self.preferences.english).to_owned()),
                        selected: self.selection
                            == Selection::Entrance {
                                building,
                                entrance: index,
                            },
                    },
                });
            }
        }
        plain
            .into_iter()
            .chain(amenities)
            .chain(highlighted)
            .chain(entrances)
            .collect()
    }

    fn level_name(&self, building: usize, id: Option<&str>) -> Option<String> {
        let (owner, level) = *self.map.levels.get(id?)?;
        (owner == building).then(|| {
            self.map.export.buildings[building].levels[level]
                .name
                .clone()
        })
    }

    fn card(&self) -> Option<CampusMapCard> {
        let building = self.selected_building()?;
        let map = &self.map;
        let english = self.preferences.english;
        let record = &map.export.buildings[building];
        let place = map.name(building, english);
        let entrance_name = |index: usize| {
            record.entrances[index]
                .name
                .as_ref()
                .map(|name| name.pick(english).to_owned())
        };
        let (title, building_name, level_name) = match &self.selection {
            Selection::None => return None,
            Selection::Target(_) | Selection::Room(_) => {
                let target = self.selected_target()?;
                (
                    Some(target.label.clone()),
                    target.building_name.clone(),
                    target.level.map(|level| record.levels[level].name.clone()),
                )
            }
            Selection::Building(_) => (Some(place.clone()), place.clone(), None),
            Selection::Unit { unit, level, .. } => (
                Some(
                    map.export.units[*unit]
                        .label
                        .clone()
                        .unwrap_or_else(|| place.clone()),
                ),
                place.clone(),
                Some(record.levels[*level].name.clone()),
            ),
            Selection::Entrance { entrance, .. } => (
                entrance_name(*entrance),
                place.clone(),
                self.level_name(building, record.entrances[*entrance].level_id.as_deref()),
            ),
        };
        let tapped = match self.selection {
            Selection::Entrance { entrance, .. } => Some(entrance),
            _ => None,
        };
        let mut order: Vec<usize> = (0..record.entrances.len()).collect();
        order.sort_by_key(|index| Some(*index) != tapped);
        let maps = self.preferences.maps;
        let mut links: Vec<CampusMapLink> = order
            .into_iter()
            .map(|index| {
                let name = entrance_name(index);
                let label = match &name {
                    Some(name) => format!("{place} {name}"),
                    None => place.clone(),
                };
                CampusMapLink {
                    entrance: true,
                    url: maps_url(maps, record.entrances[index].point, &label),
                    name,
                }
            })
            .collect();
        if links.is_empty() {
            links.push(CampusMapLink {
                entrance: false,
                name: None,
                url: maps_url(maps, record.center, &place),
            });
        }
        let candidates = if self.targets.len() > 1 {
            self.targets
                .iter()
                .enumerate()
                .map(|(index, candidate)| CampusMapCandidate {
                    label: candidate.label.clone(),
                    building_name: candidate.building_name.clone(),
                    chosen: self.selection == Selection::Target(index),
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(CampusMapCard {
            title,
            building_name,
            campus: record.campus.clone(),
            level_name,
            building_only: self
                .selected_target()
                .is_some_and(|target| target.matched == Match::Building),
            no_floor_plans: record.levels.is_empty(),
            links,
            candidates,
        })
    }
}

/// A search box's answers: each place's title, building, campus and floor,
/// and whether the plans show it.
pub(super) fn search(map: &Map, english: bool, query: &str) -> Vec<CampusMapSearchResult> {
    map.search(query)
        .into_iter()
        .map(|found| match found {
            Found::Building(building) => {
                let record = &map.export.buildings[building];
                let name = map.name(building, english);
                CampusMapSearchResult {
                    place: CampusMapPlace::Building {
                        id: record.id.clone(),
                    },
                    title: name.clone(),
                    building_name: name,
                    campus: record.campus.clone(),
                    level_name: None,
                    no_floor_plan: record.levels.is_empty(),
                }
            }
            Found::Room(room) => {
                let record = &map.export.rooms[room];
                let building = map.room_building(room);
                let floor = map.room_level(room);
                CampusMapSearchResult {
                    place: CampusMapPlace::Room {
                        id: record.id.clone(),
                    },
                    title: record.code.clone(),
                    building_name: building
                        .map(|index| map.name(index, english))
                        .unwrap_or_default(),
                    campus: building
                        .map(|index| map.export.buildings[index].campus.clone())
                        .unwrap_or_default(),
                    level_name: floor.map(|(building, level)| {
                        map.export.buildings[building].levels[level].name.clone()
                    }),
                    no_floor_plan: floor.is_none(),
                }
            }
        })
        .collect()
}

/// The id the stack gives a floor the export has no plan of: no level of
/// the export has it, so picking it changes nothing.
fn gap_level_id(building_id: &str, ordinal: i32) -> String {
    format!("{building_id}#{ordinal}")
}

/// A floor's name by its ordinal, as the export names them: `B1`, `1F`.
fn level_name(ordinal: i32) -> String {
    if ordinal < 0 {
        format!("B{}", -ordinal)
    } else {
        format!("{}F", ordinal + 1)
    }
}

#[cfg(test)]
#[path = "../tests/campus_map__page.rs"]
mod tests;
