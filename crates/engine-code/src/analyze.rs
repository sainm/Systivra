use std::collections::{BTreeMap, BTreeSet, HashMap};

use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser};
use unicode_normalization::UnicodeNormalization;

use crate::fact::{
    AnalysisError, AnalysisReport, BatchAnalysis, Evidence, Fact, FactObject, FactResolution,
    FactSubject, ParserStats, SourceAnalysis, SourceInput, Symbol, SymbolKind,
};
use crate::jsp;
use crate::language::Language;
use crate::text::{self, DecodedFile};

pub const ENGINE_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-tree-sitter");

#[derive(Debug, Clone, Default)]
pub struct AnalyzeOptions {
    /// True means a customer-provided classpath was used by a semantic worker.
    /// The in-process Tree-sitter adapter always reports false.
    pub classpath_complete: bool,
    /// Parse only these repository-relative paths when non-empty.
    pub selected_paths: Option<BTreeSet<String>>,
}

pub fn analyze_sources(inputs: &[SourceInput], options: &AnalyzeOptions) -> BatchAnalysis {
    let mut files = Vec::new();
    let mut prepared_java = Vec::new();
    let mut report = AnalysisReport {
        files_seen: inputs.len() as u64,
        classpath_complete: options.classpath_complete,
        ..AnalysisReport::default()
    };

    for input in inputs {
        let selected = !options
            .selected_paths
            .as_ref()
            .is_some_and(|selected| !selected.contains(&input.path));
        let Some(language) = Language::detect(std::path::Path::new(&input.path)) else {
            report.files_skipped += 1;
            continue;
        };
        if selected {
            report.files_analyzed += 1;
        } else {
            report.files_skipped += 1;
        }
        match text::decide(&input.bytes) {
            None => {
                if selected {
                    let error = AnalysisError {
                        path: input.path.clone(),
                        code: "ENCODING_UNDETECTED".into(),
                        message: "文字コードを確定できませんでした".into(),
                    };
                    report.errors.push(error.clone());
                    files.push(skipped_source(input, language, error));
                }
            }
            Some(decoded) => match language {
                Language::Java => {
                    prepared_java.push(prepare_java(input, language, decoded, selected))
                }
                Language::Jsp if selected => files.push(analyze_jsp(input, language, decoded)),
                Language::Xml if selected => files.push(analyze_xml(input, language, decoded)),
                Language::Sql if selected => files.push(analyze_sql_file(input, language, decoded)),
                _ => {}
            },
        }
    }

    finalize_java(prepared_java, &mut files, &mut report);
    for file in &files {
        report.symbols.extend(file.symbols.iter().cloned());
        report.facts.extend(file.facts.iter().cloned());
        report.parser.nodes += file.parser.nodes;
        report.parser.errors += file.parser.errors;
        report.parser.missing += file.parser.missing;
        if let Some(error) = &file.error {
            if !report
                .errors
                .iter()
                .any(|item| item.code == error.code && item.path == error.path)
            {
                report.errors.push(error.clone());
            }
        }
    }
    deduplicate_facts(&mut report.facts);
    for file in &mut files {
        deduplicate_facts(&mut file.facts);
    }

    BatchAnalysis { files, report }
}

#[derive(Debug)]
struct PreparedJava {
    input: SourceInput,
    language: Language,
    decoded: DecodedFile,
    package: String,
    imports: HashMap<String, String>,
    types: Vec<JavaType>,
    symbols: Vec<Symbol>,
    facts: Vec<Fact>,
    calls: Vec<PendingCall>,
    parser: ParserStats,
    error: Option<AnalysisError>,
    selected: bool,
}

#[derive(Debug, Clone)]
struct JavaType {
    fqn: String,
    simple: String,
    fields: HashMap<String, String>,
    methods: Vec<JavaMethod>,
    extends: Vec<String>,
    implements: Vec<String>,
}

#[derive(Debug, Clone)]
struct JavaMethod {
    name: String,
    arity: usize,
    key: String,
}

#[derive(Debug, Clone)]
struct PendingCall {
    subject_key: String,
    method_name: String,
    arity: usize,
    receiver: Option<String>,
    receiver_type: Option<String>,
    current_class: String,
    package: String,
    imports: HashMap<String, String>,
    utf8_start: usize,
    utf8_end: usize,
    reflection: bool,
    candidates_from_literals: Vec<String>,
}

fn prepare_java(
    input: &SourceInput,
    language: Language,
    decoded: DecodedFile,
    selected: bool,
) -> PreparedJava {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .expect("tree-sitter-java ABI mismatch");
    let Some(tree) = parser.parse(&decoded.text, None) else {
        return PreparedJava {
            input: input.clone(),
            language,
            decoded,
            package: String::new(),
            imports: HashMap::new(),
            types: Vec::new(),
            symbols: Vec::new(),
            facts: Vec::new(),
            calls: Vec::new(),
            parser: ParserStats {
                errors: 1,
                ..ParserStats::default()
            },
            error: Some(AnalysisError {
                path: input.path.clone(),
                code: "JAVA_PARSE_FAILED".into(),
                message: "Tree-sitter が構文木を返しませんでした".into(),
            }),
            selected,
        };
    };

    let root = tree.root_node();
    let mut parser_stats = ParserStats::default();
    count_parser_errors(root, &mut parser_stats);
    let mut package = String::new();
    let mut imports = HashMap::new();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        match child.kind() {
            "package_declaration" => {
                package = child
                    .child_by_field_name("name")
                    .or_else(|| child.named_child(0))
                    .map(|node| node_text(node, &decoded.text).to_string())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
            }
            "import_declaration" => {
                if let Some(name_node) = child
                    .child_by_field_name("name")
                    .or_else(|| child.named_child(0))
                {
                    let imported = normalize_type(node_text(name_node, &decoded.text));
                    if let Some(simple) = imported.rsplit('.').next() {
                        imports.insert(simple.to_string(), imported);
                    }
                }
            }
            _ => {}
        }
    }

    let mut prepared = PreparedJava {
        input: input.clone(),
        language,
        decoded,
        package,
        imports,
        types: Vec::new(),
        symbols: Vec::new(),
        facts: Vec::new(),
        calls: Vec::new(),
        parser: parser_stats,
        error: None,
        selected,
    };
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
        ) {
            parse_java_type(child, None, &mut prepared);
        }
    }
    prepared
}

fn parse_java_type(node: Node<'_>, outer: Option<&str>, prepared: &mut PreparedJava) {
    let Some(name_node) = node.child_by_field_name("name") else {
        return;
    };
    let simple = node_text(name_node, &prepared.decoded.text).to_string();
    let fqn = match outer {
        Some(parent) => format!("{parent}.{simple}"),
        None if prepared.package.is_empty() => simple.clone(),
        None => format!("{}.{simple}", prepared.package),
    };
    let kind = match node.kind() {
        "interface_declaration" => SymbolKind::Interface,
        "enum_declaration" => SymbolKind::Enum,
        _ => SymbolKind::Class,
    };
    let type_modifiers = modifiers(node, &prepared.decoded.text);
    prepared.symbols.push(Symbol {
        kind,
        fqn: fqn.clone(),
        simple_name: simple.clone(),
        signature: None,
        line_start: line_of(&prepared.decoded, name_node.start_byte()) as i32,
        line_end: line_of(&prepared.decoded, node.end_byte()) as i32,
        byte_start: raw_start(&prepared.decoded, name_node.start_byte()) as u64,
        byte_end: raw_end(&prepared.decoded, name_node.end_byte()) as u64,
        modifiers: type_modifiers,
        metadata: serde_json::json!({"path": prepared.input.path, "node": node.kind()}),
    });
    if let Some(evidence) = make_evidence(
        &prepared.input.path,
        &prepared.decoded,
        name_node.start_byte(),
        name_node.end_byte(),
    ) {
        prepared.facts.push(Fact::resolved(
            "DECLARES",
            FactSubject {
                kind: "Module".into(),
                key: prepared.input.path.clone(),
            },
            FactObject {
                kind: kind.entity_type().into(),
                key: fqn.clone(),
            },
            evidence,
        ));
    }

    let mut java_type = JavaType {
        fqn: fqn.clone(),
        simple: simple.clone(),
        fields: HashMap::new(),
        methods: Vec::new(),
        extends: Vec::new(),
        implements: Vec::new(),
    };

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "superclass" => {
                let text = node_text(child, &prepared.decoded.text);
                if let Some(name) = text.trim().strip_prefix("extends") {
                    java_type.extends.push(normalize_type(name));
                }
            }
            "super_interfaces" | "extends_interfaces" => {
                let text = node_text(child, &prepared.decoded.text);
                let list = text
                    .split_once("implements")
                    .or_else(|| text.split_once("extends"))
                    .map(|(_, value)| value)
                    .unwrap_or(text);
                for item in list.split(',') {
                    let normalized = normalize_type(item);
                    if normalized.is_empty() {
                        continue;
                    }
                    if child.kind() == "extends_interfaces" {
                        java_type.extends.push(normalized);
                    } else {
                        java_type.implements.push(normalized);
                    }
                }
            }
            "field_declaration" => {
                let field_type = child
                    .child_by_field_name("type")
                    .map(|value| normalize_type(node_text(value, &prepared.decoded.text)))
                    .unwrap_or_default();
                let mut walker = child.walk();
                for declarator in child
                    .named_children(&mut walker)
                    .filter(|item| item.kind() == "variable_declarator")
                {
                    if let Some(name) = declarator.child_by_field_name("name") {
                        let field_name = node_text(name, &prepared.decoded.text).to_string();
                        java_type
                            .fields
                            .insert(field_name.clone(), field_type.clone());
                        prepared.symbols.push(Symbol {
                            kind: SymbolKind::Field,
                            fqn: format!("{fqn}.{field_name}"),
                            simple_name: field_name,
                            signature: Some(field_type.clone()),
                            line_start: line_of(&prepared.decoded, name.start_byte()) as i32,
                            line_end: line_of(&prepared.decoded, declarator.end_byte()) as i32,
                            byte_start: raw_start(&prepared.decoded, name.start_byte()) as u64,
                            byte_end: raw_end(&prepared.decoded, name.end_byte()) as u64,
                            modifiers: modifiers(child, &prepared.decoded.text),
                            metadata: serde_json::json!({"path": prepared.input.path}),
                        });
                    }
                }
            }
            "method_declaration" | "constructor_declaration" => {
                if let Some(method) = parse_java_method(&child, &fqn, &java_type, prepared) {
                    java_type.methods.push(method);
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration" => {
                parse_java_type(child, Some(&fqn), prepared);
            }
            "class_body" | "interface_body" | "enum_body" | "record_body" => {
                parse_java_body(child, &fqn, prepared, &mut java_type);
            }
            _ => {}
        }
    }

    for target in &java_type.extends {
        if let Some(evidence) = make_evidence(
            &prepared.input.path,
            &prepared.decoded,
            name_node.start_byte(),
            name_node.end_byte(),
        ) {
            prepared.facts.push(Fact {
                relation: "EXTENDS".into(),
                subject: FactSubject {
                    kind: "Class".into(),
                    key: fqn.clone(),
                },
                object: FactObject {
                    kind: "Class".into(),
                    key: target.clone(),
                },
                resolution: FactResolution::Partial,
                confidence: 0.7,
                analysis_engine: "tree-sitter".into(),
                engine_version: ENGINE_VERSION.into(),
                unresolved_reason: Some("classpath 未指定のため外部型を解決しない".into()),
                candidates: vec![target.clone()],
                evidence: vec![evidence],
            });
        }
    }
    for target in &java_type.implements {
        if let Some(evidence) = make_evidence(
            &prepared.input.path,
            &prepared.decoded,
            name_node.start_byte(),
            name_node.end_byte(),
        ) {
            prepared.facts.push(Fact {
                relation: "IMPLEMENTS".into(),
                subject: FactSubject {
                    kind: "Class".into(),
                    key: fqn.clone(),
                },
                object: FactObject {
                    kind: "Interface".into(),
                    key: target.clone(),
                },
                resolution: FactResolution::Partial,
                confidence: 0.7,
                analysis_engine: "tree-sitter".into(),
                engine_version: ENGINE_VERSION.into(),
                unresolved_reason: Some("classpath 未指定のため外部型を解決しない".into()),
                candidates: vec![target.clone()],
                evidence: vec![evidence],
            });
        }
    }
    prepared.types.push(java_type);
}

fn parse_java_body(
    body: Node<'_>,
    class_fqn: &str,
    prepared: &mut PreparedJava,
    java_type: &mut JavaType,
) {
    let mut cursor = body.walk();
    let members = body.named_children(&mut cursor).collect::<Vec<_>>();
    for member in members {
        match member.kind() {
            "field_declaration" => {
                let field_type = member
                    .child_by_field_name("type")
                    .map(|value| normalize_type(node_text(value, &prepared.decoded.text)))
                    .unwrap_or_default();
                let member_modifiers = modifiers(member, &prepared.decoded.text);
                let mut walker = member.walk();
                let declarators = member
                    .named_children(&mut walker)
                    .filter(|item| item.kind() == "variable_declarator")
                    .collect::<Vec<_>>();
                for declarator in declarators {
                    if let Some(name) = declarator.child_by_field_name("name") {
                        let field_name = node_text(name, &prepared.decoded.text).to_string();
                        let field_fqn = format!("{class_fqn}.{field_name}");
                        java_type
                            .fields
                            .insert(field_name.clone(), field_type.clone());
                        prepared.symbols.push(Symbol {
                            kind: SymbolKind::Field,
                            fqn: field_fqn.clone(),
                            simple_name: field_name,
                            signature: Some(field_type.clone()),
                            line_start: line_of(&prepared.decoded, name.start_byte()) as i32,
                            line_end: line_of(&prepared.decoded, declarator.end_byte()) as i32,
                            byte_start: raw_start(&prepared.decoded, name.start_byte()) as u64,
                            byte_end: raw_end(&prepared.decoded, name.end_byte()) as u64,
                            modifiers: member_modifiers.clone(),
                            metadata: serde_json::json!({"path": prepared.input.path}),
                        });
                        if let Some(evidence) = make_evidence(
                            &prepared.input.path,
                            &prepared.decoded,
                            name.start_byte(),
                            name.end_byte(),
                        ) {
                            prepared.facts.push(Fact::resolved(
                                "DECLARES",
                                FactSubject {
                                    kind: "Class".into(),
                                    key: class_fqn.into(),
                                },
                                FactObject {
                                    kind: "Field".into(),
                                    key: field_fqn,
                                },
                                evidence,
                            ));
                        }
                    }
                }
            }
            "method_declaration" | "constructor_declaration" => {
                if let Some(method) = parse_java_method(&member, class_fqn, java_type, prepared) {
                    java_type.methods.push(method);
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration" => {
                parse_java_type(member, Some(class_fqn), prepared);
            }
            _ => {}
        }
    }
}

fn parse_java_method(
    node: &Node<'_>,
    class_fqn: &str,
    java_type: &JavaType,
    prepared: &mut PreparedJava,
) -> Option<JavaMethod> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, &prepared.decoded.text).to_string();
    let parameters = node
        .child_by_field_name("parameters")
        .map(|params| {
            let mut cursor = params.walk();
            params
                .named_children(&mut cursor)
                .filter(|param| {
                    matches!(
                        param.kind(),
                        "formal_parameter" | "spread_parameter" | "receiver_parameter"
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut parameter_types = Vec::new();
    let mut variable_types = java_type.fields.clone();
    for param in &parameters {
        let raw_type = param
            .child_by_field_name("type")
            .map(|value| normalize_type(node_text(value, &prepared.decoded.text)))
            .unwrap_or_else(|| "Object".into());
        parameter_types.push(raw_type.clone());
        if let Some(param_name) = param.child_by_field_name("name") {
            variable_types.insert(
                node_text(param_name, &prepared.decoded.text).to_string(),
                raw_type,
            );
        }
    }
    let key = format!("{class_fqn}.{name}({})", parameter_types.join(","));
    let arity = parameter_types.len();
    let method = JavaMethod {
        name: name.clone(),
        arity,
        key: key.clone(),
    };
    prepared.symbols.push(Symbol {
        kind: SymbolKind::Method,
        fqn: key.clone(),
        simple_name: name.clone(),
        signature: Some(format!("({})", parameter_types.join(","))),
        line_start: line_of(&prepared.decoded, name_node.start_byte()) as i32,
        line_end: line_of(&prepared.decoded, node.end_byte()) as i32,
        byte_start: raw_start(&prepared.decoded, name_node.start_byte()) as u64,
        byte_end: raw_end(&prepared.decoded, name_node.end_byte()) as u64,
        modifiers: modifiers(*node, &prepared.decoded.text),
        metadata: serde_json::json!({
            "path": prepared.input.path,
            "class": class_fqn,
            "arity": arity,
        }),
    });
    if let Some(evidence) = make_evidence(
        &prepared.input.path,
        &prepared.decoded,
        name_node.start_byte(),
        name_node.end_byte(),
    ) {
        prepared.facts.push(Fact::resolved(
            "DECLARES",
            FactSubject {
                kind: "Class".into(),
                key: class_fqn.into(),
            },
            FactObject {
                kind: "Method".into(),
                key: key.clone(),
            },
            evidence,
        ));
    }

    let Some(body) = node.child_by_field_name("body") else {
        return Some(method);
    };
    collect_variable_types(body, &prepared.decoded.text, &mut variable_types);
    collect_calls(
        body,
        &prepared.decoded.text,
        &key,
        class_fqn,
        &prepared.package,
        &prepared.imports,
        &variable_types,
        &mut prepared.calls,
        &mut prepared.facts,
        &prepared.input.path,
        &prepared.decoded,
    );
    let mut local_names = parameters
        .iter()
        .filter_map(|param| param.child_by_field_name("name"))
        .map(|name| node_text(name, &prepared.decoded.text).to_string())
        .collect::<BTreeSet<_>>();
    collect_local_names(body, &prepared.decoded.text, &mut local_names);
    collect_field_accesses(
        body,
        &prepared.decoded.text,
        &key,
        class_fqn,
        &java_type.fields,
        &local_names,
        &prepared.input.path,
        &prepared.decoded,
        &mut prepared.facts,
    );
    Some(method)
}

fn collect_local_names(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {
    if matches!(
        node.kind(),
        "variable_declarator"
            | "formal_parameter"
            | "spread_parameter"
            | "receiver_parameter"
            | "catch_formal_parameter"
    ) {
        if let Some(name) = node.child_by_field_name("name") {
            names.insert(node_text(name, source).to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_local_names(child, source, names);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_field_accesses(
    node: Node<'_>,
    source: &str,
    subject_key: &str,
    current_class: &str,
    fields: &HashMap<String, String>,
    local_names: &BTreeSet<String>,
    path: &str,
    decoded: &DecodedFile,
    facts: &mut Vec<Fact>,
) {
    if node.kind() == "identifier" {
        let name = node_text(node, source);
        if fields.contains_key(name)
            && (!local_names.contains(name)
                || is_explicit_current_field(node, source, current_class))
            && is_current_class_field_reference(node, source, current_class)
        {
            if let Some(evidence) = make_evidence(path, decoded, node.start_byte(), node.end_byte())
            {
                let relation = if is_write_target(node) {
                    "WRITES"
                } else {
                    "READS"
                };
                facts.push(Fact::resolved(
                    relation,
                    FactSubject {
                        kind: "Method".into(),
                        key: subject_key.into(),
                    },
                    FactObject {
                        kind: "Field".into(),
                        key: format!("{current_class}.{name}"),
                    },
                    evidence,
                ));
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_field_accesses(
            child,
            source,
            subject_key,
            current_class,
            fields,
            local_names,
            path,
            decoded,
            facts,
        );
    }
}

fn is_current_class_field_reference(node: Node<'_>, source: &str, current_class: &str) -> bool {
    let Some(parent) = node.parent() else {
        return true;
    };
    if parent.kind() == "method_invocation"
        && parent
            .child_by_field_name("name")
            .is_some_and(|name| name == node)
    {
        return false;
    }
    if parent.kind() != "field_access" {
        return true;
    }
    let field = parent.child_by_field_name("field");
    if field.is_some_and(|field| field != node) {
        return true;
    }
    let object = parent
        .child_by_field_name("object")
        .map(|object| node_text(object, source))
        .unwrap_or_default();
    is_current_object(object, current_class)
}

fn is_explicit_current_field(node: Node<'_>, source: &str, current_class: &str) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != "field_access"
        || parent
            .child_by_field_name("field")
            .is_none_or(|field| field != node)
    {
        return false;
    }
    parent
        .child_by_field_name("object")
        .map(|object| node_text(object, source))
        .is_some_and(|object| is_current_object(object, current_class))
}

fn is_current_object(object: &str, current_class: &str) -> bool {
    let class_simple = current_class.rsplit('.').next().unwrap_or(current_class);
    object == "this" || object == class_simple
}

fn is_write_target(node: Node<'_>) -> bool {
    let mut current = node;
    loop {
        let Some(parent) = current.parent() else {
            return false;
        };
        match parent.kind() {
            "update_expression" => return true,
            "assignment_expression" => {
                return parent.child_by_field_name("left").is_some_and(|left| {
                    left.start_byte() <= node.start_byte() && left.end_byte() >= node.end_byte()
                });
            }
            "method_invocation" | "block" | "class_body" => return false,
            _ => current = parent,
        }
    }
}

fn collect_variable_types(node: Node<'_>, source: &str, variables: &mut HashMap<String, String>) {
    if node.kind() == "local_variable_declaration" {
        let raw_type = node
            .child_by_field_name("type")
            .map(|value| normalize_type(node_text(value, source)))
            .unwrap_or_else(|| "Object".into());
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "variable_declarator" {
                if let Some(name) = child.child_by_field_name("name") {
                    variables.insert(node_text(name, source).to_string(), raw_type.clone());
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_variable_types(child, source, variables);
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_calls(
    node: Node<'_>,
    source: &str,
    subject_key: &str,
    current_class: &str,
    package: &str,
    imports: &HashMap<String, String>,
    variables: &HashMap<String, String>,
    calls: &mut Vec<PendingCall>,
    facts: &mut Vec<Fact>,
    path: &str,
    decoded: &DecodedFile,
) {
    match node.kind() {
        "method_invocation" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let method_name = node_text(name_node, source).to_string();
                let receiver = node
                    .child_by_field_name("object")
                    .map(|value| node_text(value, source).trim().to_string());
                let receiver_type = receiver
                    .as_deref()
                    .and_then(|receiver| resolve_receiver_hint(receiver, current_class, variables));
                let arguments = node
                    .child_by_field_name("arguments")
                    .map(|value| {
                        let mut cursor = value.walk();
                        value.named_children(&mut cursor).collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let literals = arguments
                    .iter()
                    .filter_map(|argument| string_literal_value(*argument, source))
                    .collect::<Vec<_>>();
                let reflection = matches!(
                    method_name.as_str(),
                    "forName" | "newProxyInstance" | "newInstance"
                ) || (method_name == "invoke"
                    && receiver_type
                        .as_deref()
                        .is_some_and(|value| value.ends_with("Method") || value == "Method"));
                let (utf8_start, utf8_end) = if reflection {
                    (name_node.start_byte(), node.end_byte())
                } else {
                    (node.start_byte(), node.end_byte())
                };
                calls.push(PendingCall {
                    subject_key: subject_key.into(),
                    method_name,
                    arity: arguments.len(),
                    receiver,
                    receiver_type,
                    current_class: current_class.into(),
                    package: package.into(),
                    imports: imports.clone(),
                    utf8_start,
                    utf8_end,
                    reflection,
                    candidates_from_literals: literals,
                });
            }
        }
        "string_literal" => {
            let literal = string_literal_value(node, source);
            if let Some(sql) = literal.filter(|value| looks_like_sql(value)) {
                let tables = sql_tables(&sql);
                if let Some(evidence) =
                    make_evidence(path, decoded, node.start_byte(), node.end_byte())
                {
                    for table in tables {
                        let dynamic = starts_dynamic_sql(&sql);
                        facts.push(Fact {
                            relation: "QUERIES".into(),
                            subject: FactSubject {
                                kind: "Method".into(),
                                key: subject_key.into(),
                            },
                            object: FactObject {
                                kind: "Table".into(),
                                key: table,
                            },
                            resolution: if dynamic {
                                FactResolution::Unresolved
                            } else {
                                FactResolution::Resolved
                            },
                            confidence: if dynamic { 0.35 } else { 0.95 },
                            analysis_engine: "tree-sitter-sql".into(),
                            engine_version: ENGINE_VERSION.into(),
                            unresolved_reason: dynamic
                                .then(|| "文字列連結またはプレースホルダで表名が動的".into()),
                            candidates: Vec::new(),
                            evidence: vec![evidence.clone()],
                        });
                    }
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_calls(
            child,
            source,
            subject_key,
            current_class,
            package,
            imports,
            variables,
            calls,
            facts,
            path,
            decoded,
        );
    }
}

fn resolve_receiver_hint(
    receiver: &str,
    current_class: &str,
    variables: &HashMap<String, String>,
) -> Option<String> {
    let receiver = receiver.trim();
    if receiver == "this" {
        return Some(current_class.into());
    }
    if receiver == "super" {
        return Some("super".into());
    }
    let tail = receiver
        .strip_prefix("this.")
        .unwrap_or(receiver)
        .rsplit('.')
        .next()
        .unwrap_or(receiver);
    if let Some(value) = variables.get(tail) {
        return Some(value.clone());
    }
    if receiver
        .chars()
        .next()
        .is_some_and(|value| value.is_uppercase())
    {
        return Some(normalize_type(receiver));
    }
    None
}

fn finalize_java(
    prepared: Vec<PreparedJava>,
    files: &mut Vec<SourceAnalysis>,
    report: &mut AnalysisReport,
) {
    let mut types = HashMap::<String, JavaType>::new();
    let mut by_simple = HashMap::<String, Vec<String>>::new();
    let mut methods_by_name = HashMap::<(String, usize), Vec<JavaMethod>>::new();
    for file in &prepared {
        for ty in &file.types {
            types.insert(ty.fqn.clone(), ty.clone());
            by_simple
                .entry(ty.simple.clone())
                .or_default()
                .push(ty.fqn.clone());
            for method in &ty.methods {
                methods_by_name
                    .entry((method.name.clone(), method.arity))
                    .or_default()
                    .push(method.clone());
            }
        }
    }

    for mut file in prepared {
        let mut facts = std::mem::take(&mut file.facts);
        let mut seen = BTreeSet::new();
        for call in file.calls {
            let mut receiver_fqn = call
                .receiver_type
                .as_deref()
                .and_then(|raw| resolve_type_fqn(raw, &call.package, &call.imports, &by_simple));
            if call.receiver.as_deref() == Some("super") {
                receiver_fqn = types
                    .get(&call.current_class)
                    .and_then(|ty| ty.extends.first())
                    .and_then(|raw| {
                        resolve_type_fqn(raw, &call.package, &call.imports, &by_simple)
                    });
            }
            if receiver_fqn.is_none()
                && call.receiver.is_none()
                && types.contains_key(&call.current_class)
            {
                receiver_fqn = Some(call.current_class.clone());
            }

            let mut candidates = receiver_fqn
                .as_ref()
                .and_then(|fqn| types.get(fqn))
                .map(|ty| {
                    ty.methods
                        .iter()
                        .filter(|method| {
                            method.name == call.method_name && method.arity == call.arity
                        })
                        .map(|method| method.key.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if candidates.is_empty() {
                candidates = methods_by_name
                    .get(&(call.method_name.clone(), call.arity))
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|method| method.key)
                    .collect();
            }
            let evidence = make_evidence(
                &file.input.path,
                &file.decoded,
                call.utf8_start,
                call.utf8_end,
            );
            let Some(evidence) = evidence else {
                continue;
            };
            if call.reflection {
                let object_key = call
                    .candidates_from_literals
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("{}:(dynamic)", call.method_name));
                let key = format!(
                    "reflection:{}:{}:{}",
                    call.subject_key, call.method_name, object_key
                );
                if seen.insert(key) {
                    facts.push(Fact::unresolved(
                        "CALLS",
                        FactSubject {
                            kind: "Method".into(),
                            key: call.subject_key.clone(),
                        },
                        FactObject {
                            kind: "Method".into(),
                            key: object_key,
                        },
                        evidence,
                        call.candidates_from_literals,
                        "反射・動的プロキシは実行時型のため静的解決しない",
                    ));
                }
            } else if candidates.len() == 1 {
                let object_key = candidates.remove(0);
                let key = format!("CALLS:{}:{object_key}", call.subject_key);
                if seen.insert(key) {
                    facts.push(Fact::resolved(
                        "CALLS",
                        FactSubject {
                            kind: "Method".into(),
                            key: call.subject_key.clone(),
                        },
                        FactObject {
                            kind: "Method".into(),
                            key: object_key,
                        },
                        evidence,
                    ));
                }
            } else if candidates.is_empty() {
                let receiver_was_present = call.receiver.is_some();
                let receiver = call.receiver.unwrap_or_else(|| "<this>".into());
                let object_key = format!("{receiver}.{}()", call.method_name);
                let key = format!("CALLS:{}:{object_key}", call.subject_key);
                if seen.insert(key) {
                    facts.push(Fact::unresolved(
                        "CALLS",
                        FactSubject {
                            kind: "Method".into(),
                            key: call.subject_key.clone(),
                        },
                        FactObject {
                            kind: "Method".into(),
                            key: object_key,
                        },
                        evidence,
                        Vec::new(),
                        if receiver_was_present {
                            "呼び出し先の型・シグネチャを確定できない"
                        } else {
                            "同じクラス内に一致するメソッド宣言が無い"
                        },
                    ));
                }
            } else {
                let key = format!("CALLS:{}:{}", call.subject_key, candidates.join("|"));
                if seen.insert(key) {
                    let primary = candidates[0].clone();
                    facts.push(Fact::inferred(
                        "CALLS",
                        FactSubject {
                            kind: "Method".into(),
                            key: call.subject_key.clone(),
                        },
                        FactObject {
                            kind: "Method".into(),
                            key: primary,
                        },
                        evidence,
                        candidates,
                        "同名・同引数の候補が複数ある",
                    ));
                }
            }
        }
        deduplicate_facts(&mut facts);
        report.classpath_complete = false;
        if file.selected {
            files.push(SourceAnalysis {
                path: file.input.path,
                language: file.language.as_str().into(),
                content_hash: Sha256::digest(&file.input.bytes).to_vec(),
                size_bytes: file.input.bytes.len() as i64,
                encoding: file.decoded.encoding.name().to_string(),
                exact_offsets: file.decoded.is_exact(),
                loc: file
                    .input
                    .bytes
                    .iter()
                    .filter(|value| **value == b'\n')
                    .count() as i32
                    + i32::from(!file.input.bytes.is_empty()),
                line_offsets: line_offsets_bytes(&file.decoded),
                symbols: file.symbols,
                facts,
                parser: file.parser,
                error: file.error,
            });
        }
    }
}

fn analyze_jsp(input: &SourceInput, language: Language, decoded: DecodedFile) -> SourceAnalysis {
    let (symbols, facts, parser, error) = match decoded.exact {
        true => {
            let analysis = jsp::analyze(&decoded.text);
            let mut facts = Vec::new();
            let source_key = format!("Jsp:{}", input.path);
            for (tag, action) in form_actions(&decoded.text) {
                let start = decoded.text.find(&tag).unwrap_or(0);
                if let Some(evidence) = make_evidence(
                    &input.path,
                    &decoded,
                    start,
                    start.saturating_add(tag.len()),
                ) {
                    facts.push(Fact {
                        relation: "SUBMITS_TO".into(),
                        subject: FactSubject {
                            kind: "Screen".into(),
                            key: source_key.clone(),
                        },
                        object: FactObject {
                            kind: "Screen".into(),
                            key: action,
                        },
                        resolution: FactResolution::Partial,
                        confidence: 0.65,
                        analysis_engine: "jsp-html".into(),
                        engine_version: ENGINE_VERSION.into(),
                        unresolved_reason: Some("画面送信先は実行時設定にも依存する".into()),
                        candidates: Vec::new(),
                        evidence: vec![evidence],
                    });
                }
            }
            let parser = ParserStats {
                nodes: (analysis.html.nodes + analysis.java.nodes + analysis.members.nodes) as u64,
                errors: (analysis.html.errors + analysis.java.errors + analysis.members.errors)
                    as u64,
                missing: (analysis.html.missing + analysis.java.missing + analysis.members.missing)
                    as u64,
            };
            let error = (!analysis.html.is_clean()).then(|| AnalysisError {
                path: input.path.clone(),
                code: "E-JSP-002".into(),
                message: format!(
                    "HTML マスク後の構文エラー: {} 件、マスク対象 {} ブロック",
                    analysis.html.errors + analysis.html.missing,
                    analysis.block_count
                ),
            });
            (Vec::new(), facts, parser, error)
        }
        false => (
            Vec::new(),
            Vec::new(),
            ParserStats::default(),
            Some(AnalysisError {
                path: input.path.clone(),
                code: "E-JSP-003".into(),
                message: "元バイト偏移を確定できないため JSP を解析しない".into(),
            }),
        ),
    };
    source_analysis(input, language, &decoded, symbols, facts, parser, error)
}

fn analyze_xml(input: &SourceInput, language: Language, decoded: DecodedFile) -> SourceAnalysis {
    let mut facts = Vec::new();
    let source = decoded.text.as_str();
    let path = input.path.as_str();

    for tag in element_tags(source, "action") {
        let Some(action_type) = attribute(&tag, "type") else {
            continue;
        };
        let Some(action_path) = attribute(&tag, "path") else {
            continue;
        };
        let start = find_element_start(source, &tag).unwrap_or(0);
        if let Some(evidence) = make_evidence(path, &decoded, start, start + tag.len()) {
            facts.push(Fact::resolved(
                "HANDLES",
                FactSubject {
                    kind: "Action".into(),
                    key: action_type,
                },
                FactObject {
                    kind: "Screen".into(),
                    key: action_path,
                },
                evidence,
            ));
        }
    }
    for tag in element_tags(source, "forward") {
        let Some(forward_path) = attribute(&tag, "path") else {
            continue;
        };
        let start = find_element_start(source, &tag).unwrap_or(0);
        if let Some(evidence) = make_evidence(path, &decoded, start, start + tag.len()) {
            facts.push(Fact {
                relation: "DISPATCHES".into(),
                subject: FactSubject {
                    kind: "Configuration".into(),
                    key: path.into(),
                },
                object: FactObject {
                    kind: "Screen".into(),
                    key: forward_path,
                },
                resolution: FactResolution::Partial,
                confidence: 0.75,
                analysis_engine: "struts-xml".into(),
                engine_version: ENGINE_VERSION.into(),
                unresolved_reason: Some("forward 元 Action との対応は親要素走査で確定する".into()),
                candidates: Vec::new(),
                evidence: vec![evidence],
            });
        }
    }
    for tag in element_tags(source, "component") {
        let Some(component_name) = attribute(&tag, "name") else {
            continue;
        };
        let Some(class_name) = attribute(&tag, "class") else {
            continue;
        };
        let start = find_element_start(source, &tag).unwrap_or(0);
        if let Some(evidence) = make_evidence(path, &decoded, start, start + tag.len()) {
            facts.push(Fact::resolved(
                "DEPENDS_ON",
                FactSubject {
                    kind: "Component".into(),
                    key: component_name,
                },
                FactObject {
                    kind: "Class".into(),
                    key: class_name,
                },
                evidence,
            ));
        }
    }

    let mapper_namespace = attribute(source, "namespace");
    for statement_kind in ["select", "insert", "update", "delete"] {
        for body in element_bodies(source, statement_kind) {
            let dynamic = body.contains("<if")
                || body.contains("<choose")
                || body.contains("${")
                || body.contains("#{");
            let tables = sql_tables(&body);
            let start = find_element_start(source, &body).unwrap_or(0);
            for table in tables {
                if let Some(evidence) =
                    make_evidence(path, &decoded, start, start.saturating_add(body.len()))
                {
                    facts.push(Fact {
                        relation: "QUERIES".into(),
                        subject: FactSubject {
                            kind: "Component".into(),
                            key: mapper_namespace.clone().unwrap_or_else(|| path.to_string()),
                        },
                        object: FactObject {
                            kind: "Table".into(),
                            key: table,
                        },
                        resolution: if dynamic {
                            FactResolution::Partial
                        } else {
                            FactResolution::Resolved
                        },
                        confidence: if dynamic { 0.65 } else { 0.96 },
                        analysis_engine: "mybatis-xml".into(),
                        engine_version: ENGINE_VERSION.into(),
                        unresolved_reason: dynamic
                            .then(|| "MyBatis 動的 SQL のため全分岐を列挙していない".into()),
                        candidates: Vec::new(),
                        evidence: vec![evidence],
                    });
                }
            }
        }
    }
    if facts.is_empty() {
        facts.push(Fact {
            relation: "DECLARES".into(),
            subject: FactSubject {
                kind: "Module".into(),
                key: path.into(),
            },
            object: FactObject {
                kind: "Configuration".into(),
                key: path.into(),
            },
            resolution: FactResolution::Resolved,
            confidence: 1.0,
            analysis_engine: "xml".into(),
            engine_version: ENGINE_VERSION.into(),
            unresolved_reason: None,
            candidates: Vec::new(),
            evidence: make_evidence(path, &decoded, 0, first_line_len(source))
                .into_iter()
                .collect(),
        });
    }
    source_analysis(
        input,
        language,
        &decoded,
        Vec::new(),
        facts,
        ParserStats {
            nodes: source.matches('<').count() as u64,
            ..ParserStats::default()
        },
        None,
    )
}

fn analyze_sql_file(
    input: &SourceInput,
    language: Language,
    decoded: DecodedFile,
) -> SourceAnalysis {
    let start = decoded
        .text
        .find(|value: char| !value.is_whitespace())
        .unwrap_or(0);
    let mut facts = Vec::new();
    if let Some(evidence) =
        make_evidence(&input.path, &decoded, start, first_line_len(&decoded.text))
    {
        for table in sql_tables(&decoded.text) {
            facts.push(Fact::resolved(
                "QUERIES",
                FactSubject {
                    kind: "SqlStatement".into(),
                    key: input.path.clone(),
                },
                FactObject {
                    kind: "Table".into(),
                    key: table,
                },
                evidence.clone(),
            ));
        }
    }
    source_analysis(
        input,
        language,
        &decoded,
        Vec::new(),
        facts,
        ParserStats {
            nodes: decoded.text.matches(';').count().max(1) as u64,
            ..ParserStats::default()
        },
        None,
    )
}

fn skipped_source(input: &SourceInput, language: Language, error: AnalysisError) -> SourceAnalysis {
    SourceAnalysis {
        path: input.path.clone(),
        language: language.as_str().into(),
        content_hash: Sha256::digest(&input.bytes).to_vec(),
        size_bytes: input.bytes.len() as i64,
        encoding: "UNKNOWN".into(),
        exact_offsets: false,
        loc: 0,
        line_offsets: Vec::new(),
        symbols: Vec::new(),
        facts: Vec::new(),
        parser: ParserStats::default(),
        error: Some(error),
    }
}

fn source_analysis(
    input: &SourceInput,
    language: Language,
    decoded: &DecodedFile,
    symbols: Vec<Symbol>,
    facts: Vec<Fact>,
    parser: ParserStats,
    error: Option<AnalysisError>,
) -> SourceAnalysis {
    SourceAnalysis {
        path: input.path.clone(),
        language: language.as_str().into(),
        content_hash: Sha256::digest(&input.bytes).to_vec(),
        size_bytes: input.bytes.len() as i64,
        encoding: decoded.encoding.name().to_string(),
        exact_offsets: decoded.is_exact(),
        loc: decoded.line_index().starts().len() as i32,
        line_offsets: line_offsets_bytes(decoded),
        symbols,
        facts,
        parser,
        error,
    }
}

fn line_offsets_bytes(decoded: &DecodedFile) -> Vec<u8> {
    decoded
        .line_index()
        .starts()
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn make_evidence(
    path: &str,
    decoded: &DecodedFile,
    utf8_start: usize,
    utf8_end: usize,
) -> Option<Evidence> {
    if !decoded.is_exact() || utf8_start >= utf8_end || utf8_end > decoded.text.len() {
        return None;
    }
    let mut start = utf8_start;
    let mut end = utf8_end.min(decoded.text.len());
    while !decoded.text.is_char_boundary(start) {
        start += 1;
    }
    while !decoded.text.is_char_boundary(end) {
        end -= 1;
    }
    while end.saturating_sub(start) > 512 {
        end -= 1;
        while !decoded.text.is_char_boundary(end) {
            end -= 1;
        }
    }
    let quote = decoded.text.get(start..end)?.to_string();
    let (byte_start, byte_end) = decoded.byte_range_of_utf8(start, end)?;
    let (line, column) = decoded.line_column_of_utf8(start)?;
    Some(Evidence {
        path: path.into(),
        byte_start: byte_start as u64,
        byte_end: byte_end as u64,
        line: line as i32,
        column: column as i32,
        quote,
    })
}

fn raw_start(decoded: &DecodedFile, utf8: usize) -> usize {
    decoded
        .raw_byte_of_utf8(utf8)
        .map(|value| value as usize)
        .unwrap_or(0)
}

fn raw_end(decoded: &DecodedFile, utf8: usize) -> usize {
    decoded
        .raw_byte_of_utf8(utf8)
        .map(|value| value as usize)
        .unwrap_or(0)
}

fn line_of(decoded: &DecodedFile, utf8: usize) -> usize {
    decoded
        .line_column_of_utf8(utf8)
        .map(|(line, _)| line as usize)
        .unwrap_or(1)
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source
        .get(node.start_byte()..node.end_byte())
        .unwrap_or_default()
}

fn modifiers(node: Node<'_>, source: &str) -> Vec<String> {
    let mut cursor = node.walk();
    let result = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "modifiers")
        .map(|child| {
            node_text(child, source)
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    result
}

fn count_parser_errors(node: Node<'_>, stats: &mut ParserStats) {
    stats.nodes += 1;
    if node.is_error() {
        stats.errors += 1;
    }
    if node.is_missing() {
        stats.missing += 1;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        count_parser_errors(child, stats);
    }
}

fn resolve_type_fqn(
    raw: &str,
    package: &str,
    imports: &HashMap<String, String>,
    by_simple: &HashMap<String, Vec<String>>,
) -> Option<String> {
    let normalized = normalize_type(raw);
    if normalized.is_empty() || normalized == "super" {
        return None;
    }
    let simple = normalized.rsplit('.').next().unwrap_or(&normalized);
    if normalized.contains('.') {
        return Some(normalized);
    }
    if let Some(imported) = imports.get(simple) {
        return Some(imported.clone());
    }
    let same_package = if package.is_empty() {
        simple.to_string()
    } else {
        format!("{package}.{simple}")
    };
    if by_simple
        .get(simple)
        .is_some_and(|items| items.contains(&same_package))
    {
        return Some(same_package);
    }
    by_simple
        .get(simple)
        .filter(|items| items.len() == 1)
        .and_then(|items| items.first().cloned())
}

fn normalize_type(raw: &str) -> String {
    let without_annotations = raw
        .split_whitespace()
        .filter(|part| !part.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ");
    let mut value = without_annotations
        .trim()
        .replace(" extends ", "")
        .replace(" implements ", "");
    if let Some(index) = value.find('<') {
        value.truncate(index);
    }
    value
        .trim()
        .trim_matches(|value: char| value == '[' || value == ']')
        .trim_end_matches("...")
        .to_string()
}

fn string_literal_value(node: Node<'_>, source: &str) -> Option<String> {
    let raw = node_text(node, source).trim();
    if raw.len() < 2 {
        return None;
    }
    let quote = raw.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    Some(
        raw[quote.len_utf8()..raw.len().saturating_sub(quote.len_utf8())]
            .replace("\\\"", "\"")
            .replace("\\n", "\n"),
    )
}

fn looks_like_sql(value: &str) -> bool {
    let value = value.trim_start().to_ascii_uppercase();
    ["SELECT ", "INSERT ", "UPDATE ", "DELETE ", "WITH "]
        .iter()
        .any(|prefix| value.starts_with(prefix))
}

fn starts_dynamic_sql(value: &str) -> bool {
    value.contains("${")
        || value.contains("#{")
        || value.contains("\" +")
        || value.contains(" + \"")
        || value.contains("? +")
}

fn sql_tables(sql: &str) -> Vec<String> {
    let tokens = sql
        .split(|value: char| !(value.is_alphanumeric() || value == '_' || value == '.'))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let mut tables = BTreeSet::new();
    for (index, token) in tokens.iter().enumerate() {
        if matches!(
            token.to_ascii_uppercase().as_str(),
            "FROM" | "JOIN" | "UPDATE" | "INTO"
        ) {
            if let Some(next) = tokens.get(index + 1) {
                let next_upper = next.to_ascii_uppercase();
                if !matches!(next_upper.as_str(), "SELECT" | "SET" | "VALUES" | "WHERE") {
                    tables.insert(next.trim_matches('.').to_string());
                }
            }
        }
    }
    tables.into_iter().collect()
}

fn form_actions(source: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for name in ["form", "html:form"] {
        for tag in element_tags(source, name) {
            if let Some(action) = attribute(&tag, "action") {
                results.push((tag, action));
            }
        }
    }
    results
}

fn element_tags(source: &str, name: &str) -> Vec<String> {
    let needle = format!("<{name}");
    let mut values = Vec::new();
    let mut offset = 0usize;
    while let Some(relative) = source[offset..].find(&needle) {
        let start = offset + relative;
        let after = source.as_bytes().get(start + needle.len());
        if after
            .is_some_and(|value| value.is_ascii_alphanumeric() || *value == b':' || *value == b'-')
        {
            offset = start + needle.len();
            continue;
        }
        let Some(end_relative) = source[start..].find('>') else {
            break;
        };
        let end = start + end_relative + 1;
        values.push(source[start..end].to_string());
        offset = end;
    }
    values
}

fn element_bodies(source: &str, name: &str) -> Vec<String> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut values = Vec::new();
    let mut offset = 0usize;
    while let Some(relative) = source[offset..].find(&open) {
        let start = offset + relative;
        let Some(open_end_relative) = source[start..].find('>') else {
            break;
        };
        let body_start = start + open_end_relative + 1;
        let Some(close_relative) = source[body_start..].find(&close) else {
            break;
        };
        let body_end = body_start + close_relative;
        values.push(source[body_start..body_end].to_string());
        offset = body_end + close.len();
    }
    values
}

fn attribute(source: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{name}={quote}");
        if let Some(start) = source.find(&needle) {
            let value_start = start + needle.len();
            if let Some(end) = source[value_start..].find(quote) {
                return Some(source[value_start..value_start + end].to_string());
            }
        }
        let spaced = format!("{name} = {quote}");
        if let Some(start) = source.find(&spaced) {
            let value_start = start + spaced.len();
            if let Some(end) = source[value_start..].find(quote) {
                return Some(source[value_start..value_start + end].to_string());
            }
        }
    }
    None
}

fn find_element_start(source: &str, element: &str) -> Option<usize> {
    source.find(element)
}

fn first_line_len(source: &str) -> usize {
    source
        .find('\n')
        .map(|index| index.saturating_add(1))
        .unwrap_or(source.len())
        .min(512)
}

fn deduplicate_facts(facts: &mut Vec<Fact>) {
    let mut by_key: BTreeMap<String, Fact> = BTreeMap::new();
    for fact in facts.drain(..) {
        let key = format!(
            "{}|{}|{}|{}",
            fact.subject.key, fact.relation, fact.object.key, fact.analysis_engine
        );
        match by_key.get(&key) {
            Some(existing) if existing.resolution.rank() >= fact.resolution.rank() => {}
            _ => {
                by_key.insert(key, fact);
            }
        }
    }
    *facts = by_key.into_values().collect();
}

pub fn normalize_nfc(value: &str) -> String {
    value.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(path: &str, source: &str) -> SourceInput {
        SourceInput {
            path: path.into(),
            bytes: source.as_bytes().to_vec(),
        }
    }

    #[test]
    fn java_source_produces_symbols_and_calls() {
        let batch = analyze_sources(
            &[
                input(
                    "src/UserService.java",
                    "package demo; public class UserService { public void update() {} }",
                ),
                input(
                    "src/UserAction.java",
                    "package demo; public class UserAction { private UserService service; void save() { service.update(); } }",
                ),
            ],
            &AnalyzeOptions::default(),
        );
        assert!(batch
            .report
            .symbols
            .iter()
            .any(|symbol| symbol.fqn == "demo.UserAction.save()"));
        let call = batch
            .report
            .facts
            .iter()
            .find(|fact| fact.relation == "CALLS")
            .expect("CALLS fact");
        assert_eq!(call.resolution, FactResolution::Resolved);
        assert_eq!(call.object.key, "demo.UserService.update()");
    }

    #[test]
    fn reflection_is_never_resolved() {
        let batch = analyze_sources(
            &[input(
                "src/Reflective.java",
                r#"package demo; public class Reflective { void run() throws Exception { Class.forName("demo.UserService"); } }"#,
            )],
            &AnalyzeOptions::default(),
        );
        let calls = batch
            .report
            .facts
            .iter()
            .filter(|fact| fact.relation == "CALLS")
            .collect::<Vec<_>>();
        assert!(!calls.is_empty());
        assert!(calls
            .iter()
            .all(|fact| fact.resolution != FactResolution::Resolved));
    }

    #[test]
    fn java_field_reads_and_writes_are_facts() {
        let batch = analyze_sources(
            &[input(
                "src/UserService.java",
                r#"
                package demo;
                public class UserService {
                  private String name;
                  private int count;
                  public String update(String name) {
                    this.name = name;
                    count++;
                    return this.name + count;
                  }
                }
                "#,
            )],
            &AnalyzeOptions::default(),
        );
        assert!(batch.report.facts.iter().any(|fact| {
            fact.relation == "WRITES"
                && fact.subject.key == "demo.UserService.update(String)"
                && fact.object.key == "demo.UserService.name"
        }));
        assert!(batch.report.facts.iter().any(|fact| {
            fact.relation == "READS"
                && fact.subject.key == "demo.UserService.update(String)"
                && fact.object.key == "demo.UserService.name"
        }));
        assert!(batch.report.facts.iter().any(|fact| {
            fact.relation == "WRITES" && fact.object.key == "demo.UserService.count"
        }));
    }

    #[test]
    fn struts_config_creates_handles() {
        let batch = analyze_sources(
            &[input(
                "WEB-INF/struts-config.xml",
                r#"<struts-config><action-mappings><action path="/userEdit" type="demo.UserEditAction"/></action-mappings></struts-config>"#,
            )],
            &AnalyzeOptions::default(),
        );
        assert!(batch.report.facts.iter().any(|fact| {
            fact.relation == "HANDLES"
                && fact.subject.key == "demo.UserEditAction"
                && fact.object.key == "/userEdit"
        }));
    }

    #[test]
    fn sql_tables_are_identified_without_keyword_confusion() {
        assert_eq!(
            sql_tables("SELECT u.id FROM app_user u JOIN address a ON a.id=u.address_id"),
            vec!["address".to_string(), "app_user".to_string()]
        );
    }
}
