// Run one JS snippet and print the completion value: `cargo run -p tb-js --example run -- 'code'
fn main() {
    let src = std::env::args().nth(1).unwrap_or_else(|| "1+1".into());
    let mut rt = tb_js::Runtime::new();
    rt.register("__pump", 0, |it, _t, _a| {
        it.run_jobs(100_000);
        Ok(tb_js::Value::Undefined)
    });
    rt.register("__run", 0, |it, _t, _a| {
        it.run_timers(10_000.0);
        Ok(tb_js::Value::Undefined)
    });
    match rt.eval(&src) {
        Ok(v) => println!("=> {}", v.inspect()),
        Err(e) => println!("!! {e}"),
    }
    for line in rt.console() {
        println!("console: {line}");
    }
    for p in rt.problems() {
        println!("problem: {p}");
    }
}
