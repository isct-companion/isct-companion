//! The page's rules over the published export, by fixed real places. The
//! file is not in this repository: `ISCT_CAMPUS_MAP_EXPORT` names a
//! downloaded `campus-map.json.gz`, the pinned one the `campus-map` workflow
//! fetches, so these run only with `--ignored`.
use super::*;
use crate::campus_map::{
    export,
    geometry::Rect,
    map::{FOCUS_ENTER_SHARE, FOCUS_EXIT_SHARE},
    CampusMapBounds,
};

/// 大岡山西9号館.
const W9: &str = "a10026fc-4e59-55a7-9d89-0193f1749620";
/// 大岡山西講義棟1, entered on 3F, up the slope from W9.
const WL1: &str = "95e63ef0-8ebc-5502-87ed-1f414753ce93";
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
fn small_zooms_near_the_threshold_never_flicker_the_floor() {
    let map = map();
    let w9 = building(&map, W9);
    let center = map.export.buildings[w9].center;
    assert_eq!(map.building_containing(center, true), Some(w9));
    let ground = map.street_level(w9).map(|level| (w9, level));

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

    // The shares sit around the tuned thresholds, so retuning them keeps
    // this passing while the gap between entering and leaving stays.
    let (enter, exit) = (FOCUS_ENTER_SHARE, FOCUS_EXIT_SHARE);
    let at = |share: f64| view_at(&map, w9, center, share);
    assert_eq!(map.focus(&at(enter * 1.1), None), ground);
    // Between the two thresholds a small zoom changes nothing either way.
    let between = (enter + exit) / 2.0;
    assert_eq!(map.focus(&at(between), None), None);
    assert_eq!(map.focus(&at(between), ground), ground);
    assert_eq!(map.focus(&at(exit * 0.9), ground), None);
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
        stacks: false,
    };
    let mut page = Page::open(map.clone(), preferences, Some("W9-202"));
    // A room opens as the stack; these are the rules of the flat floor.
    assert!(page.stacked);
    page.apply(CampusMapIntent::Stack { on: false });
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

    // A footprint tap on the already-focused building must frame too.
    // Find an unlabelled part of the real footprint rather than inventing
    // a place or accidentally exercising the named-unit selection path.
    let [west, south, east, north] = map.building_bounds(w9);
    let (_, level) = room_floor.expect("the room's floor");
    let at = (1..100)
        .flat_map(|x| (1..100).map(move |y| (x, y)))
        .map(|(x, y)| CampusMapPoint {
            longitude: west + (east - west) * f64::from(x) / 100.0,
            latitude: south + (north - south) * f64::from(y) / 100.0,
        })
        .find(|at| {
            map.building_containing([at.longitude, at.latitude], false) == Some(w9)
                && map.unit_at(w9, level, *at).is_none()
        })
        .expect("an unlabelled part of W9's footprint");
    let before = key(&page);
    page.apply(CampusMapIntent::Tap {
        point: at,
        level_id: Some(map.export.buildings[w9].levels[level].id.clone()),
    });
    assert_eq!(page.selection, Selection::Building(w9));
    assert_eq!(key(&page), before + 1);
    assert_eq!(page.arrival, Some(room_floor));
    let tapped_framing = page.framing.clone().expect("tap framing");
    // Search and footprint taps issue the same command for the same place.
    page.apply(CampusMapIntent::Select {
        place: CampusMapPlace::Building { id: W9.to_owned() },
    });
    assert_eq!(
        page.framing.as_ref().expect("search framing").bounds,
        tapped_framing.bounds
    );
    assert_eq!(page.floor, room_floor);
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

#[test]
#[ignore = "needs the published export in ISCT_CAMPUS_MAP_EXPORT"]
fn a_stack_lifts_every_floor_off_the_ground() {
    let map = map();
    let w9 = building(&map, W9);
    let preferences = Preferences {
        english: false,
        maps: MapsApp::Google,
        stacks: true,
    };
    let mut page = Page::open(map.clone(), preferences, Some("W9-324"));
    // A room on the plans opens as the stack, on the room's floor, framed
    // whole; a building alone does not.
    let opened = page.render(1);
    assert!(opened.stacked);
    assert!(opened
        .stack
        .iter()
        .any(|floor| floor.shown && !floor.highlight.is_empty()));
    assert!(!Page::open(map.clone(), preferences, Some("W9")).stacked);
    assert!(!Page::open(map.clone(), preferences, None).stacked);
    let framing = page.framing.clone().expect("a framing");
    page.apply(CampusMapIntent::Camera {
        camera: camera(&view(rect(framing.bounds)), false),
        framing: framing.key,
    });
    let shown = page.floor.expect("the room's floor");
    // Looking at another floor first: the stack still brings the room's
    // floor forward, so the room stands out.
    let other = map.export.buildings[w9]
        .levels
        .iter()
        .position(|level| level.id != map.export.buildings[w9].levels[shown.1].id)
        .expect("W9 has several floors");
    page.apply(CampusMapIntent::Level {
        id: map.export.buildings[w9].levels[other].id.clone(),
    });
    assert_ne!(page.floor, Some(shown));
    page.apply(CampusMapIntent::Stack { on: true });
    let framing = page.framing.clone().expect("the stack's framing");
    page.apply(CampusMapIntent::Camera {
        camera: camera(&view(rect(framing.bounds)), false),
        framing: framing.key,
    });
    assert_eq!(page.floor, Some(shown));
    let scene = page.render(1);
    assert!(scene.stacked);
    let levels = &map.export.buildings[w9].levels;
    // Every floor from the lowest to the highest, the ones between without
    // a plan (W9 has 1F to 3F and 7F) only an outline, listed in the
    // picker but not to be picked.
    let lowest = levels
        .iter()
        .map(|level| level.ordinal)
        .min()
        .expect("floors");
    let highest = levels
        .iter()
        .map(|level| level.ordinal)
        .max()
        .expect("floors");
    assert_eq!(scene.stack.len(), (highest - lowest + 1) as usize);
    let gaps: Vec<_> = scene.stack.iter().filter(|floor| !floor.has_plan).collect();
    assert_eq!(gaps.len(), scene.stack.len() - levels.len());
    assert!(!gaps.is_empty());
    assert!(gaps.iter().all(|floor| !floor.shown
        && !floor.outline.is_empty()
        && floor.rooms.is_empty()
        && floor.highlight.is_empty()
        && !map.levels.contains_key(&floor.level_id)));
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
    let before = page.floor;
    page.apply(CampusMapIntent::Level {
        id: gaps[0].level_id.clone(),
    });
    assert_eq!(page.floor, before);
    // Every street floor on the ground: W9's main entrance is on 2F,
    // WL1's up the slope on 3F. W9's 1F is under the ground.
    let wl1 = building(&map, WL1);
    let street = |index: usize| {
        let record = &map.export.buildings[index];
        let level = map.street_level(index).expect("floors");
        page.base_m(index, record.levels[level].ordinal)
    };
    assert_eq!(
        map.export.buildings[w9].levels[map.street_level(w9).expect("floors")].name,
        "2F"
    );
    assert_eq!(
        map.export.buildings[wl1].levels[map.street_level(wl1).expect("floors")].name,
        "3F"
    );
    assert_eq!(street(w9), 0.0);
    assert_eq!(street(wl1), 0.0);
    assert!(scene
        .stack
        .iter()
        .any(|floor| floor.name == "1F" && floor.base_m < 0.0));
    // Lowest first, each above the last, the shown floor solid.
    assert!(scene
        .stack
        .windows(2)
        .all(|pair| pair[0].base_m < pair[1].base_m));
    assert_eq!(scene.stack.iter().filter(|floor| floor.shown).count(), 1);
    assert!(scene
        .stack
        .iter()
        .any(|floor| floor.shown && floor.level_id == levels[shown.1].id));
    assert!(scene
        .stack
        .iter()
        .any(|floor| floor.shown && !floor.highlight.is_empty()));
    // Only the floor picked carries entrances, exactly the ones the export
    // puts on it; picking another floor shows that floor's own. W9 is
    // entered on more than one floor.
    let entrances_on = |scene: &CampusMapScene| {
        scene
            .stack
            .iter()
            .filter(|floor| !floor.entrances.is_empty())
            .map(|floor| {
                (
                    floor.shown,
                    floor.level_id.clone(),
                    floor
                        .entrances
                        .iter()
                        .map(|at| at.entrance_id.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
    };
    let expected = |level: &str| {
        map.export.buildings[w9]
            .entrances
            .iter()
            .filter(|entrance| entrance.level_id.as_deref() == Some(level))
            .map(|entrance| entrance.id.clone())
            .collect::<Vec<_>>()
    };
    let shown_id = levels[shown.1].id.clone();
    let on = entrances_on(&scene);
    if expected(&shown_id).is_empty() {
        assert!(on.is_empty());
    } else {
        assert_eq!(on, vec![(true, shown_id.clone(), expected(&shown_id))]);
    }
    let entered = levels
        .iter()
        .find(|level| level.id != shown_id && !expected(&level.id).is_empty())
        .expect("W9 is entered on another floor too");
    page.apply(CampusMapIntent::Level {
        id: entered.id.clone(),
    });
    assert_eq!(
        entrances_on(&page.render(1)),
        vec![(true, entered.id.clone(), expected(&entered.id))]
    );
    // Nothing of a floor is left on the ground: no plan layers, and no
    // marks; the entrances are on their floors.
    assert!(scene.layers.iter().all(|layer| matches!(
        layer.role,
        CampusMapLayerRole::Footprint | CampusMapLayerRole::FocusedFootprint
    )));
    assert!(scene.marks.is_empty());
    // Turning the stack is no reason to leave the building.
    let campus = map.campus_bounds("ookayama").expect("ookayama");
    page.apply(CampusMapIntent::Camera {
        camera: camera(&view(campus), true),
        framing: page.framing.as_ref().map_or(0, |framing| framing.key),
    });
    assert_eq!(page.floor.map(|(building, _)| building), Some(w9));
    // Back on the flat map, from another floor of the stack: the room is
    // still selected, its floor comes forward and it is drawn lit; the
    // camera settling from the angle (not the person's move) keeps it.
    page.apply(CampusMapIntent::Level {
        id: entered.id.clone(),
    });
    assert_ne!(page.floor, Some(shown));
    page.apply(CampusMapIntent::Stack { on: false });
    assert_eq!(page.floor, Some(shown));
    page.apply(CampusMapIntent::Camera {
        camera: camera(&view(map.building_bounds(w9)), false),
        framing: page.framing.as_ref().map_or(0, |framing| framing.key),
    });
    assert_eq!(page.floor, Some(shown));
    let flat = page.render(1);
    assert!(!flat.stacked);
    assert!(flat
        .layers
        .iter()
        .any(|layer| layer.role == CampusMapLayerRole::Highlight && !layer.polygons.is_empty()));
    // And into the stack again, the room's floor forward.
    page.apply(CampusMapIntent::Stack { on: true });
    assert_eq!(page.floor, Some(shown));
    let lit = |page: &Page| {
        page.render(1)
            .stack
            .iter()
            .any(|floor| floor.shown && !floor.highlight.is_empty())
    };
    assert!(lit(&page));
    // Another building and back: the room is kept, and W9 opens on its
    // floor with the room lit, the picker marking that floor.
    page.apply(CampusMapIntent::Select {
        place: CampusMapPlace::Building { id: WL1.to_owned() },
    });
    let framing = page.framing.clone().expect("WL1's framing");
    page.apply(CampusMapIntent::Camera {
        camera: camera(&view(rect(framing.bounds)), false),
        framing: framing.key,
    });
    assert_eq!(page.floor.map(|(building, _)| building), Some(wl1));
    assert!(page.render(1).stacked);
    page.apply(CampusMapIntent::Select {
        place: CampusMapPlace::Building { id: W9.to_owned() },
    });
    let framing = page.framing.clone().expect("W9's framing");
    page.apply(CampusMapIntent::Camera {
        camera: camera(&view(rect(framing.bounds)), false),
        framing: framing.key,
    });
    assert_eq!(page.floor, Some(shown));
    assert!(lit(&page));
    assert_eq!(page.marked_level(), Some(shown.1));
    // Closing the card lets it go.
    page.apply(CampusMapIntent::Clear);
    assert!(!lit(&page));
}
