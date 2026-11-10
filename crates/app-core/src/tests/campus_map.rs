//! The map's rules over a small hand-built export (format 1): matching,
//! focus, floors and taps. These are rules, not scraped data; the one test
//! over a real export is ignored and reads its path from the environment.
//! The parser's table needs no map at all.
use super::{
    export,
    map::Map,
    matching::{alias_key, Patterns},
    *,
};
use serde_json::{json, Value};
use std::io::Write;

/// A closed ring around a box.
fn square(west: f64, south: f64, east: f64, north: f64) -> Value {
    json!([[[west, south], [east, south], [east, north], [west, north], [west, south]]])
}

const W9: [f64; 4] = [139.6840, 35.6050, 139.6851, 35.6059];
/// Inside W9's north-east corner: overlapping footprints, the smaller wins.
const ANNEX: [f64; 4] = [139.6848, 35.6056, 139.6851, 35.6059];
const I1: [f64; 4] = [139.6900, 35.6100, 139.6905, 35.6104];
const GYM: [f64; 4] = [139.6880, 35.6080, 139.6884, 35.6083];

fn sq(rect: [f64; 4]) -> Value {
    square(rect[0], rect[1], rect[2], rect[3])
}

fn unit(id: &str, level: &str, category: &str, room: Option<&str>, label: Option<&str>, rect: [f64; 4]) -> Value {
    json!({
        "id": id, "levelId": level, "category": category, "roomId": room, "label": label,
        "center": [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0],
        "polygon": sq(rect)
    })
}

/// Four buildings: W9 (B1, 1F, 2F, one entrance), an annex inside W9's
/// corner (1F, 3F), I1 without a ground floor (B1, 2F) and a gym without
/// floor plans or entrances. Every unit and amenity category appears once.
pub(crate) fn sample() -> Value {
    let amenities: Vec<Value> = [
        "elevator", "stairs", "restroom", "restroom.male", "restroom.female",
        "restroom.wheelchair", "aed", "lounge", "lounge.women", "lounge.international",
        "rest_room", "calm_down", "study_room",
    ]
    .iter()
    .enumerate()
    .map(|(index, category)| {
        json!({"id": format!("a-{index}"), "levelId": "w9-2", "category": category,
               "point": [139.6846, 35.6056]})
    })
    .collect();
    let mut units = vec![
        unit("u-202", "w9-2", "classroom", Some("r-202"), Some("W9-202"), [139.6842, 35.6052, 139.6844, 35.6054]),
        unit("u-corridor", "w9-2", "corridor", None, Some("2F廊下"), W9),
        unit("u-b07", "w9-b1", "classroom", Some("r-b07"), Some("W9-B07"), [139.6842, 35.6052, 139.6843, 35.6053]),
        unit("u-112", "w9-1", "room", Some("r-112"), Some("W9-112"), [139.6845, 35.6052, 139.6846, 35.6053]),
        unit("u-s4-112", "an-1", "classroom", Some("r-s4-112"), Some("S4-112"), [139.6849, 35.6057, 139.6850, 35.6058]),
    ];
    for (index, category) in ["elevator_lobby", "restroom", "stairs", "lounge", "porch"].iter().enumerate() {
        let west = 139.6841 + index as f64 * 0.0001;
        units.push(unit(&format!("u-{category}"), "w9-1", category, None, None, [west, 35.6056, west + 0.00005, 35.60565]));
    }
    json!({
        "formatVersion": 1,
        "attribution": ["© OpenStreetMap contributors (ODbL)"],
        "buildings": [
            {
                "id": "w9", "campus": "ookayama", "name": {"ja": "西9号館", "en": "West Bldg. 9"},
                "codes": ["W9"], "center": [139.68455, 35.60545], "footprint": [sq(W9)],
                "levels": [
                    {"id": "w9-b1", "ordinal": -1, "name": "B1", "outline": [sq(W9)]},
                    {"id": "w9-1", "ordinal": 0, "name": "1F", "outline": [sq(W9)]},
                    {"id": "w9-2", "ordinal": 1, "name": "2F", "outline": [sq(W9)]}
                ],
                "entrances": [
                    {"id": "w9-main", "point": [139.68455, 35.6050], "name": {"ja": "正面玄関"}, "levelId": "w9-1"}
                ]
            },
            {
                "id": "annex", "campus": "ookayama", "name": {"ja": "南4号館"},
                "codes": ["S4"], "center": [139.68495, 35.60575], "footprint": [sq(ANNEX)],
                "levels": [
                    {"id": "an-1", "ordinal": 0, "name": "1F", "outline": [sq(ANNEX)]},
                    {"id": "an-3", "ordinal": 2, "name": "3F", "outline": [sq(ANNEX)]}
                ],
                "entrances": []
            },
            {
                "id": "i1", "campus": "ookayama", "name": {"ja": "石川台1号館"},
                "codes": ["I1"], "center": [139.69025, 35.6102], "footprint": [sq(I1)],
                "levels": [
                    {"id": "i1-b1", "ordinal": -1, "name": "B1", "outline": [sq(I1)]},
                    {"id": "i1-2", "ordinal": 1, "name": "2F", "outline": [sq(I1)]}
                ],
                "entrances": [{"id": "i1-door", "point": [139.6900, 35.6102], "name": null, "levelId": null}]
            },
            {
                "id": "gym", "campus": "ookayama", "name": {"ja": "体育館", "en": "Gymnasium"},
                "codes": [], "center": [139.6882, 35.60815], "footprint": [sq(GYM)],
                "levels": [], "entrances": []
            }
        ],
        "units": units,
        "amenities": amenities,
        "linework": [
            {"levelId": "w9-2", "tone": "wall", "widthM": 0.15, "lines": [[[139.6840, 35.6050], [139.6851, 35.6050]]]},
            {"levelId": "w9-2", "tone": "detail", "widthM": 0.0, "lines": [[[139.6842, 35.6052], [139.6844, 35.6052]]]}
        ],
        "rooms": [
            {"id": "r-202", "code": "W9-202", "buildingId": "w9", "unitIds": ["u-202"]},
            {"id": "r-b07", "code": "W9-B07", "buildingId": "w9", "unitIds": ["u-b07"]},
            {"id": "r-112", "code": "W9-112", "buildingId": "w9", "unitIds": ["u-112"]},
            {"id": "r-s4-112", "code": "S4-112", "buildingId": "annex", "unitIds": ["u-s4-112"]},
            {"id": "r-901", "code": "W9-901", "buildingId": "w9", "unitIds": []}
        ],
        "aliases": {
            "M112": ["r-112", "r-s4-112"],
            "S4-112": ["r-s4-112"],
            "W9-112": ["r-112"],
            "W9-202": ["r-202"],
            "W9-901": ["r-901"],
            "W9-B07": ["r-b07"]
        }
    })
}

pub(crate) fn gzip(value: &Value) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&serde_json::to_vec(value).unwrap()).unwrap();
    encoder.finish().unwrap()
}

fn map() -> Map {
    Map::new(export::parse(&serde_json::to_vec(&sample()).unwrap()).unwrap()).unwrap()
}

fn labels(targets: &[CampusMapTarget]) -> Vec<&str> {
    targets.iter().map(|target| target.label.as_str()).collect()
}

/// A camera at `center`, zoomed to about 18 (or 15.5 when `far`), on a
/// 390-point-wide map.
fn camera(center: [f64; 2], far: bool, by_user: bool, building: Option<&str>, level: Option<&str>) -> CampusMapCamera {
    let (dx, dy) = if far { (0.006, 0.0048) } else { (0.001, 0.0008) };
    CampusMapCamera {
        center: CampusMapPoint { latitude: center[1], longitude: center[0] },
        bounds: CampusMapBounds {
            south: center[1] - dy,
            west: center[0] - dx,
            north: center[1] + dy,
            east: center[0] + dx,
        },
        width_points: 390.0,
        by_user,
        building_id: building.map(str::to_owned),
        level_id: level.map(str::to_owned),
    }
}

fn shown(focus: &CampusMapFocus) -> (Option<&str>, Option<&str>) {
    (focus.building_id.as_deref(), focus.level_id.as_deref())
}

#[test]
fn alias_spellings_normalize_and_an_ambiguous_one_lists_every_room() {
    let map = map();
    // NFKC, whitespace removed, upper case: the exporter's normalization.
    assert_eq!(alias_key(" ｗ９－２０２ "), "W9-202");
    assert_eq!(alias_key("m\u{3000}112"), "M112");
    for spelling in ["W9-202", "ｗ９－２０２", "w9 - 202", "西9号館202"] {
        let targets = map.locate(spelling, false);
        assert_eq!(labels(&targets), ["W9-202"], "{spelling}");
        assert_eq!(targets[0].matched, CampusMapMatch::Room);
        assert_eq!(targets[0].highlighted_unit_ids, ["u-202"]);
    }
    let both = map.locate("M112", false);
    assert_eq!(labels(&both), ["W9-112", "S4-112"]);
    assert_eq!(
        both.iter().map(|target| target.building_id.as_str()).collect::<Vec<_>>(),
        ["w9", "annex"]
    );
    // The parser splits what the table does not know whole, in order.
    assert_eq!(labels(&map.locate("W9-B07, W9-202", false)), ["W9-B07", "W9-202"]);
}

#[test]
fn a_room_off_the_plans_shows_its_building_and_unknown_text_shows_nothing() {
    let map = map();
    let off = map.locate("W9-999", false);
    assert_eq!(labels(&off), ["W9-999"]);
    assert_eq!(off[0].matched, CampusMapMatch::Building);
    assert!(off[0].highlighted_unit_ids.is_empty());
    // No floor named: the ground floor.
    assert_eq!(off[0].level_id.as_deref(), Some("w9-1"));
    // A floor named: that one.
    assert_eq!(map.locate("W9-999 2階", false)[0].level_id.as_deref(), Some("w9-2"));
    // A room the export knows only by its building.
    let unplanned = map.locate("W9-901", false);
    assert_eq!(unplanned[0].matched, CampusMapMatch::Building);
    assert_eq!(unplanned[0].room_id.as_deref(), Some("r-901"));
    // A building already shown for a room gets no building-only row.
    assert_eq!(labels(&map.locate("W9-202, W9-999", false)), ["W9-202"]);
    // By name, in the content language; a building without plans has no floor.
    let gym = map.locate("体育館", true);
    assert_eq!(gym[0].building_name, "Gymnasium");
    assert!(gym[0].levels.is_empty() && gym[0].level_id.is_none());
    for nothing in ["オンライン", "", "Zoom"] {
        assert!(map.locate(nothing, false).is_empty(), "{nothing}");
    }
}

#[test]
fn entry_opens_on_the_highlighted_rooms_floor_framed_on_the_room() {
    let map = map();
    let basement = &map.locate("W9-B07", false)[0];
    assert_eq!(basement.level_id.as_deref(), Some("w9-b1"));
    let upstairs = &map.locate("W9-202", false)[0];
    assert_eq!(upstairs.level_id.as_deref(), Some("w9-2"));
    assert_eq!(
        upstairs.focus,
        CampusMapBounds { south: 35.6052, west: 139.6842, north: 35.6054, east: 139.6844 }
    );
    assert_eq!(
        upstairs.levels.iter().map(|level| level.name.as_str()).collect::<Vec<_>>(),
        ["B1", "1F", "2F"]
    );
}

#[test]
fn focus_follows_the_camera_centre_with_the_smaller_building_winning() {
    let map = map();
    let w9_centre = [139.6843, 35.6052];
    let annex_centre = [139.68495, 35.60575];
    // Too far out: nothing, whatever was in focus.
    assert_eq!(shown(&map.focus(&camera(w9_centre, true, true, Some("w9"), Some("w9-2")))), (None, None));
    // Entering from nothing: the ground floor.
    let entered = map.focus(&camera(w9_centre, false, true, None, None));
    assert_eq!(shown(&entered), (Some("w9"), Some("w9-1")));
    assert_eq!(entered.levels.len(), 3);
    // Overlapping footprints: the smaller one.
    assert_eq!(shown(&map.focus(&camera(annex_centre, false, true, None, None))).0, Some("annex"));
    // Within about 10 m outside a footprint still picks it; 15 m does not.
    let five_south = [139.68455, 35.6050 - 5.0 / 110_574.0];
    let fifteen_south = [139.68455, 35.6050 - 15.0 / 110_574.0];
    assert_eq!(shown(&map.focus(&camera(five_south, false, true, None, None))).0, Some("w9"));
    assert_eq!(shown(&map.focus(&camera(fifteen_south, false, true, None, None))).0, None);
    // Outside every footprint, the current building stays while it is in view.
    let kept = map.focus(&camera(fifteen_south, false, true, Some("w9"), Some("w9-2")));
    assert_eq!(shown(&kept), (Some("w9"), Some("w9-2")));
    let away = [139.6870, 35.6030];
    assert_eq!(shown(&map.focus(&camera(away, false, true, Some("w9"), Some("w9-2")))), (None, None));
    // A move the app made never re-picks.
    let own = map.focus(&camera(w9_centre, false, false, Some("annex"), Some("an-3")));
    assert_eq!(shown(&own), (Some("annex"), Some("an-3")));
    assert_eq!(shown(&map.focus(&camera(away, true, false, None, None))), (None, None));
    // A building without floor plans is never focused.
    assert_eq!(shown(&map.focus(&camera([139.6882, 35.60815], false, true, None, None))).0, None);
}

#[test]
fn switching_building_keeps_the_ordinal_else_the_floor_nearest_the_ground() {
    let map = map();
    let annex = [139.68495, 35.60575];
    let w9 = [139.6843, 35.6052];
    let i1 = [139.69025, 35.6102];
    // 1F (0) to the annex's 1F (0).
    assert_eq!(shown(&map.focus(&camera(annex, false, true, Some("w9"), Some("w9-1")))).1, Some("an-1"));
    // 2F (1) has no match in the annex (0, 2): its ground floor.
    assert_eq!(shown(&map.focus(&camera(annex, false, true, Some("w9"), Some("w9-2")))).1, Some("an-1"));
    // 3F (2) has no match in W9: its ground floor.
    assert_eq!(shown(&map.focus(&camera(w9, false, true, Some("annex"), Some("an-3")))).1, Some("w9-1"));
    // No ground floor: the nearest to it, above ground on a tie (B1, 2F).
    assert_eq!(shown(&map.focus(&camera(i1, false, true, Some("w9"), Some("w9-b1")))).1, Some("i1-b1"));
    assert_eq!(shown(&map.focus(&camera(i1, false, true, None, None))).1, Some("i1-2"));
    // Staying in a building keeps the floor shown.
    assert_eq!(shown(&map.focus(&camera(w9, false, true, Some("w9"), Some("w9-b1")))).1, Some("w9-b1"));
}

#[test]
fn a_tap_hits_the_smallest_named_unit_of_the_floor() {
    let map = map();
    let at = |longitude, latitude| CampusMapPoint { latitude, longitude };
    let id = |unit: Option<CampusMapUnit>| unit.map(|unit| unit.id);
    assert_eq!(id(map.unit_at("w9-2", at(139.6843, 35.6053))).as_deref(), Some("u-202"));
    assert_eq!(id(map.unit_at("w9-2", at(139.6849, 35.6051))).as_deref(), Some("u-corridor"));
    // Unnamed units are not picked; outside every unit, nothing.
    assert_eq!(id(map.unit_at("w9-1", at(139.68412, 35.60562))), None);
    assert_eq!(id(map.unit_at("w9-2", at(139.6800, 35.6000))), None);
    assert_eq!(id(map.unit_at("nowhere", at(139.6843, 35.6053))), None);
}

#[test]
fn floors_and_buildings_carry_what_the_map_draws_and_links() {
    let map = map();
    let floor = map.floor("w9-2").unwrap();
    assert_eq!(floor.building_id, "w9");
    assert_eq!(floor.units.len(), 2);
    assert_eq!(floor.amenities.len(), 13);
    assert_eq!(
        floor.lines.iter().map(|lines| lines.tone).collect::<Vec<_>>(),
        [CampusMapLineTone::Wall, CampusMapLineTone::Detail]
    );
    assert!(map.floor("nowhere").is_none());
    let overview = map.overview(false);
    assert_eq!(overview.attribution, ["© OpenStreetMap contributors (ODbL)"]);
    let building = |id: &str| overview.buildings.iter().find(|building| building.id == id).unwrap();
    // An entrance links its own point, latitude first.
    let entrance = &building("w9").entrances[0];
    assert!(entrance.maps_url.starts_with("https://maps.apple.com/?ll=35.6050000%2C139.6845500&q="), "{}", entrance.maps_url);
    assert_eq!(entrance.name.as_deref(), Some("正面玄関"));
    // Without entrances, the building's centre.
    assert!(building("gym").maps_url.contains("ll=35.6081500%2C139.6882000"), "{}", building("gym").maps_url);
    assert!(building("gym").levels.is_empty());
}

#[test]
fn only_format_1_is_read_and_a_dangling_reference_is_refused() {
    for version in [json!(2), json!("1"), Value::Null] {
        let mut file = sample();
        file["formatVersion"] = version.clone();
        assert!(
            matches!(CampusMapFile::read(gzip(&file), "h".into()), Err(CampusMapError::Format(_))),
            "{version}"
        );
    }
    let mut missing = sample();
    missing.as_object_mut().unwrap().remove("formatVersion");
    assert!(matches!(export::parse(&serde_json::to_vec(&missing).unwrap()), Err(CampusMapError::Format(_))));
    let mut dangling = sample();
    dangling["units"][0]["levelId"] = json!("no-such-level");
    assert!(matches!(CampusMapFile::read(gzip(&dangling), "h".into()), Err(CampusMapError::Invalid(_))));
    let mut unknown = sample();
    unknown["units"][0]["category"] = json!("ballroom");
    assert!(matches!(CampusMapFile::read(gzip(&unknown), "h".into()), Err(CampusMapError::Invalid(_))));
    // Not gzip, and gzip that unpacks past the limit.
    assert!(matches!(CampusMapFile::read(b"{}".to_vec(), "h".into()), Err(CampusMapError::Invalid(_))));
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&vec![b' '; MAX_JSON_BYTES + 1]).unwrap();
    assert!(matches!(
        export::read(&encoder.finish().unwrap()),
        Err(CampusMapError::Invalid(message)) if message.contains("size limit")
    ));
    // Format 1 itself: its units are the rows a shrink counts.
    assert_eq!(CampusMapFile::read(gzip(&sample()), "h".into()).unwrap().units, 10);
}

#[test]
fn the_service_answers_nothing_until_a_file_is_stored_and_rereads_a_new_one() {
    let path = std::env::temp_dir().join(format!("isct-campus-map-{}", uuid::Uuid::new_v4()));
    let store = DataStore::initialize_dir(path).unwrap().0;
    let service = CampusMapService::new(store.clone());
    assert!(service.locate("W9-202").unwrap().is_empty());
    assert!(service.overview().unwrap().buildings.is_empty());
    assert!(matches!(service.floor("w9-2"), Err(CampusMapError::UnknownLevel(_))));
    let put = |file: &Value, hash: &str| {
        let gz = gzip(file);
        store
            .write(None, |transaction| crate::data::put_campus_map(transaction, hash, &gz, 1))
            .unwrap()
    };
    assert_eq!(put(&sample(), "first"), 1);
    assert_eq!(put(&sample(), "first"), 0);
    assert_eq!(labels(&service.locate("W9-202").unwrap()), ["W9-202"]);
    assert_eq!(service.overview().unwrap().buildings.len(), 4);
    // A new file under a new hash replaces the parsed copy.
    let mut smaller = sample();
    smaller["buildings"].as_array_mut().unwrap().retain(|building| building["id"] != "gym");
    put(&smaller, "second");
    assert_eq!(service.overview().unwrap().buildings.len(), 3);
}

/// The school's room strings, read without any map.
#[test]
fn room_strings_split_into_their_rooms() {
    let patterns = Patterns::new().unwrap();
    for (room, expected) in [
        (
            "S7-207, S7-209S8-110, S8-107, S8-B107, S8-B108S8-108,109",
            vec![
                "S7-207", "S7-209", "S8-110", "S8-107", "S8-B107", "S8-B108", "S8-108", "S8-109",
            ],
        ),
        (
            "S8-107, 110, B107, B108他",
            vec!["S8-107", "S8-110", "S8-B107", "S8-B108"],
        ),
        ("I3-301、302、303", vec!["I3-301", "I3-302", "I3-303"]),
        (
            "I1-256, I1-204, I3-301~3,310",
            vec!["I1-256", "I1-204", "I3-301", "I3-310"],
        ),
        (
            "M-B07, I3- 201A~C, I3-303",
            vec!["M-B07", "I3-201A", "I3-303"],
        ),
        ("南6号館219,211演習室", vec!["S6-219", "S6-211"]),
        ("石川台2号館318", vec!["I2-318"]),
        ("理学院 地球惑星科学系南5号館112室", vec!["S5-112"]),
        (
            "S3-215, S3-207, 南5号館S5-112",
            vec!["S3-215", "S3-207", "S5-112"],
        ),
        (
            "S2-203, 南5号館2階206, 210号室, 石川台1号館2階253, 204号室",
            vec!["S2-203", "S5-206", "S5-210", "I1-253", "I1-204"],
        ),
    ] {
        let rooms: Vec<_> = patterns
            .mentions(room)
            .into_iter()
            .filter_map(|mention| mention.room)
            .collect();
        assert_eq!(rooms, expected, "{room}");
    }
    for (room, building, level) in [
        ("南6号館2F 219演習室", "S6", "2F"),
        ("南4号館3階第2演習室", "S4", "3F"),
        ("石川台3号館2階,統合創造工房 201-A号室", "I3", "2F"),
        // The building and floor are only in the note.
        ("第2ラボ(W3,4F)", "W3", "4F"),
    ] {
        let mentions = patterns.mentions(room);
        let mention = mentions.iter().find(|m| m.building.is_some()).unwrap();
        assert_eq!(mention.building.as_deref(), Some(building), "{room}");
        assert_eq!(mention.level.as_deref(), Some(level), "{room}");
    }
    // The bracket here is part of the name; other brackets are notes.
    let mention = patterns.mentions("西8号館(E) 計算機室 E207").remove(0);
    assert_eq!(mention.building.as_deref(), Some("W8E"));
    assert_eq!(
        patterns.mentions("物理学生実験室(南5、石川台6)")[0].building,
        None
    );
}

/// Real room strings against a real export:
/// `ISCT_CAMPUS_MAP_EXPORT=/path/campus-map.json.gz cargo test -p app-core
/// --features campus-map campus_map::tests::real -- --ignored --nocapture`.
#[test]
#[ignore = "needs a real export file in ISCT_CAMPUS_MAP_EXPORT"]
fn real_room_strings_find_their_buildings_in_a_real_export() {
    let path = std::env::var("ISCT_CAMPUS_MAP_EXPORT").expect("ISCT_CAMPUS_MAP_EXPORT");
    let gz = std::fs::read(path).unwrap();
    let file = CampusMapFile::read(gz.clone(), String::new()).unwrap();
    assert!(file.units > 0);
    let map = Map::new(export::read(&gz).unwrap()).unwrap();
    let codes = |target: &CampusMapTarget| {
        map.overview(false)
            .buildings
            .into_iter()
            .find(|building| building.id == target.building_id)
            .map(|building| building.codes)
            .unwrap_or_default()
    };
    for (room, code) in [
        ("W8E-101", "W8E"),
        ("WL1-301(レクチャーシアター)", "WL1"),
        ("S3-206,S3-207", "S3"),
        ("M-374", "M"),
        ("Ｗ９－２０２（演習室）", "W9"),
        ("石川台1号館 204,253号室", "I1"),
        ("南4号館3階第2演習室", "S4"),
        ("西8号館(E) 計算機室", "W8E"),
    ] {
        let targets = map.locate(room, false);
        println!("{room}: {:?}", labels(&targets));
        assert!(!targets.is_empty(), "{room}");
        assert!(codes(&targets[0]).iter().any(|known| known == code), "{room}");
    }
}
