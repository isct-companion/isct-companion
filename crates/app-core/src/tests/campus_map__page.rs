//! The page's rules over the published export, by fixed real places. The
//! file is not in this repository: `ISCT_CAMPUS_MAP_EXPORT` names a
//! downloaded `campus-map.json.gz`, the pinned one the `campus-map` workflow
//! fetches, so these run only with `--ignored`.
use super::*;
use crate::campus_map::{export, geometry::Rect, CampusMapBounds};

/// 大岡山西9号館.
const W9: &str = "a10026fc-4e59-55a7-9d89-0193f1749620";
/// 屋内運動場, south-west of W9 and smaller.
const SPORTS_CENTER: &str = "3a605ae0-88df-4574-aadf-56da13339a23";
/// Inside W9's footprint, 6 m from the sports center's.
const BY_THE_SPORTS_CENTER: [f64; 2] = [139.6821654, 35.6057532];

fn map() -> Arc<Map> {
    let path = std::env::var("ISCT_CAMPUS_MAP_EXPORT").unwrap_or_else(|_| {
        panic!("ISCT_CAMPUS_MAP_EXPORT must name the published campus-map.json.gz")
    });
    let gz = std::fs::read(&path).unwrap_or_else(|error| panic!("cannot read {path}: {error}"));
    Arc::new(Map::new(export::read(&gz).expect("format 1")).expect("a valid export"))
}

fn building(map: &Map, id: &str) -> usize {
    map.buildings[id]
}

/// A view of `rect`, upright.
fn view(rect: Rect) -> Viewport {
    let [west, south, east, north] = rect;
    Viewport::new(
        [[west, north], [east, north], [east, south], [west, south]],
        390.0,
    )
    .expect("a view with area")
}

/// A square view centred on `at` in which `building` spans `share`.
fn view_at(map: &Map, building: usize, at: [f64; 2], share: f64) -> Viewport {
    let footprint = &map.export.buildings[building].footprint;
    let probe = view(geometry::at_least([at[0], at[1], at[0], at[1]], 100.0));
    let side = 100.0 * probe.share(footprint) / share;
    view(geometry::at_least([at[0], at[1], at[0], at[1]], side))
}

fn camera(view: &Viewport, by_user: bool) -> CampusMapCamera {
    let corner = |index: usize| CampusMapPoint {
        latitude: view.ring[index][1],
        longitude: view.ring[index][0],
    };
    CampusMapCamera {
        top_left: corner(0),
        top_right: corner(1),
        bottom_right: corner(2),
        bottom_left: corner(3),
        width_points: 390.0,
        height_points: 390.0,
        by_user,
    }
}

fn rect(bounds: CampusMapBounds) -> Rect {
    [bounds.west, bounds.south, bounds.east, bounds.north]
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn a_building_comes_into_focus_at_half_the_view_and_leaves_below_a_third() {
    let map = map();
    let w9 = building(&map, W9);
    let center = map.export.buildings[w9].center;
    assert_eq!(map.building_containing(center, true), Some(w9));
    let ground = map.ground_level(w9).map(|level| (w9, level));

    // The whole campus in view, its centre inside W9: no floor.
    let campus = map.campus_bounds("ookayama").expect("ookayama");
    let (half_width, half_height) = ((campus[2] - campus[0]) / 2.0, (campus[3] - campus[1]) / 2.0);
    let whole = view([
        center[0] - half_width,
        center[1] - half_height,
        center[0] + half_width,
        center[1] + half_height,
    ]);
    assert_eq!(map.focus(&whole, None), None);

    assert_eq!(map.focus(&view_at(&map, w9, center, 0.6), None), ground);
    // Between the two thresholds a small zoom changes nothing either way.
    assert_eq!(map.focus(&view_at(&map, w9, center, 0.4), None), None);
    assert_eq!(map.focus(&view_at(&map, w9, center, 0.4), ground), ground);
    assert_eq!(map.focus(&view_at(&map, w9, center, 0.25), ground), None);
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn the_building_holding_a_point_wins_over_a_smaller_one_nearby() {
    let map = map();
    let (w9, sports) = (building(&map, W9), building(&map, SPORTS_CENTER));
    let point = BY_THE_SPORTS_CENTER;
    let footprint = |index: usize| &map.export.buildings[index].footprint;
    assert!(geometry::multipolygon_contains(footprint(w9), point));
    assert!(geometry::distance_to_edges(footprint(sports), point) < 10.0);
    assert!(
        geometry::multipolygon_area(footprint(sports)) < geometry::multipolygon_area(footprint(w9))
    );

    assert_eq!(map.building_tapped(point, 20.0), Some(w9));
    let focused = map.focus(&view_at(&map, w9, point, 0.8), None);
    assert_eq!(focused.map(|(building, _)| building), Some(w9));
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn search_finds_buildings_by_the_codes_in_the_data() {
    let map = map();
    let first = |query: &str| {
        search(&map, false, query)
            .into_iter()
            .next()
            .map(|result| result.place)
    };
    for code in ["MI1", "MI3", "W9"] {
        let id = map.export.buildings[map
            .export
            .buildings
            .iter()
            .position(|building| building.codes.iter().any(|known| known == code))
            .expect(code)]
        .id
        .clone();
        assert_eq!(first(code), Some(CampusMapPlace::Building { id }), "{code}");
    }
    let room = &map.export.rooms[map
        .export
        .rooms
        .iter()
        .position(|room| room.code == "W9-202")
        .expect("W9-202")];
    assert_eq!(
        first("w9-202"),
        Some(CampusMapPlace::Room {
            id: room.id.clone()
        })
    );
    assert!(!map.locate("MI1", false).is_empty());
}

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn old_camera_reports_and_floor_changes_never_replace_the_selection() {
    let map = map();
    let (w9, sports) = (building(&map, W9), building(&map, SPORTS_CENTER));
    let preferences = Preferences {
        english: false,
        maps: MapsApp::Apple,
    };
    let mut page = Page::open(map.clone(), preferences, Some("W9-202"));
    let key = |page: &Page| page.framing.as_ref().map_or(0, |framing| framing.key);
    let land = |page: &mut Page| {
        let framing = page.framing.clone().expect("a framing");
        let landed = view(rect(framing.bounds));
        page.apply(CampusMapIntent::Camera {
            camera: camera(&landed, false),
            framing: framing.key,
        });
    };
    let room_floor = page.targets[0].level.map(|level| (w9, level));
    assert!(room_floor.is_some());
    land(&mut page);
    assert_eq!(page.floor, room_floor);

    // Tapping the sports center from afar: a report the shell made before
    // it applied that framing does not pull the floor back to W9's.
    let before = key(&page);
    page.apply(CampusMapIntent::Select {
        place: CampusMapPlace::Building {
            id: SPORTS_CENTER.to_owned(),
        },
    });
    let center = map.export.buildings[w9].center;
    page.apply(CampusMapIntent::Camera {
        camera: camera(&view_at(&map, w9, center, 1.0), true),
        framing: before,
    });
    assert_eq!(page.floor, room_floor);
    land(&mut page);
    assert_eq!(page.floor.map(|(building, _)| building), Some(sports));
    assert_eq!(
        page.card().and_then(|card| card.title),
        Some(map.name(sports, false))
    );

    // The room again: another floor, then the whole campus, keep it selected.
    let room = map
        .export
        .rooms
        .iter()
        .find(|room| room.code == "W9-202")
        .expect("W9-202");
    page.apply(CampusMapIntent::Select {
        place: CampusMapPlace::Room {
            id: room.id.clone(),
        },
    });
    land(&mut page);
    assert_eq!(page.floor, room_floor);
    let other = map.export.buildings[w9]
        .levels
        .iter()
        .find(|level| {
            Some(level.ordinal)
                != room_floor.map(|(_, at)| map.export.buildings[w9].levels[at].ordinal)
        })
        .expect("W9 has several floors");
    page.apply(CampusMapIntent::Level {
        id: other.id.clone(),
    });
    assert_ne!(page.floor, room_floor);
    let campus = map.campus_bounds("ookayama").expect("ookayama");
    page.apply(CampusMapIntent::Camera {
        camera: camera(&view(campus), true),
        framing: key(&page),
    });
    assert_eq!(page.floor, None);
    let card = page.card().expect("still selected");
    assert_eq!(card.title.as_deref(), Some("W9-202"));
    assert_eq!(page.marked_level(), None);
}
