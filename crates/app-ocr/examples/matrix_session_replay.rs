//! Replays one recorded video as independent camera sessions through Core's
//! `MatrixCameraDriver`, the same driver, tracker, keyframe cut, dispatcher and
//! model the phones use (`submit` with a 4:2:0 YUV frame).
//!
//! `matrix_session_replay VIDEO EXPECTED OUT [--window S] [--stride S]
//!  [--starts A,B,..] [--geometry-scale K] [--ocr-ms MS] [--cache FRAMES_YUV]`
//!
//! Each window starts a fresh session at second `start` of the video and runs
//! it on the recording's timeline until it completes or Core's capture limit
//! passes. Frames reach the driver through a latest-frame slot: while one is
//! being processed newer ones replace the waiting frame. One OCR consumer runs
//! beside it. Stage costs are measured on this host (`--geometry-scale`
//! multiplies frame cost, `--ocr-ms` fixes the model latency for repeatable
//! comparisons); Core reads a clock that follows the recording. `OUT` receives
//! `frames.yuv` (decoded once, reused) and `windows.json`. Frames, truth and
//! outputs are card images or derived from them: keep them local.
#[cfg(matrix_ocr)]
#[path = "common/video.rs"]
mod video;
#[cfg(matrix_ocr)]
mod replay {
    use super::video::Video;
    use app_core::matrix_scan::{
        MatrixCameraBatch, MatrixCameraDriver, MatrixCameraPhase, MatrixCameraSnapshot,
        MatrixReading, MatrixVision,
    };
    use serde_json::{json, Value};
    use std::{
        error::Error,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    struct Options {
        window: f64,
        stride: f64,
        starts: Option<Vec<f64>>,
        geometry_scale: f64,
        ocr_ms: Option<f64>,
    }

    /// The recording's clock as Core sees it: set by the event loop.
    #[derive(Clone)]
    struct Timeline {
        base: Instant,
        now: Arc<Mutex<f64>>,
    }
    impl Timeline {
        fn set(&self, seconds: f64) {
            if let Ok(mut now) = self.now.lock() {
                *now = seconds;
            }
        }
        fn instant(&self) -> Instant {
            self.base + Duration::from_secs_f64(self.now.lock().map_or(0., |n| *n))
        }
    }

    fn session(
        video: &mut Video,
        vision: &Arc<dyn MatrixVision>,
        expected: &[u8],
        start: f64,
        options: &Options,
        latencies: &mut Vec<f64>,
        frame_costs: &mut Vec<f64>,
    ) -> Result<Value, Box<dyn Error>> {
        let timeline = Timeline {
            base: Instant::now(),
            now: Arc::new(Mutex::new(0.)),
        };
        let clock = timeline.clone();
        let driver = MatrixCameraDriver::new(vision.clone()).with_clock(move || clock.instant());
        let id = driver.begin()?;
        // Core's own limit ends the session; this bound only stops the loop.
        let horizon = 11.;
        let mut buffer = Vec::new();
        let first = video.times.partition_point(|&t| t < start);
        // (frame index, time it finishes processing)
        let mut producer: Option<usize> = (first < video.times.len()).then_some(first);
        let mut producer_free = 0.;
        let mut consumer: Option<(f64, MatrixCameraBatch, Vec<MatrixReading>)> = None;
        let (mut submitted, mut located, mut first_located) = (0, 0, None::<f64>);
        let mut now = 0.;
        let mut last: MatrixCameraSnapshot;
        loop {
            timeline.set(now);
            last = driver.poll(&id)?;
            if last.phase == MatrixCameraPhase::Complete {
                break;
            }
            if consumer.is_none() {
                if let Some(batch) = driver.reserve(&id)? {
                    let stage = Instant::now();
                    let readings = driver.recognize(&batch)?;
                    let measured = stage.elapsed().as_secs_f64() * 1000.;
                    latencies.push(measured);
                    let ms = options.ocr_ms.unwrap_or(measured);
                    consumer = Some((now + ms / 1000., batch, readings));
                }
            }
            let frame_at = producer.map(|i| (video.times[i] - start).max(producer_free));
            let next = [
                frame_at,
                consumer.as_ref().map(|(t, _, _)| *t),
                Some(horizon),
            ]
            .into_iter()
            .flatten()
            .fold(f64::INFINITY, f64::min)
            // Without new work the next event is a 10 ms consumer wakeup.
            .max(now);
            let idle = frame_at.is_none() && consumer.is_none();
            now = if idle {
                (now + 0.01).min(horizon)
            } else {
                next
            };
            if now >= horizon {
                timeline.set(horizon);
                last = driver.poll(&id)?;
                break;
            }
            timeline.set(now);
            if let (Some(index), Some(at)) = (producer, frame_at) {
                if at <= now {
                    video.read(index, &mut buffer)?;
                    let stage = Instant::now();
                    let snapshot = driver.submit(
                        &id,
                        index as u32,
                        &video.yuv(&buffer),
                        (video.times[index] - start) * 1000.,
                    )?;
                    let cost = stage.elapsed().as_secs_f64() * options.geometry_scale;
                    frame_costs.push(cost * 1000.);
                    submitted += 1;
                    if snapshot.bounds_frame_id == Some(index as u32) {
                        located += 1;
                        first_located.get_or_insert(now);
                    }
                    producer_free = now + cost;
                    // Latest-frame slot: the newest frame captured by then.
                    let newest = video
                        .times
                        .partition_point(|&t| t - start <= producer_free)
                        .saturating_sub(1);
                    let following = (index + 1).max(newest);
                    producer = (following < video.times.len()).then_some(following);
                }
            }
            if consumer.as_ref().is_some_and(|(t, _, _)| *t <= now) {
                if let Some((_, batch, readings)) = consumer.take() {
                    driver.apply(&id, &batch, readings)?;
                }
            }
        }
        let diagnostics = driver.stop(&id).ok_or("session vanished")?;
        let completed = last.phase == MatrixCameraPhase::Complete && !last.timed_out;
        let mut cells = Vec::new();
        let (mut correct, mut stable_correct, mut wrong_stable) = (0, 0, 0);
        for (i, (vote, &wanted)) in last.cells.iter().zip(expected).enumerate() {
            let matches = vote.text.as_bytes() == [wanted];
            correct += usize::from(matches);
            stable_correct += usize::from(matches && vote.stable);
            wrong_stable += usize::from(!matches && vote.stable);
            if !(matches && vote.stable) {
                cells.push(json!({"cell":i,"text":vote.text,"stable":vote.stable,"confidence":vote.confidence,
                    "qualified":diagnostics.qualified_samples[i],"glare":diagnostics.glare_rejections[i]}));
            }
        }
        let all_correct = correct == 70;
        Ok(json!({
            "start": start,
            "completed": completed,
            "elapsedMs": last.capture_elapsed_ms,
            "timedOut": last.timed_out,
            "allCorrect": all_correct,
            "success": completed && all_correct,
            // A completed session with any wrong cell: the user is shown a
            // confirmed card that is wrong.
            "wrongPass": completed && !all_correct,
            "correct": correct,
            "confirmedCorrect": stable_correct,
            "wrongConfirmed": wrong_stable,
            "frames": submitted,
            "locatedFrames": located,
            "firstLocatedMs": first_located.map(|t| t * 1000.),
            "keyframes": diagnostics.keyframes,
            "admissionRejections": diagnostics.admission_rejections,
            "completedSamples": diagnostics.completed_samples,
            "unsettled": cells,
        }))
    }

    fn percentile(values: &mut [f64], p: f64) -> Option<f64> {
        values.sort_by(f64::total_cmp);
        (!values.is_empty()).then(|| values[((values.len() - 1) as f64 * p).round() as usize])
    }

    pub fn main() -> Result<(), Box<dyn Error>> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let [video_path, expected_path, out] = [0, 1, 2].map(|i| args.get(i).cloned());
        let (Some(video_path), Some(expected_path), Some(out)) = (video_path, expected_path, out)
        else {
            return Err("usage: matrix_session_replay VIDEO EXPECTED OUT [options]".into());
        };
        let flag = |name: &str| -> Option<&String> {
            args.iter()
                .position(|a| a == name)
                .and_then(|i| args.get(i + 1))
        };
        let options = Options {
            window: flag("--window").map_or(Ok(10.), |v| v.parse())?,
            stride: flag("--stride").map_or(Ok(1.), |v| v.parse())?,
            starts: flag("--starts")
                .map(|v| {
                    v.split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()
                })
                .transpose()?,
            geometry_scale: flag("--geometry-scale").map_or(Ok(1.), |v| v.parse())?,
            ocr_ms: flag("--ocr-ms").map(|v| v.parse()).transpose()?,
        };
        let expected: Vec<u8> = std::fs::read_to_string(&expected_path)?
            .split_whitespace()
            .collect::<String>()
            .into_bytes();
        if expected.len() != 70 || !expected.iter().all(u8::is_ascii_uppercase) {
            return Err("EXPECTED must hold exactly 70 letters A-Z".into());
        }
        let out = PathBuf::from(out);
        std::fs::create_dir_all(&out)?;
        let cache = flag("--cache").map_or_else(|| out.join("frames.yuv"), PathBuf::from);
        let mut video = Video::open(Path::new(&video_path), &cache)?;
        let vision = app_ocr::matrix_vision();
        let warm = Instant::now();
        vision.warm_up()?;
        let warm_ms = warm.elapsed().as_secs_f64() * 1000.;
        let starts = options.starts.clone().unwrap_or_else(|| {
            let last = (video.duration - options.window).floor().max(0.);
            (0..)
                .map(|i| i as f64 * options.stride)
                .take_while(|&s| s <= last)
                .collect()
        });
        let (mut latencies, mut frame_costs, mut windows) = (Vec::new(), Vec::new(), Vec::new());
        for &start in &starts {
            let row = session(
                &mut video,
                &vision,
                &expected,
                start,
                &options,
                &mut latencies,
                &mut frame_costs,
            )?;
            eprintln!(
                "window {start:5.1}: {} {:>5.0} ms, {}/70 correct, {} confirmed, {} wrong confirmed, {} located/{} frames",
                if row["success"] == true { "OK  " } else if row["wrongPass"] == true { "WRONG" } else { "fail" },
                row["elapsedMs"].as_f64().unwrap_or(0.),
                row["correct"], row["confirmedCorrect"], row["wrongConfirmed"],
                row["locatedFrames"], row["frames"]
            );
            windows.push(row);
        }
        let report = json!({
            "video": {"width": video.width, "height": video.height, "frames": video.times.len(),
                "duration": video.duration},
            "windowSeconds": options.window,
            "geometryScale": options.geometry_scale,
            "fixedOcrMs": options.ocr_ms,
            "warmupMs": warm_ms,
            "batchCount": latencies.len(),
            "batchMs": {"p50": percentile(&mut latencies, 0.5), "p95": percentile(&mut latencies, 0.95)},
            "frameMs": {"p50": percentile(&mut frame_costs, 0.5), "p95": percentile(&mut frame_costs, 0.95)},
            "clock": "event replay on the recording's timeline: Core MatrixCameraDriver (submit/reserve/apply) with measured host stage costs; latest-frame slot; fresh session per window",
            "windows": windows,
        });
        std::fs::write(
            out.join("windows.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(())
    }
}
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    replay::main()
}
#[cfg(not(matrix_ocr))]
fn main() {
    eprintln!("Matrix OCR unavailable for this target");
    std::process::exit(1);
}
