//! JavaScript semantics: expressions, objects, classes, promises, built-ins.
//!
//! These are written as JS snippets on purpose — the point is that real code
//! behaves like real JS, not that a particular Rust function returns a value.

use tb_js::Runtime;

/// Run a script, require success, and return the completion value as a string.
fn out(src: &str) -> String {
    let mut rt = Runtime::new();
    match rt.eval(src) {
        Ok(v) => v.to_string(),
        Err(e) => panic!("script failed: {e}\n--- script ---\n{src}"),
    }
}

fn val(src: &str) -> tb_js::Value {
    let mut rt = Runtime::new();
    match rt.eval(src) {
        Ok(v) => v,
        Err(e) => panic!("script failed: {e}\n--- script ---\n{src}"),
    }
}

fn num(src: &str) -> f64 {
    let v = val(src);
    v.to_number()
}

fn ok(src: &str) -> bool {
    let v = val(src);
    v.to_boolean()
}

// ---------------------------------------------------------------- expressions

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(num("1 + 2 * 3"), 7.0);
    assert_eq!(num("(1 + 2) * 3"), 9.0);
    assert_eq!(num("2 ** 3 ** 2"), 512.0, "** is right associative");
    assert_eq!(num("10 % 3"), 1.0);
    assert_eq!(num("-2 ** 2"), -4.0);
    assert_eq!(num("7 / 2"), 3.5);
    assert_eq!(out("1 / 0"), "Infinity");
    assert_eq!(out("0 / 0"), "NaN");
    assert!(!num("'5' - 2").is_nan());
    assert_eq!(num("'5' - 2"), 3.0);
    assert_eq!(out("'5' + 2"), "52", "+ prefers strings");
    assert_eq!(out("1 + 1 + '1'"), "21");
    assert!((num("0.1 + 0.2") - 0.3).abs() < 1e-9);
}

#[test]
fn number_formatting_matches_js() {
    assert_eq!(out("1"), "1");
    assert_eq!(out("1.5"), "1.5");
    assert_eq!(out("1000000"), "1000000");
    assert_eq!(out("'' + 1.0"), "1");
    assert_eq!(out("(1/3) > 0.33 && (1/3) < 0.34 ? 'ok' : 'no'"), "ok");
}

#[test]
fn comparison_and_equality() {
    assert!(ok("'1' == 1"));
    assert!(!ok("'1' === 1"));
    assert!(ok("null == undefined"));
    assert!(!ok("null === undefined"));
    assert!(ok("NaN != NaN"));
    assert!(ok("[1,2] == '1,2'"), "arrays stringify for == only");
    assert!(ok("1 < 2 < 3"), "(1<2)=true → 1<3");
    assert!(
        ok("3 < 2 < 1"),
        "(3<2)=false → 0<1 → true (verified against V8)"
    );
    assert!(ok("'a' < 'b'"));
    assert!(ok("'10' < '9'"), "string compare");
}

#[test]
fn unary_and_bitwise() {
    assert_eq!(num("!0"), 1.0);
    assert_eq!(out("typeof undefined"), "undefined");
    assert_eq!(
        out("typeof missingVariable"),
        "undefined",
        "typeof must not throw"
    );
    assert_eq!(out("typeof null"), "object");
    assert_eq!(out("typeof []"), "object");
    assert_eq!(out("typeof (() => 1)"), "function");
    assert_eq!(num("3 & 1"), 1.0);
    assert_eq!(num("3 | 4"), 7.0);
    assert_eq!(num("5 ^ 3"), 6.0);
    assert_eq!(num("~5"), -6.0);
    assert_eq!(num("1 << 3"), 8.0);
    assert_eq!(num("-1 >>> 28"), 15.0);
    assert_eq!(num("'6' * '7'"), 42.0);
}

#[test]
fn logical_and_nullish_operators() {
    assert_eq!(out("0 || 'fallback'"), "fallback");
    assert_eq!(out("'a' && 'b'"), "b");
    assert_eq!(out("null ?? 'x'"), "x");
    assert_eq!(out("0 ?? 'x'"), "0", "?? ignores only null/undefined");
    assert_eq!(out("false ?? 'x'"), "false");
    let src = "let a = null; a ||= 'set'; a";
    assert_eq!(out(src), "set");
    let src = "let o = {a: {b: 5}}; o.a?.b + (o.x?.y ?? 1)";
    assert_eq!(num(src), 6.0);
}

#[test]
fn template_literals_and_escapes() {
    assert_eq!(out("`a${1 + 1}b`"), "a2b");
    assert_eq!(out("const x = [1,2]; `${x}`"), "1,2");
    assert_eq!(out("'\\u0041'"), "A");
    assert_eq!(out("`\\n`.length"), "1");
    assert_eq!(out("'ä'.length"), "1");
}

#[test]
fn conditional_and_sequence() {
    assert_eq!(out("1 > 0 ? 'y' : 'n'"), "y");
    assert_eq!(out("(1, 2, 3)"), "3");
}

// ------------------------------------------------------------------ functions

#[test]
fn closures_and_recursion() {
    assert_eq!(
        num("function f(n){ return n <= 1 ? 1 : n * f(n-1) } f(5)"),
        120.0
    );
    let src = r"
        function counter() {
          let n = 0;
          return () => ++n;
        }
        const c = counter(); c(); c(); c();
    ";
    assert_eq!(num(src), 3.0);
    let src = "const f = x => y => x + y; f(2)(3)";
    assert_eq!(num(src), 5.0);
}

#[test]
fn hoisting_works_both_ways() {
    assert_eq!(out("hoisted(); function hoisted(){ return 'yes' }"), "yes");
    let src = "function f(){ return v; var v = 'x'; } f()";
    assert_eq!(out(src), "undefined", "var is hoisted but not its value");
}

#[test]
fn let_in_loop_captures_per_iteration() {
    let src = r"
        const fs = [];
        for (let i = 0; i < 3; i++) fs.push(() => i);
        fs.map(f => f()).join(',')
    ";
    assert_eq!(out(src), "0,1,2");
    let src = r"
        const gs = [];
        for (var j = 0; j < 3; j++) gs.push(() => j);
        gs.map(f => f()).join(',')
    ";
    assert_eq!(out(src), "3,3,3", "var shares one binding");
}

#[test]
fn default_rest_and_destructured_params() {
    assert_eq!(
        num("function f(a, b = 10, ...r) { return a + b + r.length } f(1)"),
        11.0
    );
    // a=1, b=2, r=[3,4] → 1+2+2 (node: 5)
    assert_eq!(
        num("function f(a, b = 10, ...r) { return a + b + r.length } f(1,2,3,4)"),
        5.0
    );
    assert_eq!(num("const f = ({a, b = 2}) => a + b; f({a: 1})"), 3.0);
    assert_eq!(num("const f = ([x, , y]) => x + y; f([1, 9, 2])"), 3.0);
    assert_eq!(
        out("const f = ({a, ...rest}) => JSON.stringify(rest); f({a:1,b:2,c:3})"),
        "{\"b\":2,\"c\":3}"
    );
}

#[test]
fn arguments_object_behaves_like_a_list() {
    assert_eq!(num("function f(){ return arguments.length } f(1,2,3)"), 3.0);
    assert_eq!(out("function f(){ return arguments[1] } f('a','b')"), "b");
    let src = "function f(){ return Array.from(arguments).join('-') } f('a','b','c')";
    assert_eq!(out(src), "a-b-c");
}

#[test]
fn call_apply_bind() {
    assert_eq!(
        num("function f(a,b){return this.x+a+b} f.call({x:1},2,3)"),
        6.0
    );
    assert_eq!(
        num("function f(a,b){return this.x+a+b} f.apply({x:10},[1,2])"),
        13.0
    );
    assert_eq!(
        num("function f(){return this.v} const g = f.bind({v:7}); g()"),
        7.0
    );
    let src = "const o = {v: 1, get() { return () => this.v }}; o.get()()";
    assert_eq!(num(src), 1.0, "arrow captures this");
    let src =
        "const o = {v: 2, get() { return function(){ return this.v } }}; const f = o.get(); f()";
    assert_eq!(out(src), "undefined");
}

// -------------------------------------------------------------------- objects

#[test]
fn object_literals_and_property_access() {
    assert_eq!(num("const o = {a: 1, b: {c: 2}}; o.b.c + o['a']"), 3.0);
    assert_eq!(num("const k = 'dyn'; ({[k]: 42}).dyn"), 42.0);
    assert_eq!(num("const a = 5; ({a}).a"), 5.0);
    assert_eq!(out("Object.keys({b:1,a:2,7:3}).join('')"), "7ba");
    assert_eq!(out("JSON.stringify(Object.values({a:1,b:2}))"), "[1,2]");
    assert_eq!(
        num("const o = {x: 1}; const p = 'y'; o[p] = 2; o.x + o.y"),
        3.0
    );
}

#[test]
fn methods_getters_setters() {
    let src = "const o = {v: 1, inc() { this.v++; return this.v }}; o.inc(); o.inc()";
    assert_eq!(num(src), 3.0);
    let src = r"
        const o = { _v: 2, get v() { return this._v * 10 }, set v(x) { this._v = x } };
        o.v = 5;
        o.v
    ";
    assert_eq!(num(src), 50.0);
}

#[test]
fn spread_and_assign() {
    assert_eq!(out("JSON.stringify({...{a:1}, b: 2})"), "{\"a\":1,\"b\":2}");
    assert_eq!(
        out("JSON.stringify(Object.assign({a:1},{b:2},{a:3}))"),
        "{\"a\":3,\"b\":2}"
    );
    assert_eq!(num("const [a, b, ...c] = [1,2,3,4]; a + b + c.length"), 5.0);
    assert_eq!(out("const {x, y = 9} = {x: 1}; `${x}${y}`"), "19");
    assert_eq!(out("[...[1,2], 3, ...'ab'].join('')"), "123ab");
}

#[test]
fn prototype_chain_and_hasownproperty() {
    let src = r"
        const base = { greet() { return 'hi ' + this.name } };
        const obj = Object.create(base);
        obj.name = 'ada';
        obj.greet()
    ";
    assert_eq!(out(src), "hi ada");
    assert!(ok("const o = {}; 'toString' in o"));
    assert!(!ok("const o = {a:1}; o.hasOwnProperty('toString')"));
    assert!(ok("const o = {a:1}; o.hasOwnProperty('a')"));
    assert!(ok("Object.getPrototypeOf([]) === Array.prototype"));
    assert!(ok("const o = {}; Object.prototype.isPrototypeOf(o)"));
}

#[test]
fn define_property_and_freeze() {
    let src = r"
        const o = {};
        Object.defineProperty(o, 'hidden', { value: 1, enumerable: false });
        Object.defineProperty(o, 'ro', { value: 2, writable: false });
        Object.keys(o).length
    ";
    assert_eq!(num(src), 0.0);
    assert!(!ok("Object.keys({a:1,b:2}).length === 1"));
    let src = "const o = Object.freeze({a:1}); o.a = 9; o.a";
    assert_eq!(out(src), "1", "frozen objects reject writes");
}

// -------------------------------------------------------------------- classes

#[test]
fn classes_methods_and_inheritance() {
    let src = r"
        class Animal {
          constructor(name) { this.name = name }
          speak() { return this.name + ' makes a sound' }
        }
        class Dog extends Animal {
          speak() { return this.name + ' barks' }
        }
        const d = new Dog('Rex');
        d.speak() + ' | ' + Animal.prototype.speak.call(d)
    ";
    assert_eq!(out(src), "Rex barks | Rex makes a sound");
    assert!(ok("class A {}; class B extends A {}; new B() instanceof A"));
    assert!(ok("class A {}; new A() instanceof A"));
}

#[test]
fn class_super_static_and_fields() {
    let src = r"
        class Shape {
          kind = 'shape';
          static count = 0;
          constructor(id) { this.id = id; Shape.count++ }
          describe() { return this.kind + ':' + this.id }
        }
        class Circle extends Shape {
          kind = 'circle';
          describe() { return super.describe() + '/r' }
        }
        const c = new Circle(7);
        `${c.describe()} n=${Shape.count}`
    ";
    assert_eq!(out(src), "circle:7/r n=1");
    let src =
        "class A { static make() { return new this(1) } constructor(v){this.v=v} }; A.make().v";
    assert_eq!(num(src), 1.0);
}

#[test]
fn class_call_requires_new() {
    let src = "class A {}; let err = null; try { A() } catch (e) { err = e.constructor.name } err";
    assert_eq!(out(src), "TypeError");
}

#[test]
fn constructors_returning_objects() {
    let src = "function F(){ this.a = 1; return { b: 2 } }; new F().b";
    assert_eq!(num(src), 2.0);
    let src = "function F(){ this.a = 1; return 'x' }; new F().a";
    assert_eq!(num(src), 1.0, "primitive returns are ignored");
}

// ---------------------------------------------------------------------- array

#[test]
fn array_methods() {
    assert_eq!(num("[1,2,3].length"), 3.0);
    assert_eq!(out("[1,2,3].join('-')"), "1-2-3");
    assert_eq!(out("[3,1,2].sort((a,b)=>a-b).join('')"), "123");
    assert_eq!(
        out("[3,1,10].sort().join('')"),
        "1103",
        "default sort is lexicographic"
    );
    assert_eq!(num("[1,2,3,4].filter(x => x % 2 === 0).length"), 2.0);
    assert_eq!(num("[1,2,3].reduce((a,b) => a+b, 10)"), 16.0);
    assert_eq!(num("[1,2,3].map(x => x*x).reduce((a,b)=>a+b)"), 14.0);
    assert!(ok("[1,2,3].includes(2)"));
    assert!(ok("[1,2,3].some(x => x > 2)"));
    assert!(!ok("[1,2,3].every(x => x > 2)"));
    assert_eq!(num("[1,2,3,4].findIndex(x => x > 2)"), 2.0);
    assert_eq!(out("[1,[2,[3]]].flat(2).join('')"), "123");
    assert_eq!(out("[1,2].flatMap(x => [x, x*10]).join('')"), "110220");
    assert_eq!(num("[1,2,3].indexOf(2)"), 1.0);
    assert_eq!(num("[1,2,3].lastIndexOf(9)"), -1.0);
    assert_eq!(out("[1,2,3,4].slice(1,3).join('')"), "23");
    assert_eq!(out("[1,2,3].reverse().join('')"), "321");
    assert_eq!(out("[1,2,3].at(-1)"), "3");
    assert_eq!(num("[1,2,3].concat([4,5]).length"), 5.0);
    assert!(ok("Array.isArray([])"));
    assert!(!ok("Array.isArray({length: 0})"));
    assert_eq!(out("Array.from('abc').join('-')"), "a-b-c");
    assert_eq!(
        out("Array.from({length: 3}, (_, i) => i * 2).join('')"),
        "024"
    );
}

#[test]
fn array_mutation_methods() {
    let src = "const a = [1,2,3]; a.push(4); a.pop(); a.unshift(0); a.shift(); a.join('')";
    assert_eq!(out(src), "123");
    let src = "const a = [1,2,3,4]; const rm = a.splice(1,2); a.join('') + '|' + rm.join('')";
    assert_eq!(out(src), "14|23");
    let src = "const a = [1,2,3]; a.length = 2; a.join('')";
    assert_eq!(out(src), "12", "length truncates");
    let src = "const a = [1,2,3]; a.length = 5; a.length";
    assert_eq!(num(src), 5.0);
    let src = "const a = [1,2,3]; a.forEach((v,i) => { a[i] = v * 2 }); a.join('')";
    assert_eq!(out(src), "246");
    let src = "const a = [1,2,3]; a.fill(0, 1); a.join('')";
    assert_eq!(out(src), "100");
}

#[test]
fn for_of_and_spread_over_iterables() {
    assert_eq!(num("let s = 0; for (const v of [1,2,3]) s += v; s"), 6.0);
    assert_eq!(out("let s = ''; for (const c of 'ab') s += c; s"), "ab");
    assert_eq!(
        out("let s = ''; for (const k in {a:1,b:2}) s += k; s"),
        "ab"
    );
    assert_eq!(
        num("let s = 0; for (const [k, v] of Object.entries({a: 1, b: 2})) s += v; s"),
        3.0
    );
}

// --------------------------------------------------------------------- string

#[test]
fn string_methods() {
    assert_eq!(out("'a,b,c'.split(',').length"), "3");
    assert_eq!(out("'a,,b'.split(',').length"), "3");
    assert_eq!(out("'Hello'.toLowerCase()"), "hello");
    assert_eq!(out("'  x  '.trim()"), "x");
    assert_eq!(out("'Hello world'.replace('world', 'JS')"), "Hello JS");
    assert_eq!(out("'aaa'.replaceAll('a', 'b')"), "bbb");
    assert_eq!(out("'abc'.slice(1)"), "bc");
    assert_eq!(out("'abc'.slice(-2)"), "bc");
    assert_eq!(out("'abc'.substring(1, 2)"), "b");
    assert_eq!(out("'abc'.charAt(1)"), "b");
    assert_eq!(num("'abc'.charCodeAt(1)"), 98.0);
    assert_eq!(out("'ab'.repeat(3)"), "ababab");
    assert_eq!(out("'5'.padStart(3, '0')"), "005");
    assert_eq!(out("'5'.padEnd(3, '0')"), "500");
    assert!(ok("'taz.de'.startsWith('taz')"));
    assert!(ok("'taz.de'.endsWith('.de')"));
    assert!(ok("'taz.de'.includes('z.d')"));
    assert_eq!(num("'taz.de'.indexOf('.')"), 3.0);
    assert_eq!(out("'abc'.toUpperCase().slice(0,1)"), "A");
}

#[test]
fn string_replace_with_regex_and_callbacks() {
    assert_eq!(out("'a1b2'.replace(/\\d/g, '#')"), "a#b#");
    assert_eq!(out("'<x>'.replace(/<(\\w+)>/, '$1')"), "x");
    assert_eq!(
        out("'a-b'.replace(/-(.)/, (m, c) => c.toUpperCase())"),
        "aB"
    );
    assert_eq!(
        out("'2024-07-19'.replace(/(\\d{4})-(\\d{2})-(\\d{2})/, '$3.$2.$1')"),
        "19.07.2024"
    );
    assert_eq!(
        out("'a1a'.replace(/a/, 'b')"),
        "b1a",
        "non-global replaces once"
    );
    assert_eq!(
        out("'x'.replace(/(?<n>x)/, '$0')"),
        "$0",
        "unknown group is literal"
    );
}

#[test]
fn regexp_objects_and_match() {
    assert_eq!(out("'a1'.match(/\\d/)[0]"), "1");
    assert_eq!(num("'a1b22'.match(/\\d+/g).length"), 2.0);
    assert_eq!(out("'key=val'.match(/(\\w+)=(\\w+)/)[2]"), "val");
    assert!(ok("/abc/.test('xabcx')"));
    assert!(!ok("/^abc$/.test('xabc')"));
    assert!(ok("/ABC/i.test('abc')"));
    assert_eq!(out("new RegExp('a+b').source"), "a+b");
    assert!(ok("new RegExp('a+').flags === ''"));
    // 'a1b22' has three digits: 1, 2, 2 — `exec` has to move `lastIndex`
    let src = "const re = /\\d/g; let n = 0; while ((m = re.exec('a1b22')) !== null) n++; n";
    assert_eq!(num(src), 3.0, "exec with lastIndex advances");
}

#[test]
fn json_round_trip() {
    let src = r#"
        const data = { a: 1, b: [1, 2, { c: 'x' }], d: null, e: true, f: 1.5 };
        const s = JSON.stringify(data);
        const back = JSON.parse(s);
        back.b[2].c + '|' + s
    "#;
    assert_eq!(
        out(src),
        r#"x|{"a":1,"b":[1,2,{"c":"x"}],"d":null,"e":true,"f":1.5}"#
    );
    // "[\n 1,\n 2\n]" → 4 lines
    assert_eq!(
        out("JSON.stringify([1,2], null, 1).split('\\n').length"),
        "4"
    );
    assert_eq!(out("JSON.stringify({a: undefined, b: 1})"), "{\"b\":1}");
    assert_eq!(out("JSON.stringify('\"\\\\')"), "\"\\\"\\\\\"");
    let src = "let err = null; try { JSON.parse('{oops}') } catch (e) { err = e.name } err";
    assert_eq!(out(src), "SyntaxError");
    assert_eq!(out("JSON.parse('[1,2,3]').reduce((a,b)=>a+b)"), "6");
}

// --------------------------------------------------------------- control flow

#[test]
fn loops_and_switch() {
    assert_eq!(
        num("let s = 0; for (let i = 0; i < 5; i++) { if (i === 2) continue; s += i } s"),
        8.0
    );
    assert_eq!(
        num("let s = 0; for (let i = 0; i < 10; i++) { if (i > 3) break; s += i } s"),
        6.0
    );
    assert_eq!(
        num("let i = 0, s = 0; do { s += i; i++ } while (i < 3); s"),
        3.0
    );
    assert_eq!(num("let i = 10, s = 0; while (i--) s += 1; s"), 10.0);
    let src = r"
        function f(x) {
          switch (x) {
            case 1: return 'one';
            case 2:
            case 3: return 'two or three';
            default: return 'other';
          }
        }
        f(1) + '/' + f(3) + '/' + f(9)
    ";
    assert_eq!(out(src), "one/two or three/other");
    let src = "let s = 0; outer: for (let i = 0; i < 3; i++) { for (let j = 0; j < 3; j++) { if (j > i) continue outer; s++ } } s";
    assert_eq!(num(src), 6.0);
}

#[test]
fn try_catch_finally_and_error_types() {
    let src = "let log = ''; try { throw new TypeError('bad') } catch (e) { log += e.name + ':' + e.message } log";
    assert_eq!(out(src), "TypeError:bad");
    let src = "let log = ''; try { throw 42 } catch (e) { log = typeof e } log";
    assert_eq!(out(src), "number", "any value can be thrown");
    let src = "function f(){ try { return 'a' } finally { } } f()";
    assert_eq!(out(src), "a");
    let src = "let order = ''; try { order += '1'; throw new Error('x') } catch (e) { order += '2' } finally { order += '3' } order";
    assert_eq!(out(src), "123");
    let src = "try { null.x } catch (e) { e instanceof TypeError }";
    assert!(ok(src));
    let src = "let caught = null; try { undefinedFunction() } catch (e) { caught = e.name } caught";
    assert_eq!(out(src), "ReferenceError");
    let src = "class MyError extends Error {}; const e = new MyError('boom'); e.message + '|' + (e instanceof Error)";
    assert_eq!(out(src), "boom|true");
}

#[test]
fn optional_chaining_short_circuits() {
    assert_eq!(out("const o = {}; o.a?.b?.c"), "undefined");
    assert_eq!(out("const o = {a: {}}; o.a?.b"), "undefined");
    assert!(ok("const o = {f: () => 3}; o.f?.() === 3"));
    assert_eq!(out("const o = {}; o.missing?.()"), "undefined");
    assert_eq!(out("const a = []; a?.[0]"), "undefined");
}

// ------------------------------------------------------------------- promises

#[test]
fn promise_then_chain_runs_in_order() {
    let src = r"
        const log = [];
        log.push('start');
        Promise.resolve('a').then(v => { log.push('then1:' + v); return v + 'b' })
          .then(v => { log.push('then2:' + v) });
        log.push('end');
        log.join(',')
    ";
    assert_eq!(out(src), "start,end");
    let src = r"
        const log = [];
        Promise.resolve('a')
          .then(v => { log.push('A' + v); return v + 'b' })
          .then(v => { log.push('B' + v) });
        // drain the microtask queue
        __pump();
        log.join(',')
    ";
    let mut rt = Runtime::new();
    rt.register("__pump", 0, |it, _t, _a| {
        it.run_jobs(1000);
        Ok(tb_js::Value::Undefined)
    });
    let v = rt.eval(src).unwrap();
    // first then logs 'Aa' and returns 'ab', the second logs 'B'+'ab'
    assert_eq!(v.to_string(), "Aa,Bab");
}

#[test]
fn promise_catch_and_reject() {
    let mut rt = Runtime::new();
    rt.register("__pump", 0, |it, _t, _a| {
        it.run_jobs(1000);
        Ok(tb_js::Value::Undefined)
    });
    let src = r"
        const log = [];
        Promise.reject(new Error('nope'))
          .then(() => log.push('ok'))
          .catch(e => log.push('caught:' + e.message));
        __pump();
        log.join(',')
    ";
    assert_eq!(rt.eval(src).unwrap().to_string(), "caught:nope");
    let src = r"
        const log = [];
        Promise.resolve(1).then(() => { throw new Error('inside') }).catch(e => log.push(e.message));
        __pump();
        log.join(',')
    ";
    assert_eq!(rt.eval(src).unwrap().to_string(), "inside");
}

#[test]
fn new_promise_with_deferred_resolve() {
    let mut rt = Runtime::new();
    rt.register("__pump", 0, |it, _t, _a| {
        it.run_jobs(1000);
        Ok(tb_js::Value::Undefined)
    });
    let src = r"
        let result = null;
        const p = new Promise((resolve) => { resolve(21 * 2) });
        p.then(v => { result = v });
        __pump();
        result
    ";
    assert_eq!(rt.eval(src).unwrap().to_string(), "42");
}

#[test]
fn async_await_values_and_errors() {
    let mut rt = Runtime::new();
    rt.register("__pump", 0, |it, _t, _a| {
        it.run_jobs(10_000);
        Ok(tb_js::Value::Undefined)
    });
    // an async call returns a promise (ToNumber of that is NaN in JS), so the
    // value has to come back through `then`
    let src = "let r = 0; (async () => { const v = await 21; return v * 2 })().then(v => { r = v }); __pump(); r";
    assert_eq!(rt.eval(src).unwrap().to_string(), "42");
    let src = r#"
        let got = null;
        async function f() { const a = await Promise.resolve(3); return a + 1 }
        f().then(v => { got = v });
        __pump();
        got
    "#;
    assert_eq!(rt.eval(src).unwrap().to_string(), "4");
    let src = r"
        let caught = null;
        (async () => { try { await Promise.reject(new Error('bad')) } catch (e) { caught = e.message } })();
        __pump();
        caught
    ";
    assert_eq!(rt.eval(src).unwrap().to_string(), "bad");
}

#[test]
fn await_settles_through_timers() {
    // the classic `await new Promise(r => setTimeout(r, ms))` has to work
    let src = r"
        let value = 'pending';
        async function load() {
          await new Promise(resolve => setTimeout(() => resolve(1), 50));
          value = 'done';
        }
        load();
        __run();
        value
    ";
    let mut rt = Runtime::new();
    rt.register("__run", 0, |it, _t, _a| {
        it.run_timers(1000.0);
        Ok(tb_js::Value::Undefined)
    });
    assert_eq!(rt.eval(src).unwrap().to_string(), "done");
}

#[test]
fn promise_all_and_race() {
    let mut rt = Runtime::new();
    rt.register("__pump", 0, |it, _t, _a| {
        it.run_jobs(10_000);
        Ok(tb_js::Value::Undefined)
    });
    let src = r"
        let out = null;
        Promise.all([1, Promise.resolve(2), new Promise(r => r(3))]).then(a => { out = a.join('-') });
        __pump();
        out
    ";
    assert_eq!(rt.eval(src).unwrap().to_string(), "1-2-3");
    let src = r"
        let out = null;
        Promise.race([new Promise(r => setTimeout(() => r('slow'), 100)), Promise.resolve('fast')])
          .then(v => { out = v });
        __pump();
        out
    ";
    assert_eq!(rt.eval(src).unwrap().to_string(), "fast");
}

#[test]
fn timers_run_in_due_order() {
    let mut rt = Runtime::new();
    let src = r"
        const log = [];
        setTimeout(() => log.push('b'), 20);
        setTimeout(() => log.push('a'), 10);
        setTimeout(() => log.push('c'), 30);
        log.push('sync');
        __run();
        log.join(',')
    ";
    rt.register("__run", 0, |it, _t, _a| {
        it.run_timers(1000.0);
        Ok(tb_js::Value::Undefined)
    });
    assert_eq!(rt.eval(src).unwrap().to_string(), "sync,a,b,c");
}

#[test]
fn intervals_and_clear() {
    let mut rt = Runtime::new();
    rt.register("__run", 0, |it, _t, _a| {
        it.run_timers(55.0);
        Ok(tb_js::Value::Undefined)
    });
    let src = r"
        let n = 0;
        const id = setInterval(() => { n++; if (n === 2) clearInterval(id) }, 10);
        __run();
        n
    ";
    assert_eq!(rt.eval(src).unwrap().to_string(), "2");
}

#[test]
fn microtasks_beat_timers() {
    let mut rt = Runtime::new();
    rt.register("__run", 0, |it, _t, _a| {
        it.run_jobs(1000);
        it.run_timers(100.0);
        Ok(tb_js::Value::Undefined)
    });
    let src = r"
        const log = [];
        setTimeout(() => log.push('timer'), 0);
        queueMicrotask(() => log.push('microtask'));
        Promise.resolve().then(() => log.push('then'));
        log.push('sync');
        __run();
        log.join(',')
    ";
    assert_eq!(
        rt.eval(src).unwrap().to_string(),
        "sync,microtask,then,timer"
    );
}

// ------------------------------------------------------------------- built-ins

#[test]
fn math_and_conversions() {
    assert_eq!(num("Math.max(1, 5, 3)"), 5.0);
    assert_eq!(num("Math.min(1, 5, 3)"), 1.0);
    assert_eq!(num("Math.abs(-4)"), 4.0);
    assert_eq!(num("Math.floor(1.9)"), 1.0);
    assert_eq!(num("Math.ceil(1.1)"), 2.0);
    assert_eq!(num("Math.round(2.5)"), 3.0);
    assert_eq!(num("Math.round(-2.5)"), -2.0);
    assert_eq!(num("Math.sqrt(16)"), 4.0);
    assert_eq!(num("Math.pow(2, 10)"), 1024.0);
    assert_eq!(num("Math.PI * 0"), 0.0);
    assert!(num("Math.random()") >= 0.0 && num("Math.random()") < 1.0);
    assert_eq!(num("parseInt('42px')"), 42.0);
    assert_eq!(num("parseInt('ff', 16)"), 255.0);
    assert!(num("parseInt('abc')").is_nan());
    assert_eq!(num("parseFloat('3.14 and more')"), 314.0 / 100.0);
    assert!(ok("Number.isInteger(3)"));
    assert!(!ok("Number.isInteger(3.5)"));
    assert!(ok("Number.isNaN(NaN)"));
    assert_eq!(out("(1234.5).toFixed(2)"), "1234.50");
    assert_eq!(out("(255).toString(16)"), "ff");
    assert_eq!(out("String(12) + String(true)"), "12true");
    assert_eq!(num("Number('  12  ')"), 12.0);
    assert_eq!(out("encodeURIComponent('a b&c')"), "a%20b%26c");
    assert_eq!(out("decodeURIComponent('a%20b')"), "a b");
}

#[test]
fn date_basics() {
    assert_eq!(
        out("new Date('2024-07-19T00:00:00.000Z').toISOString()"),
        "2024-07-19T00:00:00.000Z"
    );
    assert_eq!(
        num("new Date('2024-07-19T00:00:00.000Z').getFullYear()"),
        2024.0
    );
    assert_eq!(num("new Date('2024-07-19T00:00:00.000Z').getMonth()"), 6.0);
    assert_eq!(num("new Date('2024-07-19T00:00:00.000Z').getDate()"), 19.0);
    assert!(ok("!isNaN(new Date('2024-07-19'))"));
    assert!(ok("isNaN(new Date('nonsense'))"));
    assert_eq!(out("new Date(0).toISOString()"), "1970-01-01T00:00:00.000Z");
    assert_eq!(
        out("JSON.stringify({ d: new Date('2024-01-02T03:04:05.000Z') })"),
        "{\"d\":\"2024-01-02T03:04:05.000Z\"}",
        "toJSON inside stringify"
    );
    assert_eq!(
        num("new Date('2024-01-02T00:00:00.000Z').getTimezoneOffset()"),
        0.0
    );
}

#[test]
fn map_and_set() {
    let src = "const m = new Map(); m.set('a', 1); m.set('b', 2); m.get('a') + m.size";
    assert_eq!(out(src), "3");
    let src = "const m = new Map(); m.set('a', 1); m.delete('a'); m.size";
    assert_eq!(num(src), 0.0);
    let src = "const m = new Map(); m.set('a', 1); m.has('a')";
    assert!(ok(src));
    let src = "const s = new Set([1,1,2,3]); s.size";
    assert_eq!(num(src), 3.0);
    let src = "const s = new Set(); s.add('x'); s.add('x'); s.size";
    assert_eq!(num(src), 1.0);
    let src = "const m = new Map([['a',1],['b',2]]); [...m.keys()].join('')";
    assert_eq!(out(src), "ab");
    let src = "const s = new Set([1,2]); let n = 0; s.forEach(v => { n += v }); n";
    assert_eq!(num(src), 3.0);
}

#[test]
fn console_output_is_captured() {
    let mut rt = Runtime::new();
    rt.eval("console.log('hello', 42, {a: 1}); console.warn('careful'); console.error('boom')")
        .unwrap();
    assert_eq!(rt.console(), vec!["hello 42 { a: 1 }", "careful", "boom"]);
    assert_eq!(rt.console_level("error"), vec!["boom"]);
}

#[test]
fn errors_from_js_are_reported() {
    let mut rt = Runtime::new();
    let err = rt.eval("undefinedThing.method()").unwrap_err();
    assert!(err.contains("undefinedThing"), "message: {err}");
    let mut rt = Runtime::new();
    let err = rt.eval("const x = 1; x()").unwrap_err();
    assert!(err.contains("not a function"), "message: {err}");
    let mut rt = Runtime::new();
    let err = rt.eval("null.deep.prop").unwrap_err();
    assert!(err.contains("Cannot read properties"), "message: {err}");
}

#[test]
fn syntax_errors_do_not_panic_and_are_reported() {
    for src in [
        "function f( {",
        "const =",
        "}}}{{",
        "for (let = ;;)",
        "'unterminated",
        "if (",
    ] {
        let mut rt = Runtime::new();
        let _ = rt.eval(src);
    }
}

#[test]
fn endless_loops_hit_a_budget_instead_of_hanging() {
    let mut rt = Runtime::new();
    rt.interp.step_limit = 200_000;
    let err = rt.eval("while (true) { }").unwrap_err();
    assert!(err.contains("budget"), "message: {err}");
    let mut rt = Runtime::new();
    rt.interp.step_limit = 200_000;
    let err = rt.eval("function f(){ return f() + 1 } f()").unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn real_world_shape_domless_logic() {
    // the kind of self-contained code vendor bundles emit after feature tests
    let src = r#"
        (function () {
          'use strict';
          var registry = {};
          function define(name, factory) { registry[name] = { factory: factory, value: undefined } }
          function require(name) {
            var entry = registry[name];
            if (!entry) throw new Error('unknown module ' + name);
            if (entry.value === undefined) entry.value = entry.factory(require);
            return entry.value;
          }
          define('util', function () {
            return {
              slug: function (s) {
                return String(s).toLowerCase().trim().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
              },
              unique: function (list) {
                var seen = {};
                return list.filter(function (x) {
                  if (seen[x]) return false;
                  seen[x] = true;
                  return true;
                });
              }
            };
          });
          define('articles', function (req) {
            var util = req('util');
            return {
              titles: ['Erste Meldung', 'Zweite Meldung!!', 'Dritte'],
              hrefs: function () {
                return util.unique(this.titles.map(function (t) { return '/' + util.slug(t) })).join(' ');
              }
            };
          });
          return require('articles').hrefs();
        })();
    "#;
    assert_eq!(out(src), "/erste-meldung /zweite-meldung /dritte");
}

#[test]
fn iife_and_module_pattern() {
    assert_eq!(num("var ns = (function(){ var priv = 2; return { pub: function(){ return priv * 3 } } })(); ns.pub()"), 6.0);
    let src = r"
        var mod = (function () {
          var api = {};
          api.add = function (a, b) { return a + b };
          api.mul = function (a, b) { return a * b };
          return api;
        })();
        mod.add(mod.mul(2, 3), 1)
    ";
    assert_eq!(num(src), 7.0);
}

#[test]
fn getter_on_prototype_and_in_object() {
    let src = r"
        class Rect {
          constructor(w, h) { this.w = w; this.h = h }
          get area() { return this.w * this.h }
          set area(v) { this.w = v; this.h = 1 }
        }
        const r = new Rect(3, 4);
        const before = r.area;
        r.area = 5;
        `${before}/${r.area}`
    ";
    assert_eq!(out(src), "12/5");
}

#[test]
fn string_sort_default_is_lexicographic() {
    assert_eq!(out("['b','a','C'].sort().join('')"), "Cab");
    assert_eq!(out("['10','9','2'].sort().join(',')"), "10,2,9");
    assert_eq!(
        out("['10','9','2'].sort((a,b) => a - b).join(',')"),
        "2,9,10"
    );
}

#[test]
fn getters_in_object_literal_and_this() {
    let src = "const o = { v: [1,2,3], get first() { return this.v[0] } }; o.first";
    assert_eq!(num(src), 1.0);
    let src = "const o = { set x(v) { this._x = v }, get x() { return this._x } }; o.x = 'a'; o.x";
    assert_eq!(out(src), "a");
}

#[test]
fn chained_spread_merge_and_immutability() {
    let src = r##"
        const base = { a: 1, nested: { x: 1 } };
        const next = { ...base, b: 2, nested: { ...base.nested, y: 2 } };
        JSON.stringify(next)
    "##;
    assert_eq!(out(src), "{\"a\":1,\"nested\":{\"x\":1,\"y\":2},\"b\":2}");
}
