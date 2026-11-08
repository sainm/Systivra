use utoipa::OpenApi;

#[test]
fn committed_spec_matches_generated() {
    let generated = api::openapi::ApiDoc::openapi()
        .to_pretty_json()
        .expect("spec must serialize");
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contracts/api/openapi.json"
    );
    let committed = std::fs::read_to_string(path).expect("contracts/api/openapi.json is missing");
    assert_eq!(
        committed.trim(),
        generated.trim(),
        "contracts/api/openapi.json が古い。cargo run -p api --bin openapi > contracts/api/openapi.json で再生成する"
    );
}
