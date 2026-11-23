use super::*;
use crate::pdf::content::tokenize;

fn empty_file() -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n1 0 obj << /Type /Catalog >> endobj\n".to_vec();
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!(
            "xref\n0 2\n0000000000 65535 f \n0000000009 00000 n \ntrailer << /Size 2 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    pdf
}

#[test]
fn text_joins_along_the_line_and_to_the_next_line_but_not_across_a_gutter() {
    let em = 12.0;
    let word = [100.0, 700.0, 140.0, 712.0];
    // A word space, and a justified line's wide one: one block.
    assert!(near(&word, &[147.0, 700.0, 180.0, 712.0], em, true));
    assert!(near(&word, &[155.0, 700.0, 180.0, 712.0], em, true));
    // A column's gutter (two em): two blocks.
    assert!(!near(&word, &[165.0, 700.0, 200.0, 712.0], em, true));
    // The next line (a 1.2 em pitch): one block; a paragraph's space: two.
    assert!(near(&word, &[100.0, 685.6, 140.0, 697.6], em, true));
    assert!(!near(&word, &[100.0, 680.0, 140.0, 692.0], em, true));
}

#[test]
fn what_this_module_writes_reads_back_as_its_own_wrapper() {
    let bytes = empty_file();
    let file = File::new(&bytes).expect("file");
    let m = Matrix([0.5, 0.0, 0.0, 0.5, 10.0, 20.0]);
    let mut data = wrapper_open(4, &m).into_bytes();
    data.extend_from_slice(crop_ops([(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]).as_bytes());
    data.extend_from_slice(b"0 0 100 100 re f");
    data.extend_from_slice(WRAPPER_CLOSE.as_bytes());
    let content = Content {
        items: Vec::new(),
        ops: vec![tokenize(&data).expect("tokenizes")],
        data: vec![data],
        wrap_streams: vec![false],
    };
    let traced = trace(&file, &Dict::default(), &content).expect("traced");
    assert_eq!(traced.wrappers.len(), 1);
    let wrapper = &traced.wrappers[0];
    assert_eq!((wrapper.group, wrapper.matrix, wrapper.crop.is_some()), (4, m, true));
    // The path is cut to the crop: one point square at (10, 20) after the map.
    assert_eq!(traced.pieces[0].bounds, Some([10.0, 20.0, 10.5, 20.5]));
    assert_eq!(traced.pieces[0].wrapper, Some(0));
}

#[test]
fn numbers_are_written_plainly() {
    assert_eq!(fmt(0.1 + 0.2), "0.3");
    assert_eq!(fmt(-0.0000001), "0");
    assert_eq!(fmt(1e20), "100000000000000000000");
    assert_eq!(fmt(f64::NAN), "0");
}
