//! Normal programs must compute correct results and consume metered resources.

use terrarium::{DefaultHost, Limits, Sandbox};

fn eval(src: &str) -> String {
    let mut sb = Sandbox::new(Limits::default());
    let out = sb.run(src, &[]);
    out.result.expect("expected a value").to_string()
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(eval("2 + 3 * 4;"), "14");
    assert_eq!(eval("(2 + 3) * 4;"), "20");
    assert_eq!(eval("17 % 5;"), "2");
    assert_eq!(eval("(0 - 10) / 3;"), "-3");
    assert_eq!(eval("2 * 3 + 4 * 5 - 6;"), "20");
}

#[test]
fn booleans_and_short_circuit() {
    assert_eq!(eval("true && false;"), "false");
    assert_eq!(eval("true || false;"), "true");
    assert_eq!(eval("!(1 == 2);"), "true");
    assert_eq!(eval("1 < 2 && 2 < 3;"), "true");
}

#[test]
fn variables_and_assignment() {
    assert_eq!(eval("let x = 5; x = x + 1; x;"), "6");
    assert_eq!(eval("let a = 1; let b = 2; a + b;"), "3");
}

#[test]
fn if_else_control_flow() {
    assert_eq!(
        eval("let x = 10; if x > 5 { x = 1; } else { x = 2; } x;"),
        "1"
    );
    assert_eq!(
        eval("let x = 2; if x > 5 { x = 1; } else { x = 2; } x;"),
        "2"
    );
    assert_eq!(
        eval("let x = 3; if x == 1 { x = 10; } else if x == 3 { x = 30; } x;"),
        "30"
    );
}

#[test]
fn while_loops_compute() {
    let src = "let i = 0; let sum = 0; while i < 10 { sum = sum + i; i = i + 1; } sum;";
    assert_eq!(eval(src), "45");
}

#[test]
fn recursive_functions() {
    let fib = "fn fib(n) { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); } fib(15);";
    assert_eq!(eval(fib), "610");
    let fact = "fn fact(n) { if n < 2 { return 1; } return n * fact(n - 1); } fact(10);";
    assert_eq!(eval(fact), "3628800");
}

#[test]
fn arrays() {
    assert_eq!(eval("[1, 2, 3][1];"), "2");
    assert_eq!(eval("let a = [1, 2, 3]; a[0] = 99; a[0];"), "99");
    assert_eq!(eval("[1, 2] + [3, 4];"), "[1, 2, 3, 4]");
    assert_eq!(eval("[1, 2, 3][0 - 1];"), "3");
}

#[test]
fn maps() {
    assert_eq!(eval("{\"a\": 1, \"b\": 2}[\"b\"];"), "2");
    assert_eq!(eval("let m = {}; m[\"k\"] = 7; m[\"k\"];"), "7");
    assert_eq!(eval("let m = {x: 1}; m[\"x\"];"), "1");
}

#[test]
fn strings() {
    assert_eq!(eval("\"foo\" + \"bar\";"), "foobar");
    assert_eq!(eval("\"hello\"[0];"), "h");
    assert_eq!(eval("\"ab\" == \"ab\";"), "true");
    assert_eq!(eval("\"a\" < \"b\";"), "true");
}

#[test]
fn nested_data_structures() {
    assert_eq!(eval("let a = [[1, 2], [3, 4]]; a[1][0];"), "3");
    assert_eq!(eval("let a = [[1]]; a[0][0] = 9; a[0][0];"), "9");
}

#[test]
fn print_capability_captures_output() {
    let mut sb = Sandbox::new(Limits::default()).grant("print");
    sb.run("print(\"line one\"); print(\"line\", 2);", &[]);
    assert_eq!(
        sb.host().output,
        vec!["line one".to_string(), "line 2".to_string()]
    );
}

#[test]
fn len_capability() {
    let mut sb = Sandbox::new(Limits::default()).grant("len");
    assert_eq!(
        sb.run("len(\"hello\");", &[]).result.unwrap().to_string(),
        "5"
    );
    assert_eq!(
        sb.run("len([1, 2, 3]);", &[]).result.unwrap().to_string(),
        "3"
    );
}

#[test]
fn input_capability_is_deterministic() {
    let mut sb = Sandbox::new(Limits::default()).grant("input");
    sb.host_mut().push_input("world");
    let out = sb.run("input();", &[]);
    assert_eq!(out.result.unwrap().to_string(), "world");
}

#[test]
fn now_capability_uses_host_clock() {
    let mut host = DefaultHost::new();
    host.clock = 12345;
    let mut sb = Sandbox::with_host(Limits::default(), host).grant("now");
    assert_eq!(sb.run("now();", &[]).result.unwrap().to_string(), "12345");
}

#[test]
fn script_args_are_exposed() {
    let mut sb = Sandbox::new(Limits::default());
    let args = vec!["alpha".to_string(), "beta".to_string()];
    assert_eq!(
        sb.run("args[0];", &args).result.unwrap().to_string(),
        "alpha"
    );
    assert_eq!(
        sb.run("args[1];", &args).result.unwrap().to_string(),
        "beta"
    );
}

#[test]
fn runs_are_deterministic_and_metered() {
    let mut sb = Sandbox::new(Limits::default());
    let a = sb.run(
        "let s = 0; let i = 0; while i < 100 { s = s + i; i = i + 1; } s;",
        &[],
    );
    let mut sb2 = Sandbox::new(Limits::default());
    let b = sb2.run(
        "let s = 0; let i = 0; while i < 100 { s = s + i; i = i + 1; } s;",
        &[],
    );
    assert_eq!(a.result.unwrap().to_string(), "4950");
    assert_eq!(a.fuel_used, b.fuel_used);
    assert!(a.fuel_used > 0);
}
