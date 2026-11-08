use std::path::Path;

#[test]
fn extracts_japanese_text_from_100_page_fixture() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/walking_skeleton_100p.pdf");
    let pages = worker::pdf::extract_pages(&path).expect("fixture must extract");
    assert_eq!(pages.len(), 100);
    assert!(pages[36].contains("住所チェックの桁数と文字種"));
    assert!(pages[36].contains("検証キー-37-顧客管理"));
    assert!(pages[0].contains("第1章"));
}
