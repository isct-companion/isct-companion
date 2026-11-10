//! One open map page: what the person chose and tapped, which building and
//! floor the camera shows, and the scene drawn from them. Shells keep none of
//! it; they forward intents and draw each scene by role.
use super::{
    export::{LineTone, UnitCategory},
    geometry,
    map::{maps_url, point, polygon, rect_bounds, Map, Match, Target},
    CampusMapCamera, CampusMapCandidate, CampusMapCard, CampusMapFraming, CampusMapIntent,
    CampusMapLayer, CampusMapLayerRole, CampusMapLink, CampusMapMark, CampusMapMarkKind,
    CampusMapPoint, CampusMapScene,
};
use std::sync::Arc;

/// However far in, a framed room keeps some of its surroundings in view.
const LEAST_FRAMED_SIDE_M: f64 = 60.0;

/// What the card describes. A tapped room or entrance keeps the building
/// (and floor) it was tapped on, wherever the camera goes after.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Selection {
    /// The room string's chosen target.
    Target,
    /// A unit tapped on the floor shown then.
    Unit {
        unit: usize,
        building: usize,
        level: usize,
    },
    /// An entrance tapped on the map.
    Entrance { building: usize, entrance: usize },
}

/// Everything a scene is drawn from: an unchanged one draws the same scene.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    chosen: usize,
    floor: Option<(usize, usize)>,
    selection: Selection,
    highlighted: Vec<usize>,
    framing: u64,
}

pub(super) struct Page {
    map: Arc<Map>,
    english: bool,
    targets: Vec<Target>,
    chosen: usize,
    /// The building whose floor is shown, and that floor.
    floor: Option<(usize, usize)>,
    selection: Selection,
    /// Units drawn highlighted, on whichever floor they are.
    highlighted: Vec<usize>,
    framing: Option<CampusMapFraming>,
    revision: u64,
    shown: Option<Shown>,
}

impl Page {
    /// A page on a room string, its first target chosen.
    pub(super) fn open(map: Arc<Map>, english: bool, room: &str) -> Self {
        let targets = map.locate(room, english);
        let mut page = Self {
            map,
            english,
            targets,
            chosen: 0,
            floor: None,
            selection: Selection::Target,
            highlighted: Vec::new(),
            framing: None,
            revision: 0,
            shown: None,
        };
        page.choose(0);
        page
    }

    pub(super) fn apply(&mut self, intent: CampusMapIntent) {
        match intent {
            CampusMapIntent::Choose { index } => self.choose(index as usize),
            CampusMapIntent::Camera { camera, framing } => self.camera(&camera, framing),
            CampusMapIntent::Level { id } => self.pick_level(&id),
            CampusMapIntent::Tap { point, level_id } => self.tap(point, level_id.as_deref()),
            CampusMapIntent::Entrance { id } => self.tap_entrance(&id),
        }
    }

    /// The scene, when anything drawn changed since the last one.
    pub(super) fn scene(&mut self, page: u64) -> Option<CampusMapScene> {
        let shown = Shown {
            chosen: self.chosen,
            floor: self.floor,
            selection: self.selection,
            highlighted: self.highlighted.clone(),
            framing: self.framing.as_ref().map_or(0, |framing| framing.key),
        };
        if self.shown.as_ref() == Some(&shown) {
            return None;
        }
        self.shown = Some(shown);
        self.revision += 1;
        Some(self.render(page))
    }

    // MARK: Intents

    fn target(&self) -> Option<&Target> {
        self.targets.get(self.chosen)
    }

    fn target_building(&self) -> Option<usize> {
        self.target().map(|target| target.building)
    }

    /// Opens on a target: its building and the floor it opens on, framed on
    /// the room. The camera move that follows is the app's, so it re-picks
    /// nothing.
    fn choose(&mut self, index: usize) {
        let Some(target) = self.targets.get(index) else {
            return;
        };
        self.chosen = index;
        self.floor = target.level.map(|level| (target.building, level));
        let bounds = geometry::at_least(
            [
                target.focus.west,
                target.focus.south,
                target.focus.east,
                target.focus.north,
            ],
            LEAST_FRAMED_SIDE_M,
        );
        let key = self.framing.as_ref().map_or(0, |framing| framing.key) + 1;
        self.framing = Some(CampusMapFraming {
            key,
            bounds: rect_bounds(bounds),
        });
        self.show_target();
    }

    /// The card back on the chosen target, its rooms highlighted; the camera
    /// stays where it is.
    fn show_target(&mut self) {
        self.selection = Selection::Target;
        self.highlighted = self
            .target()
            .map_or_else(Vec::new, |target| target.units.clone());
    }

    /// A camera report from before the latest framing (the shell had not
    /// moved the camera yet) changes nothing: it would pull the floor back
    /// to where the camera was.
    fn camera(&mut self, camera: &CampusMapCamera, framing: u64) {
        if framing < self.framing.as_ref().map_or(0, |latest| latest.key) {
            return;
        }
        self.floor = self.map.focus(camera, self.floor);
    }

    /// A floor of the building shown; a stale pick of another building's
    /// floor changes nothing.
    fn pick_level(&mut self, id: &str) {
        let Some((building, level)) = self.map.levels.get(id).copied() else {
            return;
        };
        if self.floor.is_some_and(|(shown, _)| shown == building) {
            self.floor = Some((building, level));
        }
    }

    /// A tap on a named unit of the floor shown selects it, with that floor
    /// and its building; its highlight stays while the camera is elsewhere.
    /// A tap that hits nothing, or with no floor shown, goes back to the
    /// target. `level_id` is the floor the shell drew under the finger: a
    /// tap on a floor the page has since left changes nothing.
    fn tap(&mut self, at: CampusMapPoint, level_id: Option<&str>) {
        let tapped = level_id.and_then(|id| self.map.levels.get(id).copied());
        if tapped != self.floor {
            return;
        }
        let hit = self.floor.and_then(|(building, level)| {
            let unit = self.map.unit_at(building, level, at)?;
            Some((unit, building, level))
        });
        let Some((unit, building, level)) = hit else {
            self.show_target();
            return;
        };
        self.highlighted = vec![unit];
        self.selection = Selection::Unit {
            unit,
            building,
            level,
        };
    }

    /// An entrance of the building whose entrances are drawn.
    fn tap_entrance(&mut self, id: &str) {
        let Some(building) = self.entrance_building() else {
            return;
        };
        let shown = self
            .floor
            .map(|(shown, level)| self.map.export.buildings[shown].levels[level].id.as_str());
        // Drawn: on the floor shown, or every one without a floor.
        let found = self.map.export.buildings[building]
            .entrances
            .iter()
            .position(|entrance| {
                entrance.id == id && (shown.is_none() || entrance.level_id.as_deref() == shown)
            });
        if let Some(entrance) = found {
            self.selection = Selection::Entrance { building, entrance };
        }
    }

    // MARK: Scene

    /// The building whose entrances are drawn: the one in focus, or the
    /// target's when it has no floor plans to focus on.
    fn entrance_building(&self) -> Option<usize> {
        self.floor.map(|(building, _)| building).or_else(|| {
            self.target()
                .filter(|target| target.level.is_none())
                .map(|target| target.building)
        })
    }

    /// The building the card describes: the target's, or the one a room or
    /// entrance was tapped on.
    fn selected_building(&self) -> Option<usize> {
        match self.selection {
            Selection::Target => self.target_building(),
            Selection::Unit { building, .. } | Selection::Entrance { building, .. } => {
                Some(building)
            }
        }
    }

    /// The floor the selected room is on, while the picker lists its
    /// building.
    fn marked_level(&self) -> Option<usize> {
        let (shown, _) = self.floor?;
        match self.selection {
            Selection::Target => {
                let target = self.target()?;
                if target.matched != Match::Room || target.building != shown {
                    return None;
                }
                target.level
            }
            Selection::Unit {
                building, level, ..
            } => (building == shown).then_some(level),
            Selection::Entrance { .. } => None,
        }
    }

    pub(super) fn render(&self, page: u64) -> CampusMapScene {
        let map = &self.map;
        let floor = self.floor;
        let levels = floor.map_or_else(Vec::new, |(building, _)| {
            let mut levels = map.level_models(building);
            levels.reverse();
            levels
        });
        let level_id =
            floor.map(|(building, level)| map.export.buildings[building].levels[level].id.clone());
        let marked_level_id = floor
            .zip(self.marked_level())
            .map(|((building, _), level)| map.export.buildings[building].levels[level].id.clone());
        let mut layers = self.footprints();
        layers.extend(self.floor_layers());
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

    /// Every footprint; the building in focus, else the target's, stands out.
    fn footprints(&self) -> Vec<CampusMapLayer> {
        let emphasized = self
            .floor
            .map(|(building, _)| building)
            .or_else(|| self.target_building());
        let focus_id = emphasized.map_or("none", |index| {
            self.map.export.buildings[index].id.as_str()
        });
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
            .filter(|unit| self.highlighted.contains(unit))
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let hidden_classrooms_id = units
            .iter()
            .filter(|unit| {
                self.highlighted.contains(unit)
                    && map.export.units[**unit].category == UnitCategory::Classroom
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
            if self.highlighted.contains(unit) {
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
                    layer.role == role
                        && layer.width_m.to_bits() == width.to_bits()
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
        let level_id = self
            .floor
            .map(|(building, level)| map.export.buildings[building].levels[level].id.as_str());
        if let Some(level_id) = level_id {
            for index in map.units_by_level.get(level_id).into_iter().flatten() {
                let unit = &map.export.units[*index];
                let Some(label) = &unit.label else {
                    continue;
                };
                let lit = self.highlighted.contains(index);
                let mark = CampusMapMark {
                    id: format!("unit/{}", unit.id),
                    point: point(unit.center),
                    kind: CampusMapMarkKind::Room {
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
                            .map(|name| name.pick(self.english).to_owned()),
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
        let target = self.target()?;
        let building = self.selected_building()?;
        let map = &self.map;
        let record = &map.export.buildings[building];
        let place = map.name(building, self.english);
        let entrance_name = |index: usize| {
            record.entrances[index]
                .name
                .as_ref()
                .map(|name| name.pick(self.english).to_owned())
        };
        let (title, building_name, level_name) = match self.selection {
            Selection::Target => (
                Some(target.label.clone()),
                target.building_name.clone(),
                target.level.map(|level| record.levels[level].name.clone()),
            ),
            Selection::Unit { unit, level, .. } => (
                Some(
                    map.export.units[unit]
                        .label
                        .clone()
                        .unwrap_or_else(|| place.clone()),
                ),
                place.clone(),
                Some(record.levels[level].name.clone()),
            ),
            Selection::Entrance { entrance, .. } => (
                entrance_name(entrance),
                place.clone(),
                self.level_name(building, record.entrances[entrance].level_id.as_deref()),
            ),
        };
        let tapped = match self.selection {
            Selection::Entrance { entrance, .. } => Some(entrance),
            _ => None,
        };
        let mut order: Vec<usize> = (0..record.entrances.len()).collect();
        order.sort_by_key(|index| Some(*index) != tapped);
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
                    url: maps_url(record.entrances[index].point, &label),
                    name,
                }
            })
            .collect();
        if links.is_empty() {
            links.push(CampusMapLink {
                entrance: false,
                name: None,
                url: maps_url(record.center, &place),
            });
        }
        let candidates = if self.targets.len() > 1 {
            self.targets
                .iter()
                .enumerate()
                .map(|(index, candidate)| CampusMapCandidate {
                    label: candidate.label.clone(),
                    building_name: candidate.building_name.clone(),
                    chosen: index == self.chosen,
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(CampusMapCard {
            title,
            building_name,
            level_name,
            building_only: self.selection == Selection::Target && target.matched == Match::Building,
            links,
            candidates,
        })
    }
}
