use coralc::ast::*;
use coralc::lexer::lex;
use coralc::parser::Parser;
use pretty_assertions::assert_eq;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn parser_valid_snapshots_match() {
    for fixture in collect_fixtures("tests/fixtures/parser/valid") {
        let source = fs::read_to_string(&fixture).expect("failed to read fixture");
        let tokens = lex(&source).expect("lexing failed");
        let parser = Parser::new(tokens, source.len());
        let program = parser.parse().expect("fixture should parse");
        let actual = program_snapshot(&program);
        let snapshot_path = snapshot_path(&fixture);
        if !snapshot_path.exists() {
            panic!(
                "missing snapshot for {:?}. write the following to {:?}:\n{}",
                fixture,
                snapshot_path,
                serde_json::to_string_pretty(&actual).unwrap()
            );
        }
        let expected = read_snapshot(&snapshot_path);
        assert_eq!(
            actual,
            expected,
            "AST snapshot mismatch for {:?}. update {:?}",
            fixture,
            snapshot_path
        );
    }
}

fn collect_fixtures(dir: &str) -> Vec<PathBuf> {
    let mut fixtures = Vec::new();
    if let Ok(read_dir) = fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("coral") {
                fixtures.push(path);
            }
        }
    }
    fixtures.sort();
    fixtures
}

fn snapshot_path(fixture: &Path) -> PathBuf {
    let mut path = fixture.to_path_buf();
    path.set_extension("ast.json");
    path
}

fn read_snapshot(path: &Path) -> Value {
    let contents = fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("failed to read snapshot {:?}", path));
    serde_json::from_str(&contents)
        .unwrap_or_else(|_| panic!("snapshot {:?} contains invalid JSON", path))
}

fn program_snapshot(program: &Program) -> Value {
    let items: Vec<Value> = program.items.iter().map(item_snapshot).collect();
    json!({ "items": items })
}

fn item_snapshot(item: &Item) -> Value {
    match item {
        Item::Binding(binding) => json!({ "binding": binding_snapshot(binding) }),
        Item::Function(function) => json!({ "function": function_snapshot(function) }),
        Item::Type(ty) => json!({ "type": type_snapshot(ty) }),
        Item::Store(store) => json!({ "store": store_snapshot(store) }),
        Item::Taxonomy(taxonomy) => json!({ "taxonomy": taxonomy_snapshot(taxonomy) }),
        Item::Expression(expr) => json!({ "expression": expression_snapshot(expr) }),
        Item::ExternFunction(extern_fn) => json!({
            "extern_fn": {
                "name": extern_fn.name,
                "params": extern_fn.params.iter().map(parameter_snapshot).collect::<Vec<_>>(),
                "return_type": extern_fn.return_type.as_ref().map(|t| t.segments.join("."))
            }
        }),
        Item::ErrorDefinition(err_def) => json!({
            "error_def": error_def_snapshot(err_def)
        }),
        Item::TraitDefinition(trait_def) => json!({
            "trait_def": trait_def_snapshot(trait_def)
        }),
    }
}

fn binding_snapshot(binding: &Binding) -> Value {
    json!({
        "name": binding.name,
        "value": expression_snapshot(&binding.value)
    })
}

fn function_snapshot(function: &Function) -> Value {
    json!({
        "name": function.name,
        "params": function.params.iter().map(parameter_snapshot).collect::<Vec<_>>(),
        "body": block_snapshot(&function.body),
        "kind": match function.kind {
            FunctionKind::Free => "Free",
            FunctionKind::Method => "Method",
            FunctionKind::ActorMessage => "ActorMessage",
        }
    })
}

fn parameter_snapshot(param: &Parameter) -> Value {
    json!({
        "name": param.name,
        "default": param.default.as_ref().map(expression_snapshot)
    })
}

fn block_snapshot(block: &Block) -> Value {
    let statements: Vec<Value> = block.statements.iter().map(statement_snapshot).collect();
    json!({
        "statements": statements,
        "value": block.value.as_ref().map(|expr| expression_snapshot(expr))
    })
}

fn statement_snapshot(statement: &Statement) -> Value {
    match statement {
        Statement::Binding(binding) => json!({ "binding": binding_snapshot(binding) }),
        Statement::Expression(expr) => json!({ "expression": expression_snapshot(expr) }),
        Statement::Return(expr, _) => json!({ "return": expression_snapshot(expr) }),
    }
}

fn type_snapshot(ty: &TypeDefinition) -> Value {
    let mut obj = json!({
        "name": ty.name,
        "fields": ty.fields.iter().map(field_snapshot).collect::<Vec<_>>(),
        "methods": ty.methods.iter().map(function_snapshot).collect::<Vec<_>>()
    });
    
    // Include variants if this is an enum/ADT
    if !ty.variants.is_empty() {
        obj["variants"] = json!(ty.variants.iter().map(variant_snapshot).collect::<Vec<_>>());
    }
    
    obj
}

fn variant_snapshot(variant: &TypeVariant) -> Value {
    json!({
        "name": variant.name,
        "fields": variant.fields.iter().map(|f| {
            json!({
                "name": f.name.clone().unwrap_or_default()
            })
        }).collect::<Vec<_>>()
    })
}

fn error_def_snapshot(err_def: &coralc::ast::ErrorDefinition) -> Value {
    let mut obj = json!({
        "name": err_def.name,
    });
    if let Some(code) = err_def.code {
        obj["code"] = json!(code);
    }
    if let Some(ref message) = err_def.message {
        obj["message"] = json!(message);
    }
    if !err_def.children.is_empty() {
        obj["children"] = json!(err_def.children.iter().map(error_def_snapshot).collect::<Vec<_>>());
    }
    obj
}

fn trait_def_snapshot(trait_def: &coralc::ast::TraitDefinition) -> Value {
    json!({
        "name": trait_def.name,
        "required_traits": trait_def.required_traits,
        "methods": trait_def.methods.iter().map(trait_method_snapshot).collect::<Vec<_>>()
    })
}

fn trait_method_snapshot(method: &coralc::ast::TraitMethod) -> Value {
    let mut obj = json!({
        "name": method.name,
        "params": method.params.iter().map(parameter_snapshot).collect::<Vec<_>>(),
    });
    if let Some(ref body) = method.body {
        obj["body"] = json!(body.statements.iter().map(statement_snapshot).collect::<Vec<_>>());
    }
    obj
}

fn store_snapshot(store: &StoreDefinition) -> Value {
    json!({
        "name": store.name,
        "is_actor": store.is_actor,
        "fields": store.fields.iter().map(field_snapshot).collect::<Vec<_>>(),
        "methods": store.methods.iter().map(function_snapshot).collect::<Vec<_>>()
    })
}

fn taxonomy_snapshot(node: &TaxonomyNode) -> Value {
    json!({
        "name": node.name,
        "bindings": node.bindings.iter().map(binding_snapshot).collect::<Vec<_>>(),
        "children": node.children.iter().map(taxonomy_snapshot).collect::<Vec<_>>()
    })
}

fn field_snapshot(field: &Field) -> Value {
    json!({
        "name": field.name,
        "is_reference": field.is_reference,
        "default": field.default.as_ref().map(expression_snapshot)
    })
}

fn expression_snapshot(expr: &Expression) -> Value {
    match expr {
        Expression::Unit => json!({ "unit": null }),
        Expression::None(_) => json!({ "none": null }),
        Expression::Identifier(name, _) => json!({ "identifier": name }),
        Expression::Integer(value, _) => json!({ "integer": value }),
        Expression::Float(value, _) => json!({ "float": value }),
        Expression::Bool(value, _) => json!({ "bool": value }),
        Expression::String(value, _) => json!({ "string": value }),
    Expression::Bytes(bytes, _) => json!({ "bytes": bytes }),
        Expression::Placeholder(index, _) => json!({ "placeholder": index }),
        Expression::TaxonomyPath { segments, .. } => json!({ "taxonomy_path": segments }),
        Expression::Throw { value, .. } => json!({ "throw": expression_snapshot(value) }),
        Expression::Lambda { params, body, .. } => json!({
            "lambda": {
                "params": params.iter().map(parameter_snapshot).collect::<Vec<_>>(),
                "body": block_snapshot(body)
            }
        }),
        Expression::List(items, _) => json!({
            "list": items.iter().map(expression_snapshot).collect::<Vec<_>>()
        }),
        Expression::Map(entries, _) => json!({
            "map": entries
                .iter()
                .map(|(key, value)| json!({
                    "key": expression_snapshot(key),
                    "value": expression_snapshot(value)
                }))
                .collect::<Vec<_>>()
        }),
        Expression::Binary { op, left, right, .. } => json!({
            "binary": {
                "op": binary_op_name(*op),
                "left": expression_snapshot(left),
                "right": expression_snapshot(right)
            }
        }),
        Expression::Unary { op, expr, .. } => json!({
            "unary": {
                "op": unary_op_name(*op),
                "expr": expression_snapshot(expr)
            }
        }),
        Expression::Call { callee, args, .. } => json!({
            "call": {
                "callee": expression_snapshot(callee),
                "args": args.iter().map(expression_snapshot).collect::<Vec<_>>()
            }
        }),
        Expression::Member { target, property, .. } => json!({
            "member": {
                "target": expression_snapshot(target),
                "property": property
            }
        }),
        Expression::Ternary { condition, then_branch, else_branch, .. } => json!({
            "ternary": {
                "condition": expression_snapshot(condition),
                "then": expression_snapshot(then_branch),
                "else": expression_snapshot(else_branch)
            }
        }),
        Expression::Match(match_expr) => json!({
            "match": {
                "value": expression_snapshot(&match_expr.value),
                "arms": match_expr
                    .arms
                    .iter()
                    .map(|arm| json!({
                        "pattern": pattern_snapshot(&arm.pattern),
                        "body": block_snapshot(&arm.body)
                    }))
                    .collect::<Vec<_>>(),
                "default": match_expr
                    .default
                    .as_ref()
                    .map(|block| block_snapshot(block))
            }
        }),
        Expression::InlineAsm { template, inputs, .. } => json!({
            "asm": {
                "template": template,
                "inputs": inputs.iter().map(|(constraint, expr)| json!({
                    "constraint": constraint,
                    "expr": expression_snapshot(expr)
                })).collect::<Vec<_>>()
            }
        }),
        Expression::PtrLoad { address, .. } => json!({
            "ptr_load": expression_snapshot(address)
        }),
        Expression::Unsafe { block, .. } => json!({
            "unsafe": block_snapshot(block)
        }),
        Expression::Pipeline { left, right, .. } => json!({
            "pipeline": {
                "left": expression_snapshot(left),
                "right": expression_snapshot(right)
            }
        }),
        Expression::ErrorValue { path, .. } => json!({
            "error_value": path
        }),
        Expression::ErrorPropagate { expr, .. } => json!({
            "error_propagate": expression_snapshot(expr)
        }),
    }
}

fn pattern_snapshot(pattern: &MatchPattern) -> Value {
    match pattern {
        MatchPattern::Integer(value) => json!({ "integer": value }),
        MatchPattern::Bool(value) => json!({ "bool": value }),
        MatchPattern::Identifier(name) => json!({ "identifier": name }),
        MatchPattern::String(value) => json!({ "string": value }),
        MatchPattern::List(items) => json!({
            "list": items.iter().map(expression_snapshot).collect::<Vec<_>>()
        }),
        MatchPattern::Constructor { name, fields, .. } => json!({
            "constructor": {
                "name": name,
                "fields": fields.iter().map(pattern_snapshot).collect::<Vec<_>>()
            }
        }),
        MatchPattern::Wildcard(_) => json!({ "wildcard": "_" }),
    }
}

fn binary_op_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "Add",
        BinaryOp::Sub => "Sub",
        BinaryOp::Mul => "Mul",
        BinaryOp::Div => "Div",
        BinaryOp::Mod => "Mod",
        BinaryOp::And => "And",
        BinaryOp::Or => "Or",
        BinaryOp::BitAnd => "BitAnd",
        BinaryOp::BitOr => "BitOr",
        BinaryOp::BitXor => "BitXor",
        BinaryOp::ShiftLeft => "ShiftLeft",
        BinaryOp::ShiftRight => "ShiftRight",
        BinaryOp::Equals => "Equals",
        BinaryOp::Greater => "Greater",
        BinaryOp::GreaterEq => "GreaterEq",
        BinaryOp::Less => "Less",
        BinaryOp::LessEq => "LessEq",
    }
}

fn unary_op_name(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "Neg",
        UnaryOp::Not => "Not",
        UnaryOp::BitNot => "BitNot",
    }
}
