//! Production geometry export and deterministic event replay with measured stage latency.
#[cfg(matrix_ocr)]
mod replay {
    use isct_core::matrix_scan::{
        MatrixCameraBatch, MatrixCameraPhase, MatrixCameraScanner, MatrixCaptureFrame,
        MatrixCaptureGlyph, MatrixPoint, MatrixReading,
    };
    use serde::Deserialize;
    use serde_json::json;
    use std::{
        error::Error,
        io::{Read, Write},
        path::Path,
        time::{Duration, Instant},
    };
    #[derive(Deserialize)]
    struct Frame {
        t: f64,
        ms: f64,
        offset: Option<usize>,
        quality: Vec<[f64; 5]>,
    }
    #[derive(Deserialize)]
    struct Manifest {
        duration: f64,
        expected: String,
        frames: Vec<Frame>,
    }
    fn export() -> Result<(), Box<dyn Error>> {
        let mut input = std::io::stdin().lock();
        let mut output = std::io::BufWriter::new(std::io::stdout().lock());
        loop {
            let mut length = [0u8; 4];
            match input.read_exact(&mut length) {
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                result => result?,
            }
            let size = u32::from_le_bytes(length) as usize;
            if size > isct_ocr::MAX_IMAGE_BYTES {
                return Err("oversized frame".into());
            }
            let mut bytes = vec![0; size];
            input.read_exact(&mut bytes)?;
            let start = Instant::now();
            let captured = isct_ocr::capture_matrix_frame(&bytes);
            let ms = start.elapsed().as_secs_f64() * 1000.;
            match captured {
                Ok(frame) => {
                    let quality: Vec<_> = frame
                        .glyphs
                        .iter()
                        .map(|g| [g.contrast, g.sharpness, g.source_size, g.centering, g.glare])
                        .collect();
                    writeln!(output, "{}", json!({"ms":ms,"quality":quality}))?;
                    for g in frame.glyphs {
                        output.write_all(&g.gray)?;
                    }
                }
                Err(isct_ocr::MatrixOcrError::GridNotFound) => {
                    writeln!(output, "{}", json!({"ms":ms,"quality":[]}))?
                }
                Err(e) => return Err(e.into()),
            }
            output.flush()?;
        }
        Ok(())
    }
    fn captured(frame: &Frame, pixels: &[u8]) -> Option<MatrixCaptureFrame> {
        let offset = frame.offset?;
        let quads = (0..70)
            .map(|i| {
                let (x, y) = ((i % 10) as f64 / 10., (i / 10) as f64 / 7.);
                [
                    MatrixPoint { x, y },
                    MatrixPoint { x: x + 0.1, y },
                    MatrixPoint {
                        x: x + 0.1,
                        y: y + 1. / 7.,
                    },
                    MatrixPoint { x, y: y + 1. / 7. },
                ]
            })
            .collect();
        let glyphs = frame
            .quality
            .iter()
            .enumerate()
            .map(|(i, q)| MatrixCaptureGlyph {
                gray: pixels[offset + i * 9216..offset + (i + 1) * 9216].to_vec(),
                contrast: q[0],
                sharpness: q[1],
                source_size: q[2],
                centering: q[3],
                glare: q[4],
            })
            .collect();
        Some(MatrixCaptureFrame {
            frame_id: offset as u32,
            quads,
            glyphs,
        })
    }
    fn run(directory: &Path) -> Result<(), Box<dyn Error>> {
        let manifest: Manifest =
            serde_json::from_slice(&std::fs::read(directory.join("manifest.json"))?)?;
        let pixels = std::fs::read(directory.join("patches.bin"))?;
        if manifest.expected.len() != 70
            || !manifest.expected.bytes().all(|c| c.is_ascii_uppercase())
            || manifest.duration < 6.
            || manifest.frames.is_empty()
            || manifest.frames.windows(2).any(|f| f[0].t >= f[1].t)
            || manifest.frames.iter().any(|f| {
                !f.t.is_finite()
                    || !f.ms.is_finite()
                    || f.ms <= 0.
                    || f.offset.is_some_and(|o| {
                        o.checked_add(70 * 9216)
                            .is_none_or(|end| end > pixels.len())
                    })
                    || (f.offset.is_some() && f.quality.len() != 70)
            })
        {
            return Err("invalid replay bundle".into());
        }
        let model = isct_ocr::MatrixAlphabetService::default();
        // The app keeps this model service for its lifetime. Warm-up is reported separately.
        let warm = Instant::now();
        let _ = model.recognize(&[vec![255; 9216]])?;
        let warm_ms = warm.elapsed().as_secs_f64() * 1000.;
        let mut results = Vec::new();
        let mut latencies = Vec::new();
        for start in 0..=(manifest.duration - 6.).floor() as usize {
            let base = Instant::now();
            let mut scanner = MatrixCameraScanner::default();
            let id = scanner.begin_at(base, Duration::from_secs(6));
            let first = manifest.frames.partition_point(|f| f.t < start as f64);
            let mut producer = if first < manifest.frames.len() {
                Some((
                    first,
                    manifest.frames[first].t - start as f64 + manifest.frames[first].ms / 1000.,
                ))
            } else {
                None
            };
            let mut consumer: Option<(f64, MatrixCameraBatch, Vec<MatrixReading>)> = None;
            let mut now = 0.;
            let mut processed = 0;
            let mut located = 0;
            loop {
                let at = base + Duration::from_secs_f64(now);
                let snapshot = scanner.observe(&id, None, at)?;
                if snapshot.phase == MatrixCameraPhase::Complete {
                    break;
                }
                if consumer.is_none() {
                    if let Some(batch) = scanner.batch(&id, at)? {
                        let stage = Instant::now();
                        let readings = model
                            .recognize(&batch.images)?
                            .into_iter()
                            .map(|r| MatrixReading {
                                text: r.text,
                                confidence: r.confidence,
                            })
                            .collect();
                        let latency = stage.elapsed().as_secs_f64();
                        latencies.push(latency * 1000.);
                        consumer = Some((now + latency, batch, readings));
                    }
                }
                let next = producer
                    .as_ref()
                    .map_or(6., |(_, t)| *t)
                    .min(consumer.as_ref().map_or(6., |(t, _, _)| *t))
                    .min(6.);
                now = next;
                if now >= 6. {
                    scanner.observe(&id, None, base + Duration::from_secs(6))?;
                    break;
                }
                if producer.as_ref().is_some_and(|(_, t)| *t <= now) {
                    let (i, _) = producer.take().ok_or("producer lost")?;
                    let frame = captured(&manifest.frames[i], &pixels);
                    processed += 1;
                    located += usize::from(frame.is_some());
                    scanner.observe(&id, frame, base + Duration::from_secs_f64(now))?;
                    // Latest-frame slot: busy geometry drops intermediate video frames.
                    let latest = manifest
                        .frames
                        .partition_point(|f| f.t <= start as f64 + now)
                        .saturating_sub(1);
                    let index = (i + 1).max(latest);
                    if index < manifest.frames.len() {
                        let f = &manifest.frames[index];
                        producer = Some((index, now.max(f.t - start as f64) + f.ms / 1000.));
                    }
                }
                if consumer.as_ref().is_some_and(|(t, _, _)| *t <= now) {
                    let (_, batch, readings) = consumer.take().ok_or("consumer lost")?;
                    scanner.apply(&id, &batch, readings, base + Duration::from_secs_f64(now))?;
                }
            }
            let final_state = scanner.observe(&id, None, base + Duration::from_secs_f64(now))?;
            let mut correct = 0;
            let mut stable_correct = 0;
            let mut wrong_stable = 0;
            for (v, wanted) in final_state.cells.iter().zip(manifest.expected.bytes()) {
                let matches = v.text.as_bytes() == [wanted];
                correct += usize::from(matches);
                stable_correct += usize::from(matches && v.stable);
                wrong_stable += usize::from(!matches && v.stable);
            }
            let row = json!({"start":start,"end":start+6,"correct":correct,"confirmedCorrect":stable_correct,"wrongConfirmed":wrong_stable,
            "success":stable_correct==70 && !final_state.timed_out,"timedOut":final_state.timed_out,"elapsedMs":final_state.capture_elapsed_ms,
            "frames":processed,"locatedFrames":located,"diagnostics":scanner.diagnostics(&id,base+Duration::from_secs_f64(now))?});
            eprintln!("window {start:02}: {stable_correct}/70 confirmed, {correct}/70 provisional, {wrong_stable} wrong confirmed");
            results.push(row);
        }
        latencies.sort_by(f64::total_cmp);
        let report = json!({"windows":results,"windowSeconds":6,"strideSeconds":1,"warmupMs":warm_ms,"batchCount":latencies.len(),
        "batchMedianMs":latencies.get(latencies.len()/2),"batchP95Ms":latencies.get(latencies.len()*95/100),
        "clock":"event replay: local production geometry latency + measured TSUBAME production OCR latency; latest-frame slot; fresh Core session per window",
        "hostname":std::env::var("HOSTNAME").ok(),"jobId":std::env::var("JOB_ID").ok()});
        std::fs::write(
            directory.join("results.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        Ok(())
    }
    pub fn main() -> Result<(), Box<dyn Error>> {
        let args: Vec<_> = std::env::args().collect();
        match args.get(1).map(String::as_str) {
            Some("export") => export(),
            Some("run") => run(Path::new(args.get(2).ok_or("bundle directory required")?)),
            _ => Err("use export or run DIRECTORY".into()),
        }
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
