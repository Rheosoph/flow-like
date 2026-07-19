//! Render-level smoke tests: build a small `BoardAst` by hand and assert the FlowScript text.

use flow_like_ast::model::*;
use flow_like_ast::{RenderOptions, render};

fn call(node_type: &str, display: &str, args: Vec<Arg>) -> Call {
    Call {
        node_type: node_type.to_string(),
        display: display.to_string(),
        args,
        anchor: None,
    }
}

#[test]
fn renders_event_with_let_and_named_args() {
    let ast = BoardAst {
        board_id: "b1".to_string(),
        interfaces: vec![],
        variables: vec![VarDecl {
            name: "inputText".to_string(),
            ty: TypeRef::new("string", Container::Normal),
            default: Some(Literal::String("hi".to_string())),
            exposed: true,
            secret: false,
            editable: true,
            runtime_configured: false,
            category: None,
            description: None,
            schema: None,
            anchor: None,
        }],
        functions: vec![],
        events: vec![EventBlock {
            name: "onStart".to_string(),
            node_type: "events_simple_start".to_string(),
            event_name: None,
            params: vec![],
            anchor: None,
            body: Block {
                stmts: vec![
                    Stmt::Let {
                        name: "model".to_string(),
                        anchor: None,
                        call: call(
                            "ai_generative_find_model",
                            "aiGenerativeFindModel",
                            vec![
                                Arg {
                                    name: "provider".to_string(),
                                    value: Expr::Literal(Literal::String("openai".to_string())),
                                },
                                Arg {
                                    name: "model".to_string(),
                                    value: Expr::Literal(Literal::String("gpt-4o".to_string())),
                                },
                            ],
                        ),
                    },
                    Stmt::Call {
                        anchor: None,
                        call: call(
                            "log",
                            "log",
                            vec![Arg {
                                name: "text".to_string(),
                                value: Expr::Field {
                                    base: Box::new(Expr::Ref("model".to_string())),
                                    pin: "name".to_string(),
                                },
                            }],
                        ),
                    },
                ],
            },
        }],
    };

    let text = render(&ast, &RenderOptions::default());
    let expected = "\
let inputText: string = \"hi\"

onStart() {
    const model = aiGenerativeFindModel({ provider: \"openai\", model: \"gpt-4o\" })
    log({ text: model.name })
}
";
    assert_eq!(text, expected);
}

#[test]
fn renders_if_else_branch() {
    let ast = BoardAst {
        board_id: "b2".to_string(),
        interfaces: vec![],
        variables: vec![],
        functions: vec![],
        events: vec![EventBlock {
            name: "onStart".to_string(),
            node_type: "start".to_string(),
            event_name: None,
            params: vec![],
            anchor: None,
            body: Block {
                stmts: vec![Stmt::Branch {
                    bind: None,
                    call: call("control_branch", "controlBranch", vec![]),
                    anchor: None,
                    condition: None,
                    arms: vec![
                        BranchArm {
                            label: "True".to_string(),
                            body: Block {
                                stmts: vec![Stmt::Call {
                                    anchor: None,
                                    call: call("yes", "yes", vec![]),
                                }],
                            },
                        },
                        BranchArm {
                            label: "False".to_string(),
                            body: Block {
                                stmts: vec![Stmt::Call {
                                    anchor: None,
                                    call: call("no", "no", vec![]),
                                }],
                            },
                        },
                    ],
                }],
            },
        }],
    };

    let text = render(&ast, &RenderOptions::default());
    let expected = "\
onStart() {
    if (controlBranch()) { // True
        yes()
    } else { // False
        no()
    }
}
";
    assert_eq!(text, expected);
}

/// Render a single expression by embedding it as the sole named argument of a `probe(...)`
/// call inside an event, then return just the rendered expression text. Keeps per-construct
/// assertions tiny and pinpointed (a failure points at the exact `Expr`/sugar, not a fixture).
fn expr_text(value: Expr) -> String {
    let ast = BoardAst {
        board_id: "t".to_string(),
        interfaces: vec![],
        variables: vec![],
        functions: vec![],
        events: vec![EventBlock {
            name: "onTest".to_string(),
            node_type: "test".to_string(),
            event_name: None,
            params: vec![],
            anchor: None,
            body: Block {
                stmts: vec![Stmt::Call {
                    anchor: None,
                    call: call(
                        "probe",
                        "probe",
                        vec![Arg {
                            name: "value".to_string(),
                            value,
                        }],
                    ),
                }],
            },
        }],
    };
    let text = render(&ast, &RenderOptions::default());
    let line = text
        .lines()
        .find(|l| l.trim_start().starts_with("probe("))
        .expect("probe line present");
    line.trim()
        .strip_prefix("probe({ value: ")
        .and_then(|s| s.strip_suffix(" })"))
        .expect("probe wrapper present")
        .to_string()
}

fn r(name: &str) -> Expr {
    Expr::Ref(name.to_string())
}

#[test]
fn renders_array_literal() {
    assert_eq!(expr_text(Expr::Array(vec![])), "[]");
    assert_eq!(
        expr_text(Expr::Array(vec![r("a"), r("b"), r("c")])),
        "[a, b, c]"
    );
}

#[test]
fn renders_index_access() {
    let base = Expr::Field {
        base: Box::new(r("rows")),
        pin: "values".to_string(),
    };
    assert_eq!(
        expr_text(Expr::Index {
            base: Box::new(base),
            index: Box::new(Expr::Literal(Literal::Int(0))),
        }),
        "rows.values[0]"
    );
}

#[test]
fn renders_member_field_access() {
    let expr = Expr::Member {
        base: Box::new(Expr::Index {
            base: Box::new(r("rows")),
            index: Box::new(Expr::Literal(Literal::Int(0))),
        }),
        field: "report_id".to_string(),
    };
    assert_eq!(expr_text(expr), "rows[0].report_id");
}

#[test]
fn renders_member_bracket_fallback_for_non_ident_key() {
    let expr = Expr::Member {
        base: Box::new(r("row")),
        field: "weird key".to_string(),
    };
    assert_eq!(expr_text(expr), "row[\"weird key\"]");
}

#[test]
fn renders_ternary() {
    assert_eq!(
        expr_text(Expr::Ternary {
            cond: Box::new(r("cond")),
            then: Box::new(r("a")),
            otherwise: Box::new(r("b")),
        }),
        "cond ? a : b"
    );
}

#[test]
fn renders_ternary_parenthesises_binary_condition() {
    let cond = Expr::Binary {
        op: ">".to_string(),
        lhs: Box::new(r("len")),
        rhs: Box::new(Expr::Literal(Literal::Int(10))),
    };
    assert_eq!(
        expr_text(Expr::Ternary {
            cond: Box::new(cond),
            then: Box::new(r("a")),
            otherwise: Box::new(r("b")),
        }),
        "(len > 10) ? a : b"
    );
}

#[test]
fn renders_binary_operator() {
    assert_eq!(
        expr_text(Expr::Binary {
            op: "!=".to_string(),
            lhs: Box::new(r("hash")),
            rhs: Box::new(r("other")),
        }),
        "hash != other"
    );
}

#[test]
fn renders_nested_binary_parenthesised() {
    let inner = Expr::Binary {
        op: "+".to_string(),
        lhs: Box::new(r("a")),
        rhs: Box::new(r("b")),
    };
    assert_eq!(
        expr_text(Expr::Binary {
            op: "*".to_string(),
            lhs: Box::new(inner),
            rhs: Box::new(r("c")),
        }),
        "(a + b) * c"
    );
}

#[test]
fn renders_field_camelcases_pin() {
    let expr = Expr::Field {
        base: Box::new(r("model")),
        pin: "array_out".to_string(),
    };
    assert_eq!(expr_text(expr), "model.arrayOut");
}

#[test]
fn renders_object_literal() {
    assert_eq!(expr_text(Expr::Object(vec![])), "{}");
    let obj = Expr::Object(vec![
        ObjectField {
            key: "title".to_string(),
            value: r("t"),
        },
        ObjectField {
            key: "report id".to_string(),
            value: Expr::Literal(Literal::Int(1)),
        },
    ]);
    assert_eq!(expr_text(obj), "{ title: t, \"report id\": 1 }");
}

#[test]
fn renders_return_statement() {
    let ast = BoardAst {
        board_id: "t".to_string(),
        interfaces: vec![],
        variables: vec![],
        functions: vec![],
        events: vec![EventBlock {
            name: "writeReport".to_string(),
            node_type: "events_generic".to_string(),
            event_name: None,
            params: vec![Param {
                name: "title".to_string(),
                ty: TypeRef::new("string", Container::Normal),
            }],
            anchor: None,
            body: Block {
                stmts: vec![Stmt::Return {
                    values: vec![Expr::Ref("title".to_string())],
                    anchor: None,
                }],
            },
        }],
    };
    let text = render(&ast, &RenderOptions::default());
    let expected = "\
writeReport(title: string) {
    return title
}
";
    assert_eq!(text, expected);
}

#[test]
fn renders_event_with_multiple_params() {
    let ast = BoardAst {
        board_id: "t".to_string(),
        interfaces: vec![],
        variables: vec![],
        functions: vec![],
        events: vec![EventBlock {
            name: "now".to_string(),
            node_type: "events_generic".to_string(),
            event_name: None,
            params: vec![
                Param {
                    name: "date".to_string(),
                    ty: TypeRef::new("Date", Container::Normal),
                },
                Param {
                    name: "items".to_string(),
                    ty: TypeRef::new("Struct", Container::Array),
                },
            ],
            anchor: None,
            body: Block { stmts: vec![] },
        }],
    };
    let text = render(&ast, &RenderOptions::default());
    let expected = "\
now(date: Date, items: Struct[]) {
}
";
    assert_eq!(text, expected);
}
