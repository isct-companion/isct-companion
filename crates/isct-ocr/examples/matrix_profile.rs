//! Local replay through the production scheduler and real model. No images or
//! recognized letters are written to the report. Pass a public/synthetic image,
//! expected 70-letter text file, and optional maximum number of frames.
use isct_core::matrix_scan::{MatrixPoint, MatrixReading, MatrixScanTiming, MatrixScanner};
use isct_ocr::MatrixOcrService;
use std::{
    error::Error,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = std::path::Path::new(
        args.get(1)
            .ok_or("Pass an image path or replay directory")?,
    );
    let mut paths = if input.is_dir() {
        std::fs::read_dir(input)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jpg" || e == "png"))
            .collect::<Vec<_>>()
    } else {
        vec![input.to_path_buf()]
    };
    paths.sort();
    let images = paths
        .iter()
        .map(std::fs::read)
        .collect::<Result<Vec<_>, _>>()?;
    if images.is_empty() {
        return Err("No replay frames".into());
    }
    let expected = std::fs::read_to_string(args.get(2).ok_or("Pass an expected-text path")?)?;
    let expected = expected.trim();
    if expected.len() != 70 {
        return Err("Expected text must have 70 letters".into());
    }
    let max: usize = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(100);
    let service = MatrixOcrService::default();
    let mut scanner = MatrixScanner::default();
    let id = scanner.begin();
    let start = Instant::now();
    let mut updates = vec![];
    let mut initial_ms = 0.;
    let mut complete = false;
    let mut wrong = 70;
    for index in 0..max {
        let frame_start = Instant::now();
        let plan = scanner.plan(&id)?;
        let frame = service.scan_live_frame(
            &images[index % images.len()],
            &plan.cells,
            |_| true,
            |_| true,
        )?;
        let timing = MatrixScanTiming {
            decode_ms: frame.timing.decode_ms,
            geometry_ms: frame.timing.geometry_ms,
            preprocess_ms: frame.timing.preprocess_ms,
            inference_ms: frame.timing.inference_ms,
            total_ms: frame.timing.total_ms,
        };
        let snapshot = scanner.observe(
            &id,
            &plan,
            Some((
                frame
                    .quads
                    .into_iter()
                    .map(|q| q.map(|[x, y]| MatrixPoint { x, y }))
                    .collect(),
                frame
                    .cells
                    .into_iter()
                    .map(|r| MatrixReading {
                        text: r.text,
                        confidence: r.confidence,
                    })
                    .collect(),
            )),
            timing,
        )?;
        let frame_ms = frame_start.elapsed().as_secs_f64() * 1000.;
        if index == 0 {
            initial_ms = frame_ms;
        } else if plan.phase == isct_core::matrix_scan::MatrixScanPhase::Refine {
            updates.push(frame_ms);
        }
        wrong = snapshot
            .cells
            .iter()
            .zip(expected.bytes())
            .filter(|(r, c)| r.text.as_bytes() != [*c])
            .count();
        eprintln!(
            "frame={} phase={:?} cells={} ms={:.1} stable={}/70 wrong={}",
            index + 1,
            plan.phase,
            plan.cells.len(),
            frame_ms,
            snapshot.stable_cells,
            wrong
        );
        if snapshot.complete {
            complete = true;
            break;
        }
        // Replay reproduces the desktop's start-to-start budget, while native
        // measurements additionally include real capture and IPC overhead.
        if frame_start.elapsed() < Duration::from_millis(200) {
            std::thread::sleep(Duration::from_millis(200) - frame_start.elapsed());
        }
    }
    let diagnostics = scanner.diagnostics(&id)?;
    let mean = if updates.is_empty() {
        0.
    } else {
        updates.iter().sum::<f64>() / updates.len() as f64
    };
    let mut sorted = updates.clone();
    sorted.sort_by(f64::total_cmp);
    let p95 = sorted
        .get(sorted.len().saturating_mul(95) / 100)
        .copied()
        .unwrap_or(0.);
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"kind":"synthetic-or-file-replay","complete":complete,"wrongCells":wrong,
        "sourceFrames":images.len(),"elapsedMs":start.elapsed().as_secs_f64()*1000.,"initialMs":initial_ms,
        "observedUpdateFps":diagnostics.frames as f64/start.elapsed().as_secs_f64(),
        "fullCardEquivalentFps":diagnostics.analyzed_cells as f64/70./start.elapsed().as_secs_f64(),"refineMeanMs":mean,"refineP95Ms":p95,
        "unthrottledUpdateFps":if mean>0.{1000./mean}else{0.},"diagnostics":diagnostics})
        )?
    );
    if !complete || wrong != 0 {
        std::process::exit(2);
    }
    Ok(())
}
