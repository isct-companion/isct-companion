use super::*;
use lopdf::{dictionary, Object};

/// A PDF of `count` empty A4 pages, page `n` with `/Rotate` set to
/// `90 * (n - 1) % 360`, so a kept page can be told apart from the others.
fn pdf(count: u32) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let kids: Vec<Object> = (1..=count)
        .map(|n| {
            doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Rotate" => i64::from(90 * (n - 1) % 360),
            })
            .into()
        })
        .collect();
    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => kids,
        "Count" => i64::from(count),
        "MediaBox" => vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(595),
            Object::Integer(842),
        ],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

fn rotations(document: &[u8]) -> Vec<i64> {
    let doc = Document::load_mem(document).unwrap();
    doc.get_pages()
        .values()
        .map(|id| {
            doc.get_dictionary(*id)
                .unwrap()
                .get(b"Rotate")
                .and_then(Object::as_i64)
                .unwrap()
        })
        .collect()
}

#[test]
fn counts_the_pages() {
    assert_eq!(count(&pdf(5)), Ok(5));
}

// The kept pages stay in the file's order, whatever order they were chosen in.
#[test]
fn keeps_only_the_chosen_pages_in_file_order() {
    let kept = keep(&pdf(4), &[4, 2, 2]).unwrap();
    assert_eq!(rotations(&kept), vec![90, 270]);
}

#[test]
fn every_page_chosen_leaves_the_bytes_alone() {
    let original = pdf(3);
    assert_eq!(keep(&original, &[1, 2, 3]).unwrap(), original);
}

#[test]
fn refuses_pages_it_cannot_keep() {
    assert_eq!(keep(&pdf(3), &[]), Err(PagesError::OutOfRange));
    assert_eq!(keep(&pdf(3), &[0]), Err(PagesError::OutOfRange));
    assert_eq!(keep(&pdf(3), &[4]), Err(PagesError::OutOfRange));
    assert_eq!(keep(b"%!PS-Adobe-3.0\n", &[1]), Err(PagesError::NotPdf));
    // A PDF header with no PDF behind it: not readable, or no page 1 in it.
    assert!(keep(b"%PDF-1.7 not really", &[1]).is_err());
}
