//! The page's rules over the published export, by fixed real places, through
//! its contract: intents in, the rendered scene out. The file is not in this
//! repository: `ISCT_CAMPUS_MAP_EXPORT` names a downloaded
//! `campus-map.json.gz`, the pinned one the `campus-map` workflow fetches, so
//! these run only with `--ignored`.
use super::*;
use crate::locale::ContentLanguage;
use crate::campus_map::{
    export,
    geometry::Rect,
    map::{FOCUS_ENTER_SHARE, FOCUS_EXIT_SHARE},
};

/// 大岡山西9号館.
const W9: &str = "a10026fc-4e59-55a7-9d89-0193f1749620";
/// 屋内運動場, south-west of W9 and smaller.
const SPORTS_CENTER: &str = "3a605ae0-88df-4574-aadf-56da13339a23";
/// Inside W9's footprint, 6 m from the sports center's.
const BY_THE_SPORTS_CENTER: [f64; 2] = [139.6821654, 35.6057532];
const PREFERENCES: Preferences = Preferences {
    language: ContentLanguage::Ja,
    maps: MapsApp::Apple,
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
    let tapped = tap(&mut page, point, None, 2.0);
    let w9 = map.buildings[W9];
    assert_eq!(
        tapped.card.and_then(|card| card.title),
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
    let title = |scene: &CampusMapScene| scene.card.as_ref().and_then(|card| card.title.clone());
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
    let tapped = tap(&mut page, at, room_floor.clone(), 0.1);
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
    assert_eq!(title(&sports), Some(map.name(sports_center, ContentLanguage::Ja)));
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
