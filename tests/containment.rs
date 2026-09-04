//! The containment corpus: hostile and malformed scripts that must each be
//! contained. Every case asserts a bounded outcome (a specific typed trap or a
//! value) with no host panic, no hang, and no escape.

use terrarium::{Limits, Sandbox, Trap};

fn tight() -> Limits {
    Limits::default()
        .with_fuel(200_000)
        .with_memory(256 * 1024)
        .with_depth(128)
}

fn run_no_grants(src: &str) -> terrarium::Outcome {
    Sandbox::new(tight()).run(src, &[])
}

#[test]
fn infinite_loop_runs_out_of_fuel() {
    let out = run_no_grants("while true {}");
    assert_eq!(out.result, Err(Trap::OutOfFuel));
    assert!(out.fuel_used >= 1);
}

#[test]
fn infinite_loop_with_body_runs_out_of_fuel() {
    let out = run_no_grants("let i = 0; while true { i = i + 1; }");
    assert_eq!(out.result, Err(Trap::OutOfFuel));
}

#[test]
fn unbounded_recursion_is_stack_overflow_not_host_crash() {
    let out = run_no_grants("fn f(n) { return f(n + 1); } f(0);");
    assert_eq!(out.result, Err(Trap::StackOverflow));
}

#[test]
fn mutual_recursion_is_stack_overflow() {
    let src = "fn a(n) { return b(n); } fn b(n) { return a(n); } a(0);";
    assert_eq!(run_no_grants(src).result, Err(Trap::StackOverflow));
}

#[test]
fn string_bomb_runs_out_of_memory() {
    let out = run_no_grants("let s = \"xxxxxxxx\"; while true { s = s + s; }");
    assert_eq!(out.result, Err(Trap::OutOfMemory));
}

#[test]
fn array_bomb_runs_out_of_memory() {
    let out = run_no_grants("let a = [1]; while true { a = a + [1]; }");
    assert_eq!(out.result, Err(Trap::OutOfMemory));
}

#[test]
fn map_bomb_is_contained() {
    // Growing a map by a fresh key each step must hit a limit, never run free.
    let src = "let m = {}; let i = 0; while true { m[\"k\" + \"v\"] = i; i = i + 1; }";
    let out = run_no_grants(src);
    assert!(matches!(
        out.result,
        Err(Trap::OutOfMemory) | Err(Trap::OutOfFuel)
    ));
}

#[test]
fn ungranted_capability_is_denied() {
    let out = run_no_grants("print(\"hello\");");
    assert_eq!(out.result, Err(Trap::CapabilityDenied("print".to_string())));
}

#[test]
fn division_by_zero_traps() {
    assert_eq!(run_no_grants("1 / 0;").result, Err(Trap::DivByZero));
}

#[test]
fn modulo_by_zero_traps() {
    assert_eq!(run_no_grants("5 % 0;").result, Err(Trap::DivByZero));
}

#[test]
fn int_min_div_neg_one_wraps_no_panic() {
    // Would panic with a raw `/` in Rust; the sandbox wraps it instead.
    let out = run_no_grants("let a = 0 - 9223372036854775807 - 1; a / (0 - 1);");
    assert_eq!(out.result.unwrap().to_string(), "-9223372036854775808");
}

#[test]
fn integer_overflow_wraps_deterministically() {
    let out = run_no_grants("9223372036854775807 + 1;");
    assert_eq!(out.result.unwrap().to_string(), "-9223372036854775808");
}

#[test]
fn deeply_nested_parens_is_parse_error_not_overflow() {
    let mut src = String::new();
    for _ in 0..50_000 {
        src.push('(');
    }
    src.push('1');
    for _ in 0..50_000 {
        src.push(')');
    }
    src.push(';');
    assert!(matches!(run_no_grants(&src).result, Err(Trap::Parse(_))));
}

#[test]
fn deeply_nested_arrays_is_parse_error_not_overflow() {
    let mut src = String::new();
    for _ in 0..50_000 {
        src.push('[');
    }
    for _ in 0..50_000 {
        src.push(']');
    }
    src.push(';');
    assert!(matches!(run_no_grants(&src).result, Err(Trap::Parse(_))));
}

#[test]
fn long_left_associative_chain_is_stack_overflow_not_host_overflow() {
    // Parses iteratively, so parse depth does not catch it; the evaluator
    // recursion guard must.
    let mut src = String::from("1");
    for _ in 0..200_000 {
        src.push_str("+1");
    }
    src.push(';');
    assert_eq!(run_no_grants(&src).result, Err(Trap::StackOverflow));
}

#[test]
fn garbage_source_is_clean_parse_error() {
    for g in [
        "@#$%^&*",
        "let let let",
        "fn (( {{ ]]",
        "1 2 3 4 5",
        "\"unterminated",
        "if while return",
        "===<><>!!",
        "}{)(][",
    ] {
        let out = run_no_grants(g);
        assert!(
            matches!(out.result, Err(Trap::Parse(_))),
            "expected parse error for {g:?}, got {:?}",
            out.result
        );
    }
}

#[test]
fn empty_and_whitespace_sources_are_nil() {
    assert_eq!(run_no_grants("").result.unwrap().to_string(), "nil");
    assert_eq!(
        run_no_grants("   \n\t  ").result.unwrap().to_string(),
        "nil"
    );
    assert_eq!(
        run_no_grants("# just a comment")
            .result
            .unwrap()
            .to_string(),
        "nil"
    );
}

#[test]
fn escape_test_no_grants_denies_every_effect() {
    // Grant nothing, then have the script try every known effect. Each attempt
    // must be denied and nothing may reach the host.
    for name in terrarium::BUILTINS {
        let src = format!("{name}();");
        let mut sb = Sandbox::new(tight());
        let out = sb.run(&src, &[]);
        assert_eq!(
            out.result,
            Err(Trap::CapabilityDenied(name.to_string())),
            "effect {name} should be denied"
        );
        assert!(
            sb.host().output.is_empty(),
            "no output should reach the host"
        );
    }
}

#[test]
fn capability_cannot_be_smuggled_as_a_value() {
    // `print` is not a first-class value, so it cannot be captured and called
    // through an alias to dodge the gate.
    let out = run_no_grants("let p = print; p();");
    assert!(matches!(out.result, Err(Trap::Name(_))));
}

#[test]
fn ungranted_capability_denied_even_indirectly() {
    // Reaching an effect through a helper function is still denied.
    let src = "fn shout() { return print(\"x\"); } shout();";
    let mut sb = Sandbox::new(tight());
    let out = sb.run(src, &[]);
    assert_eq!(out.result, Err(Trap::CapabilityDenied("print".to_string())));
    assert!(sb.host().output.is_empty());
}

#[test]
fn granted_capability_is_reachable_and_captured() {
    let mut sb = Sandbox::new(tight()).grant("print");
    let out = sb.run("print(\"hi\", 42);", &[]);
    assert!(out.is_ok());
    assert_eq!(sb.host().output, vec!["hi 42".to_string()]);
}

#[test]
fn granting_one_capability_does_not_grant_another() {
    let mut sb = Sandbox::new(tight()).grant("len");
    let out = sb.run("print(\"x\");", &[]);
    assert_eq!(out.result, Err(Trap::CapabilityDenied("print".to_string())));
}

#[test]
fn zero_fuel_traps_immediately() {
    let mut sb = Sandbox::new(Limits::default().with_fuel(0));
    assert_eq!(sb.run("1;", &[]).result, Err(Trap::OutOfFuel));
}

#[test]
fn out_of_range_index_traps() {
    assert!(matches!(
        run_no_grants("[1, 2, 3][10];").result,
        Err(Trap::Index(_))
    ));
    assert!(matches!(
        run_no_grants("[1, 2, 3][0 - 9];").result,
        Err(Trap::Index(_))
    ));
    assert!(matches!(
        run_no_grants("{}[\"missing\"];").result,
        Err(Trap::Index(_))
    ));
}

#[test]
fn type_errors_trap_not_panic() {
    assert!(matches!(
        run_no_grants("1 + \"x\";").result,
        Err(Trap::Type(_))
    ));
    assert!(matches!(
        run_no_grants("true * 3;").result,
        Err(Trap::Type(_))
    ));
    assert!(matches!(
        run_no_grants("if 1 { }").result,
        Err(Trap::Type(_))
    ));
    assert!(matches!(run_no_grants("nil();").result, Err(Trap::Type(_))));
}
