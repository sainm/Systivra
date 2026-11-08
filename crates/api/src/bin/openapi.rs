use utoipa::OpenApi;

fn main() {
    let generated = api::openapi::ApiDoc::openapi()
        .to_pretty_json()
        .expect("OpenAPI は常に JSON へ変換できる");
    if std::env::args().any(|arg| arg == "--check") {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/api/openapi.json"
        );
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        if committed.trim() != generated.trim() {
            eprintln!("contracts/api/openapi.json が生成結果と一致しません。再生成してください。");
            std::process::exit(1);
        }
        println!("openapi.json is up to date");
    } else {
        println!("{generated}");
    }
}
