//! The page's rules over the published export, by fixed real places, through
//! its contract: intents in, the rendered scene out. The file is not in this
//! repository: `ISCT_CAMPUS_MAP_EXPORT` names a downloaded
//! `campus-map.json.gz`, the pinned one the `campus-map` workflow fetches, so
//! these run only with `--ignored`.
use super::*;
use crate::campus_map::{
    export,
    geometry::Rect,
    map::{FOCUS_ENTER_SHARE, FOCUS_EXIT_SHARE},
};
use crate::locale::ContentLanguage;

/// 大岡山西9号館.
const W9: &str = "a10026fc-4e59-55a7-9d89-0193f1749620";
/// 大岡山西講義棟1, south of W9, entered on 3F.
const WL1: &str = "95e63ef0-8ebc-5502-87ed-1f414753ce93";
/// 本館: the export lists its eight entrances out of order.
const MAIN_BUILDING: &str = "2af6a38b-1caa-5516-a54b-c748494f14a9";
/// 屋内運動場, south-west of W9 and smaller.
const SPORTS_CENTER: &str = "3a605ae0-88df-4574-aadf-56da13339a23";
/// Inside W9's footprint, 6 m from the sports center's.
const BY_THE_SPORTS_CENTER: [f64; 2] = [139.6821654, 35.6057532];
const PREFERENCES: Preferences = Preferences {
    language: ContentLanguage::Ja,
    maps: MapsApp::Apple,
    stacks: false,
};

fn map() -> Arc<Map> {
    let path = std::env::var("ISCT_CAMPUS_MAP_EXPORT").unwrap_or_else(|_| {
        panic!("ISCT_CAMPUS_MAP_EXPORT must name the published campus-map.json.gz")
    });
    let gz = std::fs::read(&path).unwrap_or_else(|error| panic!("cannot read {path}: {error}"));
    Arc::new(Map::new(export::read(&gz).expect("format 2")).expect("a valid export"))
}

/// A view of `rect`, upright.
fn view(rect: Rect) -> Viewport {
    let [west, south, east, north] = rect;
    Viewport::new([[west, north], [east, north], [east, south], [west, south]])
        .expect("a view with area")
}

/// A square view centred on `at` in which `building`'s footprint spans `share`.
fn view_at(map: &Map, building: &str, at: [f64; 2], share: f64) -> Viewport {
    let footprint = &map.export.buildings[map.buildings[building]].footprint;
    let probe = view(geometry::at_least([at[0], at[1], at[0], at[1]], 100.0));
    let side = 100.0 * probe.share(footprint) / share;
    view(geometry::at_least([at[0], at[1], at[0], at[1]], side))
}

/// The person moved the camera to `view`; the scene drawn after.
fn look(page: &mut Page, view: &Viewport) -> CampusMapScene {
    let corner = |index: usize| CampusMapPoint {
        latitude: view.ring[index][1],
        longitude: view.ring[index][0],
    };
    page.apply(CampusMapIntent::Camera {
        camera: CampusMapCamera {
            top_left: corner(0),
            top_right: corner(1),
            bottom_right: corner(2),
            bottom_left: corner(3),
        },
    });
    page.render(1)
}

fn send(page: &mut Page, intent: CampusMapIntent) -> CampusMapScene {
    page.apply(intent);
    page.render(1)
}

fn select(page: &mut Page, place: CampusMapPlace) -> CampusMapScene {
    send(page, CampusMapIntent::Select { place })
}

fn tap(
    page: &mut Page,
    at: [f64; 2],
    level_id: Option<String>,
    metres_per_point: f64,
    rooms_only: bool,
) -> CampusMapScene {
    let point = CampusMapPoint {
        latitude: at[1],
        longitude: at[0],
    };
    send(
        page,
        CampusMapIntent::Tap {
            point,
            level_id,
            metres_per_point,
            rooms_only,
        },
    )
}

fn building(id: &str) -> CampusMapPlace {
    CampusMapPlace::Building { id: id.to_owned() }
}

fn room(map: &Map, code: &str) -> CampusMapPlace {
    let room = map
        .export
        .rooms
        .iter()
        .find(|room| room.code == code)
        .expect(code);
    CampusMapPlace::Room {
        id: room.id.clone(),
    }
}

fn building_of(map: &Map, level_id: Option<&str>) -> Option<usize> {
    level_id.map(|id| map.levels[id].0)
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn focus_holds_through_small_zooms_and_enters_on_the_street_floor() {
    let map = map();
    let center = map.export.buildings[map.buildings[W9]].center;
    let mut page = Page::open(map.clone(), PREFERENCES, None);
    let at = |share: f64| view_at(&map, W9, center, share);
    // The whole campus in view, its centre inside W9: no floor.
    let campus = map.campus_bounds("ookayama").expect("ookayama");
    let (half_w, half_h) = ((campus[2] - campus[0]) / 2.0, (campus[3] - campus[1]) / 2.0);
    let whole = view([
        center[0] - half_w,
        center[1] - half_h,
        center[0] + half_w,
        center[1] + half_h,
    ]);
    assert_eq!(look(&mut page, &whole).level_id, None);
    // The shares sit around the thresholds, so retuning them keeps this
    // passing while the gap between entering and leaving stays.
    let between = (FOCUS_ENTER_SHARE + FOCUS_EXIT_SHARE) / 2.0;
    assert_eq!(look(&mut page, &at(between)).level_id, None);
    let entered = look(&mut page, &at(FOCUS_ENTER_SHARE * 1.1));
    // W9 opens on the floor its main entrance is on, which the picker marks.
    let street: Vec<_> = entered.levels.iter().filter(|level| level.street).collect();
    assert_eq!(street.len(), 1);
    assert_eq!(entered.level_id.as_deref(), Some(street[0].id.as_str()));
    assert_eq!(look(&mut page, &at(between)).level_id, entered.level_id);
    assert_eq!(look(&mut page, &at(FOCUS_EXIT_SHARE * 0.9)).level_id, None);
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn the_building_holding_a_point_wins_over_a_smaller_one_nearby() {
    let map = map();
    let footprint = |id: &str| &map.export.buildings[map.buildings[id]].footprint;
    let point = BY_THE_SPORTS_CENTER;
    assert!(geometry::multipolygon_contains(footprint(W9), point));
    assert!(geometry::distance_to_edges(footprint(SPORTS_CENTER), point) < 10.0);
    assert!(
        geometry::multipolygon_area(footprint(SPORTS_CENTER))
            < geometry::multipolygon_area(footprint(W9))
    );

    let mut page = Page::open(map.clone(), PREFERENCES, None);
    // At a scale whose slop reaches the sports center.
    let tapped = tap(&mut page, point, None, 2.0, false);
    let w9 = map.buildings[W9];
    assert_eq!(
        tapped.card.map(|card| card.title),
        Some(map.name(w9, ContentLanguage::Ja))
    );
    let focused = look(&mut page, &view_at(&map, W9, point, 0.8));
    assert_eq!(building_of(&map, focused.level_id.as_deref()), Some(w9));
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn search_finds_buildings_by_the_codes_in_the_data() {
    let map = map();
    let first = |query: &str| {
        search(&map, ContentLanguage::Ja, query)
            .into_iter()
            .next()
            .map(|result| result.place)
    };
    for code in ["MI1", "MI3", "W9"] {
        let coded = map
            .export
            .buildings
            .iter()
            .find(|record| record.codes.iter().any(|known| known == code));
        assert_eq!(
            first(code),
            Some(building(&coded.expect(code).id)),
            "{code}"
        );
    }
    assert_eq!(first("w9-202"), Some(room(&map, "W9-202")));
    assert!(!map.locate("MI1", ContentLanguage::Ja).is_empty());
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn a_selection_survives_floor_changes_zooming_out_and_other_buildings() {
    let map = map();
    let w9 = map.buildings[W9];
    let lit = |scene: &CampusMapScene| {
        scene
            .layers
            .iter()
            .any(|layer| layer.role == CampusMapLayerRole::Highlight && !layer.polygons.is_empty())
    };
    let title = |scene: &CampusMapScene| scene.card.as_ref().map(|card| card.title.clone());
    // The name a building carries on the map, when it carries one.
    let named = |scene: &CampusMapScene, id: &str| {
        scene.marks.iter().find_map(|mark| match &mark.kind {
            CampusMapMarkKind::Building { building_id, label } if building_id == id => {
                Some(label.clone())
            }
            _ => None,
        })
    };
    let mut page = Page::open(map.clone(), PREFERENCES, Some("W9-202"));
    // The room's floor shows at once with the room lit, marked in the picker.
    let opened = page.render(1);
    let room_floor = opened.level_id.clone();
    assert_eq!(building_of(&map, room_floor.as_deref()), Some(w9));
    assert_eq!(opened.marked_level_id, room_floor);
    assert!(lit(&opened) && opened.framing.is_some());
    assert_eq!(title(&opened).as_deref(), Some("W9-202"));

    // A tap on a part of W9's footprint no named unit covers picks W9 and
    // frames it again, though it is in focus; the room stays lit on its floor.
    let [west, south, east, north] = map.building_bounds(w9);
    let level = map.levels[room_floor.as_deref().expect("a floor")].1;
    let at = (1..100)
        .flat_map(|x| (1..100).map(move |y| (x, y)))
        .map(|(x, y)| {
            let share = |step: i32| f64::from(step) / 100.0;
            [
                west + (east - west) * share(x),
                south + (north - south) * share(y),
            ]
        })
        .find(|at| {
            let point = CampusMapPoint {
                latitude: at[1],
                longitude: at[0],
            };
            map.building_containing(*at, false) == Some(w9)
                && map.unit_at(w9, level, point).is_none()
        })
        .expect("an unnamed part of W9's footprint");
    let tapped = tap(&mut page, at, room_floor.clone(), 0.1, false);
    assert_eq!(title(&tapped), Some(map.name(w9, ContentLanguage::Ja)));
    assert_eq!(tapped.level_id, room_floor);
    assert!(lit(&tapped));
    // Search picks the same place the same way, and frames it again though
    // it is in focus.
    let searched = select(&mut page, building(W9));
    let (framed, again) = (
        tapped.framing.expect("a framing"),
        searched.framing.expect("a framing"),
    );
    assert_eq!(framed.bounds, again.bounds);
    assert_ne!(framed, again);

    // Another building shows at once, its own card.
    let sports = select(&mut page, building(SPORTS_CENTER));
    let sports_center = map.buildings[SPORTS_CENTER];
    assert_eq!(
        building_of(&map, sports.level_id.as_deref()),
        Some(sports_center)
    );
    assert_eq!(
        title(&sports),
        Some(map.name(sports_center, ContentLanguage::Ja))
    );
    // The selected building is named by its card, the others on the map,
    // without the campus name the Japanese names start with.
    assert_eq!(named(&sports, SPORTS_CENTER), None);
    assert_eq!(named(&sports, W9).as_deref(), Some("西9号館"));

    // The room again: another floor, then the whole campus, keep it selected.
    let back = select(&mut page, room(&map, "W9-202"));
    assert_eq!(back.level_id, room_floor);
    let other = back
        .levels
        .iter()
        .find(|level| Some(&level.id) != room_floor.as_ref())
        .expect("W9 has several floors");
    let upstairs = send(
        &mut page,
        CampusMapIntent::Level {
            id: other.id.clone(),
        },
    );
    assert_eq!(upstairs.level_id.as_ref(), Some(&other.id));
    assert_eq!(upstairs.marked_level_id, room_floor);
    assert_eq!(title(&upstairs).as_deref(), Some("W9-202"));
    let campus = look(
        &mut page,
        &view(map.campus_bounds("ookayama").expect("ookayama")),
    );
    assert!(campus.level_id.is_none() && campus.marked_level_id.is_none());
    assert_eq!(title(&campus).as_deref(), Some("W9-202"));
    assert_eq!(named(&campus, W9), None);
    let cleared = send(&mut page, CampusMapIntent::Clear);
    assert_eq!(cleared.card, None);
    assert!(named(&cleared, W9).is_some());
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn a_quick_tap_picks_a_room_and_a_tap_on_nothing_closes_the_card() {
    let map = map();
    let mut page = Page::open(map.clone(), PREFERENCES, Some("W9-202"));
    let opened = page.render(1);
    let floor = opened.level_id.clone();
    let classrooms = |scene: &CampusMapScene| {
        scene
            .layers
            .iter()
            .find(|layer| layer.role == CampusMapLayerRole::Classroom)
            .map(|layer| layer.id.clone())
    };
    // Another named unit on the room's floor.
    let other = map.units_by_level[floor.as_deref().expect("a floor")]
        .iter()
        .map(|unit| &map.export.units[*unit])
        .find(|unit| unit.label.as_deref().is_some_and(|label| label != "W9-202"))
        .expect("another named unit on W9-202's floor");
    let title = |scene: &CampusMapScene| scene.card.as_ref().map(|card| card.title.clone());

    // The quick tap selects it on its floor and asks for it to be revealed,
    // not fitted; the floor's classrooms stay the same layer.
    let quick = tap(&mut page, other.center, floor.clone(), 0.1, true);
    assert_eq!(title(&quick), other.label);
    assert_eq!(quick.level_id, floor);
    assert!(quick.framing.as_ref().is_some_and(|framing| framing.reveal));
    assert_eq!(classrooms(&quick), classrooms(&opened));
    // The full tap that follows finds it selected: no new scene.
    assert!(page.scene(1).is_some());
    page.apply(CampusMapIntent::Tap {
        point: point(other.center),
        level_id: floor.clone(),
        metres_per_point: 0.1,
        rooms_only: false,
    });
    assert!(page.scene(1).is_none());

    // Far from every building: the quick tap leaves the card, the full one
    // closes it.
    let nowhere = [139.0, 35.0];
    assert_eq!(
        title(&tap(&mut page, nowhere, floor.clone(), 0.1, true)),
        other.label
    );
    assert_eq!(tap(&mut page, nowhere, floor, 0.1, false).card, None);
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn an_entrance_picked_on_the_building_card_shows_its_floor_in_the_map() {
    let map = map();
    let w9 = map.buildings[W9];
    let mut page = Page::open(map.clone(), PREFERENCES, None);
    let card = select(&mut page, building(W9)).card.expect("W9's card");
    assert_eq!(card.kind, CampusMapCardKind::Building);
    assert_eq!(card.building, None);
    // W9 is not marked complete: a build that can send photos asks for them.
    assert_eq!(
        card.photo_building_id.as_deref(),
        crate::campus_map::photos::available().then_some(W9)
    );
    let record = &map.export.buildings[w9];
    assert_eq!(card.entrances.len(), record.entrances.len());
    // An entrance on a floor other than the one W9 opened on, else any.
    let shown = page.render(1).level_id;
    let entrance = record
        .entrances
        .iter()
        .find(|entrance| entrance.level_id.is_some() && entrance.level_id != shown)
        .or_else(|| record.entrances.first())
        .expect("W9 has entrances");
    let picked = select(
        &mut page,
        CampusMapPlace::Entrance {
            id: entrance.id.clone(),
        },
    );
    if entrance.level_id.is_some() {
        assert_eq!(picked.level_id, entrance.level_id);
    }
    // Still W9's one card, the entrance's row checked.
    let card = picked.card.expect("W9's card");
    assert_eq!(card.kind, CampusMapCardKind::Building);
    assert_eq!(card.building, None);
    let checked: Vec<_> = card.entrances.iter().filter(|row| row.selected).collect();
    assert_eq!(checked.len(), 1);
    assert_eq!(checked[0].entrance_id, entrance.id);
    // Its one way out pins the entrance, not the building.
    let [lng, lat] = entrance.point;
    assert!(
        card.maps_url.contains(&format!("{lat:.7}"))
            && card.maps_url.contains(&format!("{lng:.7}"))
    );
    assert!(picked.marks.iter().any(|mark| matches!(
        &mark.kind,
        CampusMapMarkKind::Entrance { entrance_id, selected: true, .. } if *entrance_id == entrance.id
    )));
    // A card lists the entrances by number, and with none picked its way
    // out pins the first.
    let mut page = Page::open(map.clone(), PREFERENCES, None);
    let card = select(&mut page, building(MAIN_BUILDING))
        .card
        .expect("本館's card");
    let names: Vec<_> = card
        .entrances
        .iter()
        .map(|row| row.name.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(
        names,
        ["玄関1", "玄関2", "玄関3", "玄関4", "玄関5", "玄関6", "玄関7", "玄関8"]
    );
    let first = map.export.buildings[map.buildings[MAIN_BUILDING]]
        .entrances
        .iter()
        .find(|entrance| entrance.id == card.entrances[0].entrance_id)
        .expect("a row is one of its entrances");
    let [lng, lat] = first.point;
    assert!(
        card.maps_url.contains(&format!("{lat:.7}"))
            && card.maps_url.contains(&format!("{lng:.7}"))
    );
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn locate_picks_the_building_the_person_stands_in_or_near() {
    let map = map();
    let w9 = map.buildings[W9];
    let mut page = Page::open(map.clone(), PREFERENCES, None);
    let point = |at: [f64; 2]| CampusMapPoint {
        latitude: at[1],
        longitude: at[0],
    };
    // Inside W9: W9's card, its floor shown, the camera on it.
    let inside = send(
        &mut page,
        CampusMapIntent::Locate {
            point: point(BY_THE_SPORTS_CENTER),
        },
    );
    assert_eq!(
        inside.card.as_ref().map(|card| card.title.clone()),
        Some(map.name(w9, ContentLanguage::Ja))
    );
    assert_eq!(building_of(&map, inside.level_id.as_deref()), Some(w9));
    let framed = inside.framing.clone().expect("a framing");
    assert!(!framed.reveal);

    // Far from campus: the card stays, only the position is framed.
    let far = send(
        &mut page,
        CampusMapIntent::Locate {
            point: point([139.0, 35.0]),
        },
    );
    assert_eq!(far.card, inside.card);
    let around = far.framing.expect("a framing");
    assert_ne!(around.key, framed.key);
    assert!(around.bounds.west < 139.0 && around.bounds.east > 139.0);
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn a_room_on_the_plans_opens_as_a_stack_about_its_street_floor() {
    let map = map();
    let w9 = map.buildings[W9];
    let record = &map.export.buildings[w9];
    // Without `stacks` the room opens on one floor.
    let flat = Page::open(map.clone(), PREFERENCES, Some("W9-324"));
    assert!(!flat.render(1).stacked);
    let stacks = Preferences {
        stacks: true,
        ..PREFERENCES
    };
    let mut page = Page::open(map.clone(), stacks, Some("W9-324"));
    let scene = page.render(1);
    assert!(scene.stacked);
    let shown = page.floor.expect("the room's floor");
    assert_eq!(shown.0, w9);
    // Nothing of a floor on the ground: footprints only, no marks.
    assert!(scene.marks.is_empty());
    assert!(scene.layers.iter().all(|layer| matches!(
        layer.role,
        CampusMapLayerRole::Footprint | CampusMapLayerRole::FocusedFootprint
    )));
    // Every floor from the lowest to the highest, lowest first, a storey
    // apart; the street floor (the export's) at 0, those under it below.
    let ordinals: Vec<i32> = record.levels.iter().map(|level| level.ordinal).collect();
    let (lowest, highest) = (
        *ordinals.iter().min().expect("floors"),
        *ordinals.iter().max().expect("floors"),
    );
    assert_eq!(scene.stack.len(), (highest - lowest + 1) as usize);
    assert!(scene
        .stack
        .windows(2)
        .all(|pair| pair[1].base_m - pair[0].base_m == STACK_SPACING_M));
    let street = map.street_level(w9).expect("W9's street floor");
    let streets: Vec<_> = scene.stack.iter().filter(|floor| floor.street).collect();
    assert_eq!(streets.len(), 1);
    assert_eq!(streets[0].level_id, record.levels[street].id);
    assert_eq!(streets[0].base_m, 0.0);
    assert!(scene.stack[0].base_m < 0.0 || record.levels[street].ordinal == lowest);
    // The room's floor is picked and carries the room; floors with no plan
    // are listed but cannot be picked.
    let picked: Vec<_> = scene.stack.iter().filter(|floor| floor.shown).collect();
    assert_eq!(picked.len(), 1);
    assert_eq!(picked[0].level_id, record.levels[shown.1].id);
    assert!(!picked[0].highlight.is_empty());
    let gaps: Vec<_> = scene.stack.iter().filter(|floor| !floor.has_plan).collect();
    assert_eq!(gaps.len(), scene.stack.len() - record.levels.len());
    assert!(!gaps.is_empty(), "W9 has floors between its plans");
    assert_eq!(
        scene
            .levels
            .iter()
            .map(|level| (level.name.clone(), level.has_plan))
            .collect::<Vec<_>>(),
        scene
            .stack
            .iter()
            .rev()
            .map(|floor| (floor.name.clone(), floor.has_plan))
            .collect::<Vec<_>>()
    );
    send(
        &mut page,
        CampusMapIntent::Level {
            id: gaps[0].level_id.clone(),
        },
    );
    assert_eq!(page.floor, Some(shown));
    // The ground at the street floor is the building's footprint.
    assert_eq!(scene.stack_ground.len(), record.footprint.len());
    // The map shows the entrances of the floor picked alone; every floor
    // with a way in is marked as entered, in the stack and in the picker.
    // W9 is entered on more than one floor.
    for floor in &scene.stack {
        let mut expected: Vec<&str> = record
            .entrances
            .iter()
            .filter(|entrance| entrance.level_id.as_deref() == Some(floor.level_id.as_str()))
            .map(|entrance| entrance.id.as_str())
            .collect();
        assert_eq!(floor.entered, !expected.is_empty(), "{}", floor.name);
        if !floor.shown {
            expected.clear();
        }
        let mut drawn: Vec<&str> = floor
            .entrances
            .iter()
            .map(|at| at.entrance_id.as_str())
            .collect();
        expected.sort_unstable();
        drawn.sort_unstable();
        assert_eq!(drawn, expected, "{}", floor.name);
    }
    assert!(scene.stack.iter().filter(|floor| floor.entered).count() > 1);
    assert_eq!(
        scene
            .levels
            .iter()
            .filter(|level| level.entered)
            .map(|level| &level.id)
            .collect::<Vec<_>>(),
        scene
            .stack
            .iter()
            .rev()
            .filter(|floor| floor.entered)
            .map(|floor| &floor.level_id)
            .collect::<Vec<_>>()
    );
    // Turning the stack is no reason to leave the building.
    let campus = map.campus_bounds("ookayama").expect("ookayama");
    look(&mut page, &view(campus));
    assert_eq!(page.floor, Some(shown));
    // Another floor, then one floor again: the room's floor comes forward,
    // the room still lit; and into the stack again on it.
    let other = record
        .levels
        .iter()
        .find(|level| level.id != record.levels[shown.1].id)
        .expect("W9 has several floors");
    send(
        &mut page,
        CampusMapIntent::Level {
            id: other.id.clone(),
        },
    );
    assert_ne!(page.floor, Some(shown));
    let flat = send(&mut page, CampusMapIntent::Stack { on: false });
    assert!(!flat.stacked && flat.stack.is_empty());
    assert_eq!(page.floor, Some(shown));
    assert!(flat
        .layers
        .iter()
        .any(|layer| layer.role == CampusMapLayerRole::Highlight && !layer.polygons.is_empty()));
    let again = send(&mut page, CampusMapIntent::Stack { on: true });
    assert!(again.stacked);
    assert_eq!(page.floor, Some(shown));
    // With no room kept, picking the building goes to its street floor,
    // whichever floor was shown.
    send(&mut page, CampusMapIntent::Clear);
    let off_street = record
        .levels
        .iter()
        .position(|level| level.id != record.levels[street].id)
        .expect("W9 has several floors");
    send(
        &mut page,
        CampusMapIntent::Level {
            id: record.levels[off_street].id.clone(),
        },
    );
    assert_eq!(page.floor, Some((w9, off_street)));
    select(&mut page, building(W9));
    assert_eq!(page.floor, Some((w9, street)));
    // Panning the stack onto another building brings it into focus, as on
    // one floor: the stack moves to its street floor.
    let wl1 = map.buildings[WL1];
    let wl1_street = map.street_level(wl1).expect("WL1's street floor");
    let center = map.export.buildings[wl1].center;
    let moved = look(
        &mut page,
        &view_at(&map, WL1, center, FOCUS_ENTER_SHARE * 1.1),
    );
    assert!(moved.stacked);
    assert_eq!(page.floor, Some((wl1, wl1_street)));
    assert!(moved
        .stack
        .iter()
        .any(|floor| floor.shown
            && floor.level_id == map.export.buildings[wl1].levels[wl1_street].id));
    // Zoomed out over no building it stays; back over W9, its street floor.
    look(&mut page, &view(campus));
    assert_eq!(page.floor.map(|(building, _)| building), Some(wl1));
    let center = map.export.buildings[w9].center;
    look(
        &mut page,
        &view_at(&map, W9, center, FOCUS_ENTER_SHARE * 1.1),
    );
    assert_eq!(page.floor, Some((w9, street)));
}
