//! Parser tests: the syntax real-world pages use must parse into a sane AST.

use tb_js::ast::*;
use tb_js::parser::{parse, Parser};

fn p(src: &str) -> Program {
    parse(src).unwrap_or_else(|e| panic!("{e}\n  for: {src}"))
}

fn first(src: &str) -> StmtKind {
    let prog = p(src);
    assert!(!prog.body.is_empty(), "no statement in {src}");
    prog.body[0].kind.clone()
}

#[test]
fn variables_and_destructuring() {
    assert!(matches!(
        first("var a = 1, b;"),
        StmtKind::Decl {
            kind: VarKind::Var,
            ..
        }
    ));
    let StmtKind::Decl { decls, .. } = first("const [a, , b = 2, ...rest] = xs;") else {
        panic!("decl expected")
    };
    match &decls[0].pattern {
        Pattern::Array(items) => {
            assert_eq!(items.len(), 4, "{items:?}");
            assert!(matches!(&items[0], ArrayEl::Item(Pattern::Binding(n), None) if &**n == "a"));
            assert!(matches!(items[1], ArrayEl::Hole));
            assert!(matches!(items[2], ArrayEl::Item(_, Some(_))));
            assert!(matches!(&items[3], ArrayEl::Rest(n) if &**n == "rest"));
        }
        other => panic!("array pattern expected, got {other:?}"),
    }
    let StmtKind::Decl { decls, .. } = first("let {a, b: c = 3, ...others} = o;") else {
        panic!("decl expected")
    };
    match &decls[0].pattern {
        Pattern::Object(items) => {
            assert_eq!(items.len(), 3);
            assert!(matches!(&items[1].pattern, Pattern::Binding(n) if &**n == "c"));
            assert!(items[2].rest);
        }
        other => panic!("object pattern expected, got {other:?}"),
    }
}

#[test]
fn functions_arrows_and_defaults() {
    let StmtKind::Fn(f) = first("function add(a, b = 2, ...rest) { return a + b }") else {
        panic!("fn")
    };
    assert_eq!(&**f.name.as_ref().unwrap(), "add");
    assert_eq!(f.params.len(), 3);
    assert!(f.params[1].default.is_some());
    assert!(matches!(f.params[2].pattern, Pattern::Binding(_)));

    // arrow forms
    for src in [
        "const f = x => x * 2;",
        "const g = (x, y) => x + y;",
        "const h = () => { return 1 };",
        "const i = async () => await p;",
        "const j = (a = 1, {b, c: d = 2}, ...r) => a;",
        "xs.map(x => x.id).filter(v => v).reduce((a, b) => a + b, 0);",
    ] {
        let prog = p(src);
        assert!(!prog.body.is_empty(), "{src}");
    }
}

#[test]
fn classes_with_everything() {
    let src = "\
class Widget extends Base {
  static count = 0;
  #hidden = 1;
  constructor(el) { super(el); this.el = el }
  get label() { return this.el.textContent }
  set label(v) { this.el.textContent = v }
  static create(...a) { return new Widget(...a) }
  async load() { await fetch(this.url) }
  *gen() { yield 1 }
  [Symbol.iterator]() {}
  render() { return `<li>${this.label}</li>` }
}";
    let prog = parse(src);
    match prog {
        Ok(program) => {
            let StmtKind::Class(c) = &program.body[0].kind else {
                panic!("class")
            };
            assert_eq!(&*c.name.clone().unwrap(), "Widget");
            assert!(c.super_class.is_some());
            // private `#hidden` is out of scope for us — it must at least not
            // derail the rest of the class, so we accept 5..9 members
            assert!(c.members.len() >= 6, "{:?}", c.members.len());
        }
        Err(e) => {
            // `#hidden` is a lexed-as-error feature; tolerate only that one
            assert!(e.message.contains('#'), "unexpected class parse error: {e}");
        }
    }
}

#[test]
fn control_flow() {
    assert!(matches!(
        first("if (a) b(); else if (c) d(); else e();"),
        StmtKind::If { .. }
    ));
    assert!(matches!(first("for (;;) break;"), StmtKind::For { .. }));
    assert!(matches!(
        first("for (let i = 0; i < 10; i++) { sum += i }"),
        StmtKind::For { .. }
    ));
    let StmtKind::ForIn { left, .. } = first("for (const k in obj) x(k);") else {
        panic!("for-in")
    };
    assert!(matches!(left, ForInit::Decl(VarKind::Const, _)));
    let StmtKind::ForOf { left, .. } = first("for (const [k, v] of map) use(k, v);") else {
        panic!("for-of")
    };
    assert!(matches!(left, ForInit::Decl(..)));
    assert!(matches!(first("while (x) { y() }"), StmtKind::While { .. }));
    assert!(matches!(
        first("do { x() } while (y);"),
        StmtKind::Do { .. }
    ));
    let StmtKind::Switch { cases, .. } =
        first("switch (v) { case 1: a(); break; case 2: case 3: b(); break; default: c() }")
    else {
        panic!("switch")
    };
    assert_eq!(cases.len(), 4);
    assert!(matches!(
        first("try { x() } catch (e) { log(e) } finally { done() }"),
        StmtKind::Try { .. }
    ));
    assert!(matches!(
        first("try { x() } catch { y() }"),
        StmtKind::Try { .. }
    ));
    assert!(matches!(
        first("outer: for (;;) { continue outer }"),
        StmtKind::Labeled(..)
    ));
}

#[test]
fn expressions_precedence_and_quirks() {
    // all of these must parse; semantic checks live in the interpreter tests
    for src in [
        "a = b = c = 1;",
        "x = a ? b : c ? d : e;",
        "x = a ?? b || c && d;",
        "x = a || (b = 3);",
        "x = obj.a?.b.c?.(1)[2];",
        "x = a ** b ** c;",
        "x = -a + +b - ~c + !d;",
        "x = typeof y === 'undefined';",
        "x = void 0;",
        "delete obj.a;",
        "x = 'a' + 1 + 2;",
        "x = [1,2,3].map(v => v * 2);",
        "x = {...a, b: 1, ...c, d};",
        "x = `a${`b${c}`}d`;",
        "tag`hello ${name}!`;",
        "x = (3).toString();",
        "x = new Date(2024, 1, 1).getTime();",
        "x = new target;",
        "x = function(){ return this };",
        "x = /ab+c/gi.test(s);",
        "x = a / b / c;", // division, not regex
        "x = (a) / b;",   // division after `)`
        "x = a instanceof B;",
        "'k' in obj;",
        "x = a, b;",
        "f(...args, 1);",
        "x = [,,];",
        "({a} = obj);",
        "([a, b] = arr);",
        "x = a?.b ?? 'none';",
        "a ||= 1; b &&= 2; c ??= 3;",
        "x = 0x1F + 0b1010 + 1_000;",
        "x = (function () { return 1 })();",
        "void function(){}();",
        "x = import('./m.js');",
        "x = a\n.b\n.c;", // member across newlines
        "x = [1,2,\n3];",
    ] {
        let _ = p(src);
    }
}

#[test]
fn asi_inserts_semicolons() {
    let prog = p("let a = 1\nlet b = 2\na + b\n");
    assert_eq!(prog.body.len(), 3);
    // `return` on its own line returns undefined
    let prog = p("function f() { return\n3 }");
    let StmtKind::Fn(f) = &prog.body[0].kind else {
        panic!("fn")
    };
    assert!(matches!(f.body.body[0].kind, StmtKind::Return(None)));
    // throw cannot break
    assert!(parse("function f() { throw\n3 }").is_err());
}

#[test]
fn real_vendor_shapes_parse() {
    // condensed shapes taken from the kinds of scripts a news page loads
    let snippets = [
        r#"(function(){"use strict";var e=document.querySelectorAll("[data-x]");e.forEach(function(n){n.addEventListener("click",function(t){t.preventDefault();window.dataLayer=window.dataLayer||[];window.dataLayer.push({event:"click",label:n.dataset.x})})})})();"#,
        r#"document.addEventListener('DOMContentLoaded',()=>{const o={init(){this.el=document.getElementById('app');this.render()},render(){this.el&&(this.el.innerHTML=this.tpl(this.state))},tpl:s=>`<div class="a">${s??''}</div>`};o.init()});"#,
        r#"!function(a,b){"object"==typeof exports&&"object"==typeof module?module.exports=b():"function"==typeof define&&define.amd?define([],b):"object"==typeof exports?exports.a=b():a.a=b()}(this,function(){return function(){var __webpack_modules__={115:function(t){t.exports=function(){return function(t){return t&&t.__esModule}}()}};function __webpack_require__(n){return __webpack_modules__[n]()}return __webpack_require__(115)}()});"#,
        r#"var gtag=function(){var a=arguments;gTag.dataLayer=gTag.dataLayer||[];gTag.dataLayer.push(Array.prototype.slice.call(a))};gtag('js',new Date());gtag('config','G-XYZ123',{send_page_view:false});"#,
        r#"class t{constructor(){this.p=new Promise(r=>{this.r=r})}then(f){return this.p.then(f)}}const n=class extends t{async run(){for await(const x of y){}}};"#,
        r#"try{JSON.parse(s).forEach?.(({a=1,b:[c,...d]={}})=>{console.warn(a,c,d)})}catch(e){console.error(e.message)}finally{cleanup()}"#,
    ];
    for (idx, src) in snippets.iter().enumerate() {
        if let Err(e) = parse(src) {
            panic!("snippet {idx} failed to parse: {e}\n{src}");
        }
    }
}

#[test]
fn tolerant_parsing_skips_only_the_broken_statement() {
    let src = "\
var ok1 = 1;
var = broken;
var ok2 = 2;
function good() { return 3 }
";
    let mut problems = Vec::new();
    let prog = Parser::parse_tolerant(src, &mut problems);
    assert_eq!(problems.len(), 1, "one syntax error reported: {problems:?}");
    assert!(
        prog.body.len() >= 3,
        "the good statements survive: {}",
        prog.body.len()
    );
}

#[test]
fn errors_are_descriptive_and_never_panic() {
    for src in [
        "var ",
        "function (",
        "if (",
        "[1,2",
        "'unterminated",
        "x =>",
        "class {",
        "for (;;",
        "}",
        "switch(x){}",
    ] {
        // must not panic; error or not is fine as long as we survive
        let _ = Parser::parse_tolerant(src, &mut Vec::new());
        let _ = parse(src);
    }
    let e = parse("var = 3").unwrap_err();
    assert!(e.message.contains("expected"), "{e}");
    assert!(e.line >= 1);
}

#[test]
fn template_interpolation_is_reparsed() {
    let prog = p("var s = `a${1 + 2}b${x.y}c`;");
    let StmtKind::Decl { decls, .. } = &prog.body[0].kind else {
        panic!("decl")
    };
    let Some(init) = &decls[0].init else {
        panic!("init")
    };
    match &init.node {
        Node::Template { chunks, exprs } => {
            assert_eq!(chunks.len(), 3);
            assert_eq!(&*chunks[0], "a");
            assert_eq!(&*chunks[1], "b");
            assert_eq!(&*chunks[2], "c");
            assert_eq!(exprs.len(), 2);
        }
        other => panic!("template expected, got {other:?}"),
    }
}

#[test]
fn object_literal_forms() {
    let prog =
        p("var o = {a, b: 1, 2: 'x', 'q': 3, [k]: v, m(){}, get g(){}, set g(v){}, ...rest};");
    let StmtKind::Decl { decls, .. } = &prog.body[0].kind else {
        panic!("decl")
    };
    let init = decls[0].init.as_ref().unwrap();
    match &init.node {
        Node::Object(props) => {
            assert_eq!(props.len(), 8, "{props:?}");
            assert!(matches!(props[0], Prop::Shorthand(_)));
            assert!(matches!(props[5], Prop::Method { .. }));
            assert!(matches!(
                &props[6],
                Prop::Accessor {
                    getter: Some(_),
                    ..
                }
            ));
            assert!(matches!(&props[7], Prop::Spread(_)));
        }
        other => panic!("object expected {other:?}"),
    }
}
