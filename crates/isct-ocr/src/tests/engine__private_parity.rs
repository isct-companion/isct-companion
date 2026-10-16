//! Explicit local diagnostic; never logs characters, cell images, or hashes.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(path: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("private input unavailable"))
        .expect("invalid private test input")
}
fn decode_prepared(value: &str) -> RgbImage {
    decode_image(&STANDARD.decode(value).expect("invalid prepared encoding"))
        .expect("invalid prepared image")
}
fn points(v: &Value) -> Vec<[geometry::Point; 4]> {
    v.as_array()
        .expect("invalid private grid")
        .iter()
        .map(|cell| {
            std::array::from_fn(|i| geometry::Point {
                x: cell[i]["x"].as_f64().unwrap(),
                y: cell[i]["y"].as_f64().unwrap(),
            })
        })
        .collect()
}
#[test]
#[ignore = "explicit private TS/native comparison; aggregate-only output"]
fn private_ts_native_parity() {
    let root = std::path::PathBuf::from(
        std::env::var("MATRIX_PRIVATE_DIR").expect("set MATRIX_PRIVATE_DIR"),
    );
    let input = read(&root.join("prepared.json"));
    let source = decode_prepared(input["data"].as_str().unwrap());
    let small = decode_prepared(input["preview"].as_str().unwrap());
    let cells = geometry::locate(&small);
    if std::env::var("MATRIX_PRIVATE_STAGE").as_deref() == Ok("prepare") {
        let grid = cells.as_ref().map(|cs| {
            cs.iter()
                .map(|ps| ps.map(|p| json!({"x":p.x,"y":p.y})))
                .collect::<Vec<_>>()
        });
        std::fs::write(
            root.join("native-grid.json"),
            serde_json::to_vec(&grid).unwrap(),
        )
        .expect("private grid write failed");
        eprintln!("Native geometry found: {}", cells.is_some());
        return;
    }
    let reference = read(&root.join("reference.json"));
    let grid = points(&reference["grid"]);
    let references = reference["cells"]
        .as_array()
        .expect("missing private cells");
    let mut paddle = Paddle::new().expect("native model initialization failed");
    let (
        mut crop_diff,
        mut binary_diff,
        mut clean_diff,
        mut tensor_diff,
        mut raw_diff,
        mut final_diff,
        mut confidence_delta,
    ) = (0, 0, 0, 0, 0, 0, 0f64);
    let (mut high, mut medium, mut low, mut blank) = (0, 0, 0, 0);
    for (i, points) in grid.iter().enumerate() {
        let points = points.map(|p| geometry::Point {
            x: p.x * source.width() as f64 / small.width() as f64,
            y: p.y * source.height() as f64 / small.height() as f64,
        });
        let crop = geometry::crop(&source, &points);
        let binary = preprocess::binarize(&crop);
        let prepared = preprocess::prepare(&binary.cleaned);
        let r = &references[i];
        crop_diff += usize::from(hash(crop.as_raw()) != r["crop_hash"].as_str().unwrap());
        binary_diff +=
            usize::from(hash(binary.original.as_raw()) != r["binary_hash"].as_str().unwrap());
        clean_diff +=
            usize::from(hash(binary.cleaned.as_raw()) != r["clean_hash"].as_str().unwrap());
        let bytes = prepared
            .tensor
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        tensor_diff += usize::from(hash(&bytes) != r["tensor_hash"].as_str().unwrap());
        let mut accepted = Recognition::default();
        if binary.ink >= 12 && binary.ink as f64 <= 64. * 64. * 0.55 {
            let raw = paddle
                .recognize(prepared.tensor)
                .expect("native inference failed");
            raw_diff += usize::from(raw.text != r["raw"]["text"].as_str().unwrap());
            confidence_delta = confidence_delta
                .max((raw.confidence - r["raw"]["confidence"].as_f64().unwrap()).abs());
            accepted = recognition::accept(&raw.text, raw.confidence);
            if !accepted.text.is_empty() && binary.original != binary.cleaned {
                let original = paddle
                    .recognize(preprocess::prepare(&binary.original).tensor)
                    .expect("native reference inference failed");
                accepted = recognition::review(accepted, &original.text);
            }
        }
        final_diff += usize::from(accepted.text != r["accepted"]["text"].as_str().unwrap());
        if accepted.text.is_empty() {
            blank += 1;
            low += 1;
        } else if accepted.confidence < 0.75 {
            low += 1;
        } else if accepted.confidence < 0.9 {
            medium += 1;
        } else {
            high += 1;
        }
    }
    let geometry_delta = cells.as_ref().map(|cs| {
        cs.iter()
            .zip(&grid)
            .flat_map(|(a, b)| {
                a.iter()
                    .zip(b)
                    .map(|(a, b)| (a.x - b.x).abs().max((a.y - b.y).abs()))
            })
            .fold(0., f64::max)
    });
    let (mut actual_diff, mut actual_crop_diff, mut actual_blank, mut actual_red) = (0, 0, 0, 0);
    if let Some(actual_grid) = cells.as_ref() {
        for (i, points) in actual_grid.iter().enumerate() {
            let points = points.map(|p| geometry::Point {
                x: p.x * source.width() as f64 / small.width() as f64,
                y: p.y * source.height() as f64 / small.height() as f64,
            });
            let crop = geometry::crop(&source, &points);
            actual_crop_diff +=
                usize::from(hash(crop.as_raw()) != references[i]["crop_hash"].as_str().unwrap());
            let binary = preprocess::binarize(&crop);
            let mut accepted = Recognition::default();
            if binary.ink >= 12 && binary.ink as f64 <= 4096. * 0.55 {
                let raw = paddle
                    .recognize(preprocess::prepare(&binary.cleaned).tensor)
                    .expect("native actual inference failed");
                accepted = recognition::accept(&raw.text, raw.confidence);
                if !accepted.text.is_empty() && binary.original != binary.cleaned {
                    let raw = paddle
                        .recognize(preprocess::prepare(&binary.original).tensor)
                        .expect("native actual reference failed");
                    accepted = recognition::review(accepted, &raw.text);
                }
            }
            actual_diff +=
                usize::from(accepted.text != references[i]["accepted"]["text"].as_str().unwrap());
            actual_blank += usize::from(accepted.text.is_empty());
            actual_red += usize::from(recognition::is_red(&accepted));
        }
    }
    let summary = json!({"legacy_geometry_found":reference["legacy_found"],"native_geometry_found":cells.is_some(),"geometry_max_delta":geometry_delta,"cells":grid.len(),"crop_mismatches":crop_diff,"binary_mismatches":binary_diff,"denoise_mismatches":clean_diff,"tensor_mismatches":tensor_diff,"raw_ocr_mismatches":raw_diff,"final_ocr_mismatches":final_diff,"max_confidence_delta":confidence_delta,"shared_grid_bands":{"green":high,"yellow":medium,"red":low,"blank":blank},"actual_native_crop_mismatches":actual_crop_diff,"actual_native_ocr_mismatches":actual_diff,"actual_native_blank":actual_blank,"actual_native_red":actual_red});
    // Only numeric/boolean aggregates leave this test. Never assert_eq! on strings.
    eprintln!("{summary}");
    std::fs::write(
        root.join("summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .expect("summary write failed");
    assert!(
        crop_diff == 0
            && binary_diff == 0
            && clean_diff == 0
            && tensor_diff == 0
            && raw_diff == 0
            && final_diff == 0
            && actual_diff == 0,
        "pixel parity failed; see aggregate counts"
    );
}
