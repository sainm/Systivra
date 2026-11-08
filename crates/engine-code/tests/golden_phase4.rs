use std::collections::BTreeSet;
use std::path::PathBuf;

use engine_code::{analyze_sources, AnalyzeOptions, FactResolution, SourceInput, SymbolKind};
use serde_json::Value;

fn sample() -> (Vec<SourceInput>, BTreeSet<String>, BTreeSet<String>) {
    let mut inputs = Vec::new();
    let mut expected_symbols = BTreeSet::new();
    let mut expected_calls = BTreeSet::new();
    for service in 0..20 {
        let mut source = format!("package sample; public class Service{service} {{\n");
        expected_symbols.insert(format!("sample.Service{service}"));
        for method in 0..10 {
            source.push_str(&format!("  public void work{method}() {{ }}\n"));
            expected_symbols.insert(format!("sample.Service{service}.work{method}()"));
        }
        source.push_str("}\n");
        inputs.push(SourceInput {
            path: format!("src/main/java/sample/Service{service}.java"),
            bytes: source.into_bytes(),
        });

        let mut action = format!("package sample; public class Action{service} {{\n");
        expected_symbols.insert(format!("sample.Action{service}"));
        for target in 0..10 {
            action.push_str(&format!(
                "  private Service{} service{};\n",
                (service + target) % 20,
                target
            ));
        }
        action.push_str("  public void execute() {\n");
        expected_symbols.insert(format!("sample.Action{service}.execute()"));
        for target in 0..10 {
            let service_name = format!("Service{}", (service + target) % 20);
            action.push_str(&format!("    service{target}.work{target}();\n"));
            expected_calls.insert(format!(
                "sample.Action{service}.execute()->sample.{service_name}.work{target}()"
            ));
        }
        action.push_str("  }\n}\n");
        inputs.push(SourceInput {
            path: format!("src/main/java/sample/Action{service}.java"),
            bytes: action.into_bytes(),
        });
    }

    let mut reflection = String::from("package sample; public class ReflectionSample {\n");
    expected_symbols.insert("sample.ReflectionSample".into());
    for index in 0..30 {
        if index < 15 {
            reflection.push_str(&format!(
                "  void dynamic{index}() throws Exception {{ Class.forName(\"sample.Service{}\"); }}\n",
                index % 20
            ));
        } else {
            reflection.push_str(&format!(
                "  void dynamic{index}() throws Exception {{ java.lang.reflect.Proxy.newProxyInstance(null, null, null); }}\n"
            ));
        }
        expected_symbols.insert(format!("sample.ReflectionSample.dynamic{index}()"));
    }
    reflection.push_str("}\n");
    inputs.push(SourceInput {
        path: "src/main/java/sample/ReflectionSample.java".into(),
        bytes: reflection.into_bytes(),
    });
    (inputs, expected_symbols, expected_calls)
}

#[test]
fn source_sample_meets_phase4_quality_gate() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/code/golden-phase4.json");
    let thresholds: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_path).unwrap()).unwrap();
    let (inputs, expected_symbols, expected_calls) = sample();
    let analysis = analyze_sources(&inputs, &AnalyzeOptions::default());

    let classes = analysis
        .report
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Class)
        .count();
    let methods = analysis
        .report
        .symbols
        .iter()
        .filter(|symbol| symbol.kind == SymbolKind::Method)
        .count();
    assert_eq!(classes, thresholds["classes"].as_u64().unwrap() as usize);
    assert_eq!(methods, thresholds["methods"].as_u64().unwrap() as usize);

    let actual_symbols = analysis
        .report
        .symbols
        .iter()
        .map(|symbol| symbol.fqn.clone())
        .collect::<BTreeSet<_>>();
    let symbol_recall = expected_symbols.intersection(&actual_symbols).count() as f64
        / expected_symbols.len() as f64;
    assert!(
        symbol_recall >= thresholds["min_symbol_recall"].as_f64().unwrap(),
        "symbol recall={symbol_recall}"
    );

    let actual_calls = analysis
        .report
        .facts
        .iter()
        .filter(|fact| fact.relation == "CALLS" && fact.resolution == FactResolution::Resolved)
        .map(|fact| format!("{}->{}", fact.subject.key, fact.object.key))
        .collect::<BTreeSet<_>>();
    let precise = actual_calls.intersection(&expected_calls).count() as f64;
    let precision = if actual_calls.is_empty() {
        0.0
    } else {
        precise / actual_calls.len() as f64
    };
    let recall = if expected_calls.is_empty() {
        0.0
    } else {
        precise / expected_calls.len() as f64
    };
    assert!(
        precision >= thresholds["min_call_precision"].as_f64().unwrap(),
        "call precision={precision}, expected={}, actual={}",
        expected_calls.len(),
        actual_calls.len()
    );
    assert!(recall >= 0.98, "call recall={recall}");

    let dynamic = analysis
        .report
        .facts
        .iter()
        .filter(|fact| {
            fact.relation == "CALLS"
                && fact
                    .subject
                    .key
                    .starts_with("sample.ReflectionSample.dynamic")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        dynamic.len(),
        thresholds["reflection_calls"].as_u64().unwrap() as usize
    );
    assert!(dynamic
        .iter()
        .all(|fact| fact.resolution != FactResolution::Resolved));
}

#[test]
fn selected_paths_keep_incremental_scan_below_ten_percent() {
    let (inputs, _, _) = sample();
    let selected = BTreeSet::from([inputs[1].path.clone()]);
    let analysis = analyze_sources(
        &inputs,
        &AnalyzeOptions {
            classpath_complete: false,
            selected_paths: Some(selected.clone()),
        },
    );
    assert_eq!(analysis.report.files_seen, 41);
    assert_eq!(analysis.report.files_analyzed, 1);
    assert_eq!(analysis.report.files_skipped, 40);
    assert!(analysis.report.files_analyzed as f64 / (analysis.report.files_seen as f64) < 0.1);
    assert_eq!(analysis.files[0].path, *selected.iter().next().unwrap());
}

#[test]
fn incremental_selection_keeps_unchanged_java_types_as_resolution_context() {
    let inputs = vec![
        SourceInput {
            path: "src/main/java/demo/AddressService.java".into(),
            bytes: b"package demo; public class AddressService { public void validate() {} }"
                .to_vec(),
        },
        SourceInput {
            path: "src/main/java/demo/UserService.java".into(),
            bytes: b"package demo; public class UserService { private AddressService addressService; void update() { addressService.validate(); } }".to_vec(),
        },
    ];
    let selected = BTreeSet::from(["src/main/java/demo/UserService.java".to_string()]);
    let analysis = analyze_sources(
        &inputs,
        &AnalyzeOptions {
            classpath_complete: false,
            selected_paths: Some(selected),
        },
    );
    assert_eq!(analysis.report.files_analyzed, 1);
    assert!(analysis.report.facts.iter().any(|fact| {
        fact.relation == "CALLS"
            && fact.object.key == "demo.AddressService.validate()"
            && fact.resolution == FactResolution::Resolved
    }));
}
