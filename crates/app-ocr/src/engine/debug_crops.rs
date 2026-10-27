use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
pub struct Crop {
    name: String,
    data: String,
}

impl Crop {
    pub(crate) fn from_png(name: String, bytes: Vec<u8>) -> Self {
        Self {
            name,
            data: STANDARD.encode(bytes),
        }
    }
}

fn decode(crop: Crop) -> Result<(String, Vec<u8>), String> {
    let name = crop.name.as_bytes();
    if name.len() != 6
        || !(b'A'..=b'J').contains(&name[0])
        || !(b'1'..=b'7').contains(&name[1])
        || &name[2..] != b".png"
        || crop.data.len() > 100_000
    {
        return Err("Invalid matrix crop".into());
    }
    let bytes = STANDARD
        .decode(crop.data)
        .map_err(|_| "Invalid crop encoding")?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("Expected PNG crop".into());
    }
    Ok((crop.name, bytes))
}

pub fn save(root: &Path, files: Vec<Crop>) -> Result<(), String> {
    if files.is_empty() || files.len() > 70 {
        return Err("Invalid crop count".into());
    }
    let decoded = files
        .into_iter()
        .map(decode)
        .collect::<Result<Vec<_>, _>>()?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let folder = root.join(format!("matrix-{stamp}"));
    fs::create_dir(&folder).map_err(|e| e.to_string())?;
    for (name, bytes) in decoded {
        fs::write(folder.join(name), bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/engine__debug_crops.rs"]
mod tests;
