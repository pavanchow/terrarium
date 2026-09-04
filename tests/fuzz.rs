//! The containment fuzzer. It throws large volumes of adversarial and random
//! programs at the sandbox and asserts the containment property holds for every
//! one: the run returns (no hang), it never panics the host into an
//! `Internal` trap, and with no capabilities granted nothing ever reaches the
//! host (no escape).
//!
//! Termination is guaranteed structurally by the fuel budget, the evaluator
//! recursion guard, and the bounded parser, so "no hang" is enforced here by a
//! generous per-run wall-clock ceiling: any run that blew past it would fail.

use std::time::{Duration, Instant};

use terrarium::{Limits, Sandbox, Trap};

/// A tiny deterministic PRNG (xorshift64*). No external crates.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// An alphabet biased toward the language's own vocabulary, so the parser and
/// evaluator are actually exercised rather than just the first-byte error path.
const CHUNKS: &[&str] = &[
    "let ",
    "fn ",
    "if ",
    "else ",
    "while ",
    "return ",
    "true",
    "false",
    "nil",
    "print",
    "input",
    "now",
    "len",
    "(",
    ")",
    "{",
    "}",
    "[",
    "]",
    ";",
    ",",
    ":",
    "=",
    "==",
    "!=",
    "<",
    ">",
    "<=",
    ">=",
    "&&",
    "||",
    "+",
    "-",
    "*",
    "/",
    "%",
    "!",
    "\"",
    "\"x\"",
    " ",
    "\n",
    "0",
    "1",
    "42",
    "999999999999",
    "x",
    "y",
    "foo",
    "@",
    "#",
    "\\",
    "	",
];

fn random_garbage(rng: &mut Rng, chunks: usize) -> String {
    let mut s = String::new();
    for _ in 0..chunks {
        s.push_str(CHUNKS[rng.below(CHUNKS.len())]);
    }
    s
}

/// Build a syntactically plausible expression to a bounded depth, so the
/// evaluator (not just the parser) gets a real workout.
fn random_expr(rng: &mut Rng, depth: usize) -> String {
    if depth == 0 {
        return match rng.below(5) {
            0 => rng.below(1000).to_string(),
            1 => "x".to_string(),
            2 => "true".to_string(),
            3 => "\"s\"".to_string(),
            _ => "[1, 2, 3]".to_string(),
        };
    }
    match rng.below(8) {
        0 => format!(
            "({} + {})",
            random_expr(rng, depth - 1),
            random_expr(rng, depth - 1)
        ),
        1 => format!(
            "({} * {})",
            random_expr(rng, depth - 1),
            random_expr(rng, depth - 1)
        ),
        2 => format!(
            "({} / {})",
            random_expr(rng, depth - 1),
            random_expr(rng, depth - 1)
        ),
        3 => format!(
            "({} < {})",
            random_expr(rng, depth - 1),
            random_expr(rng, depth - 1)
        ),
        4 => format!(
            "({} && {})",
            random_expr(rng, depth - 1),
            random_expr(rng, depth - 1)
        ),
        5 => format!("f({})", random_expr(rng, depth - 1)),
        6 => format!(
            "{}[{}]",
            random_expr(rng, depth - 1),
            random_expr(rng, depth - 1)
        ),
        _ => format!("print({})", random_expr(rng, depth - 1)),
    }
}

fn random_program(rng: &mut Rng) -> String {
    let mut s = String::from("fn f(x) { return x + 1; }\nlet x = 0;\n");
    let stmts = 1 + rng.below(6);
    for _ in 0..stmts {
        match rng.below(4) {
            0 => {
                let d = 2 + rng.below(4);
                s.push_str(&format!("x = {};\n", random_expr(rng, d)));
            }
            1 => s.push_str(&format!("while {} {{ x = x + 1; }}\n", random_expr(rng, 2))),
            2 => s.push_str(&format!(
                "if {} {{ x = 1; }} else {{ x = 2; }}\n",
                random_expr(rng, 2)
            )),
            _ => {
                let d = 2 + rng.below(3);
                s.push_str(&format!("{};\n", random_expr(rng, d)));
            }
        }
    }
    s
}

fn fuzz_limits() -> Limits {
    Limits::default()
        .with_fuel(50_000)
        .with_memory(128 * 1024)
        .with_depth(64)
}

#[test]
fn fuzz_containment_holds() {
    let seeds: [u64; 5] = [1, 7, 42, 1337, 0xDEAD_BEEF];
    let per_seed = 2_000;

    let mut iterations = 0u64;
    let mut host_panics = 0u64;
    let mut hangs = 0u64;
    let mut escapes = 0u64;
    let mut traps = 0u64;
    let mut values = 0u64;
    let ceiling = Duration::from_secs(5);

    for &seed in &seeds {
        let mut rng = Rng::new(seed);
        for i in 0..per_seed {
            // Mix garbage and structured programs.
            let src = if i % 2 == 0 {
                let chunks = 3 + rng.below(60);
                random_garbage(&mut rng, chunks)
            } else {
                random_program(&mut rng)
            };

            // No capabilities granted: any output would be an escape.
            let mut sb = Sandbox::new(fuzz_limits());
            let start = Instant::now();
            let out = sb.run(&src, &[]);
            let elapsed = start.elapsed();

            iterations += 1;

            if elapsed > ceiling {
                hangs += 1;
            }
            if !sb.host().output.is_empty() {
                escapes += 1;
            }
            match &out.result {
                Ok(_) => values += 1,
                Err(Trap::Internal(_)) => host_panics += 1,
                Err(_) => traps += 1,
            }

            assert!(elapsed <= ceiling, "run exceeded time ceiling: {src:?}");
            assert!(
                sb.host().output.is_empty(),
                "capability escape (no grants) for: {src:?}"
            );
            assert!(
                !matches!(out.result, Err(Trap::Internal(_))),
                "host panic surfaced as Internal trap for: {src:?}"
            );
        }
    }

    println!(
        "fuzz report: seeds={} iterations={} host_panics={} hangs={} escapes={} traps={} values={}",
        seeds.len(),
        iterations,
        host_panics,
        hangs,
        escapes,
        traps,
        values
    );

    assert_eq!(host_panics, 0);
    assert_eq!(hangs, 0);
    assert_eq!(escapes, 0);
}

#[test]
fn fuzz_specific_hostile_cases_are_all_typed_traps() {
    // Each of these is a distinct containment scenario; none may panic or hang.
    let cases = [
        "while true {}",
        "fn f() { return f(); } f();",
        "let s = \"x\"; while true { s = s + s; }",
        "let a = [0]; while true { a = a + a; }",
        "1 / 0;",
        "print(\"x\");",
        "nonexistent_variable;",
        "1 + true;",
    ];
    for c in cases {
        let mut sb = Sandbox::new(fuzz_limits());
        let out = sb.run(c, &[]);
        assert!(out.result.is_err(), "expected a trap for {c:?}");
        assert!(
            !matches!(out.result, Err(Trap::Internal(_))),
            "unexpected internal (panic) trap for {c:?}"
        );
        assert!(sb.host().output.is_empty());
    }
}
