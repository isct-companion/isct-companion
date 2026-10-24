//! A local recording decoded once to 4:2:0 YUV, read frame by frame as the
//! `MatrixYuvFrame` a phone camera hands to Core. Shared by the session
//! replay and the dataset export. Requires `ffmpeg` and `ffprobe` on PATH.
use isct_core::matrix_scan::{MatrixYuvFrame, MatrixYuvMatrix};
use serde_json::Value;
use std::{
    error::Error,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    process::Command,
};

pub struct Video {
    pub width: u32,
    pub height: u32,
    pub matrix: MatrixYuvMatrix,
    pub full_range: bool,
    /// Presentation time of each frame in seconds, from 0.
    pub times: Vec<f64>,
    pub duration: f64,
    file: File,
}

fn frame_bytes(width: u32, height: u32) -> usize {
    let (w, h) = (width as usize, height as usize);
    w * h + 2 * w.div_ceil(2) * h.div_ceil(2)
}

impl Video {
    /// Probes `source` and decodes it into `cache` unless a cache of the
    /// expected size is already there.
    pub fn open(source: &Path, cache: &Path) -> Result<Self, Box<dyn Error>> {
        let output = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
            .arg("stream=width,height,color_space,color_range:stream_side_data=rotation:frame=best_effort_timestamp_time:format=duration")
            .args(["-of", "json"])
            .arg(source)
            .output()?;
        if !output.status.success() {
            return Err(format!("ffprobe failed on {}", source.display()).into());
        }
        let info: Value = serde_json::from_slice(&output.stdout)?;
        let stream = &info["streams"][0];
        let (mut width, mut height) = (
            stream["width"].as_u64().ok_or("width")? as u32,
            stream["height"].as_u64().ok_or("height")? as u32,
        );
        // ffmpeg applies the display rotation while decoding.
        let rotation = stream["side_data_list"]
            .as_array()
            .and_then(|l| l.iter().find_map(|d| d["rotation"].as_i64()))
            .unwrap_or(0);
        if rotation.rem_euclid(180) == 90 {
            std::mem::swap(&mut width, &mut height);
        }
        let mut times: Vec<f64> = info["frames"]
            .as_array()
            .ok_or("frames")?
            .iter()
            .filter_map(|f| f["best_effort_timestamp_time"].as_str()?.parse().ok())
            .collect();
        let origin = *times.first().ok_or("no frames")?;
        times.iter_mut().for_each(|t| *t -= origin);
        if times.windows(2).any(|p| p[0] >= p[1]) {
            return Err("frame timestamps are not increasing".into());
        }
        let duration = info["format"]["duration"]
            .as_str()
            .and_then(|d| d.parse().ok())
            .unwrap_or(times[times.len() - 1]);
        let matrix = match stream["color_space"].as_str() {
            Some("smpte170m" | "bt470bg") => MatrixYuvMatrix::Bt601,
            Some("bt709") => MatrixYuvMatrix::Bt709,
            _ if height >= 720 => MatrixYuvMatrix::Bt709,
            _ => MatrixYuvMatrix::Bt601,
        };
        let full_range = stream["color_range"].as_str() == Some("pc");
        let expected = (times.len() * frame_bytes(width, height)) as u64;
        if std::fs::metadata(cache).map(|m| m.len()).ok() != Some(expected) {
            let status = Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-i"])
                .arg(source)
                .args([
                    "-map",
                    "0:v:0",
                    "-fps_mode",
                    "passthrough",
                    "-f",
                    "rawvideo",
                ])
                .args(["-pix_fmt", "yuv420p"])
                .arg(cache)
                .status()?;
            if !status.success() {
                return Err("ffmpeg failed".into());
            }
        }
        let file = File::open(cache)?;
        if file.metadata()?.len() != expected {
            return Err("decoded frame cache does not match the probed frames".into());
        }
        Ok(Self {
            width,
            height,
            matrix,
            full_range,
            times,
            duration,
            file,
        })
    }

    pub fn read(&mut self, index: usize, buffer: &mut Vec<u8>) -> Result<(), Box<dyn Error>> {
        let size = frame_bytes(self.width, self.height);
        buffer.resize(size, 0);
        self.file.seek(SeekFrom::Start((index * size) as u64))?;
        self.file.read_exact(buffer)?;
        Ok(())
    }

    pub fn yuv<'a>(&self, data: &'a [u8]) -> MatrixYuvFrame<'a> {
        let (w, h) = (self.width as usize, self.height as usize);
        let chroma = w.div_ceil(2) * h.div_ceil(2);
        MatrixYuvFrame {
            width: self.width,
            height: self.height,
            y: &data[..w * h],
            y_row_stride: self.width,
            u: &data[w * h..w * h + chroma],
            v: &data[w * h + chroma..],
            uv_row_stride: w.div_ceil(2) as u32,
            uv_pixel_stride: 1,
            matrix: self.matrix,
            full_range: self.full_range,
        }
    }
}
