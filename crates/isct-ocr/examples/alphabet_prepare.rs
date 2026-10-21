//! Stream synthetic 96x96 u8 cells through the production model preprocessing.
//! Output record: one flag byte (0 rejected, 1 original, 2 also corrected), then
//! two 9216-byte views. Keeping records fixed-size preserves source identities.
#[cfg(matrix_ocr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{BufReader, BufWriter, Read, Write};
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = BufWriter::new(std::io::stdout().lock());
    let mut pixels = vec![0u8; 96 * 96];
    let blank = vec![255u8; 96 * 96];
    let recognize = std::env::args().any(|argument| argument == "--recognize");
    let model = isct_ocr::MatrixAlphabetService::default();
    loop {
        if input.read(&mut pixels[..1])? == 0 {
            break;
        }
        input.read_exact(&mut pixels[1..])?;
        if recognize {
            // A numeric/label parity probe for public synthetic inputs only.
            serde_json::to_writer(&mut output, &model.recognize(&[pixels.clone()])?)?;
            output.write_all(b"\n")?;
            continue;
        }
        match isct_ocr::prepare_matrix_alphabet_input(&pixels)? {
            None => {
                output.write_all(&[0])?;
                output.write_all(&blank)?;
                output.write_all(&blank)?;
            }
            Some(glyph) => {
                output.write_all(&[if glyph.corrected.is_some() { 2 } else { 1 }])?;
                output.write_all(&glyph.original)?;
                output.write_all(glyph.corrected.as_ref().unwrap_or(&glyph.original))?;
            }
        }
    }
    output.flush()?;
    Ok(())
}

#[cfg(not(matrix_ocr))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("Matrix OCR backend unavailable".into())
}
