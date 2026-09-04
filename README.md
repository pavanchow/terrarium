# Terrarium

A from-scratch scripting sandbox that runs untrusted code without letting it hang you, exhaust your memory, blow your stack, or touch anything you did not hand it.

Writing an interpreter is the easy part. Running someone else's script safely is the part everyone skips. Terrarium is built around that hard part. The little language exists to give the sandbox something to contain.

## The gap this fills

Most toy interpreters are one honest infinite loop away from taking down the process that embedded them. Feed them `while true {}` and they spin forever. Feed them a recursive function with no base case and they overflow the host stack into a segfault. Feed them a loop that grows an array and they eat all the memory on the box. Hand them a `print` and they can reach your terminal whether you wanted that or not.

Terrarium treats all four of those as the actual product:

1. Fuel. Every evaluation step costs one unit of fuel. Run out and you get `Trap::OutOfFuel`. An infinite loop stops in bounded time.
2. Memory. Strings, arrays, maps, and call frames are charged against a byte budget before they are allocated. Exceed it and you get `Trap::OutOfMemory`. An allocation bomb stops before it touches real memory.
3. Call depth. Recursion is counted. Too deep and you get `Trap::StackOverflow` rather than a host segfault.
4. Capabilities. A script has no ambient authority. Effects like `print`, `input`, `now`, and `len` are reachable only if the host granted that exact name. Everything else is `Trap::CapabilityDenied`.

## The containment property

This is the one claim the whole project is judged on:

> For ANY input script, including adversarial and malformed ones, `run` returns in bounded time with either a Value or a typed Trap. It never panics the host process, never hangs, never overflows the host stack, and never performs an effect outside the granted capabilities.

Integer overflow wraps and is documented, never a panic. Division or modulo by zero is a trap, never a panic. A garbage source is a clean parse error, never a crash. Deeply nested input is a parse trap or a stack trap, never a real stack overflow.

## How the fuzzer proves it

`tests/fuzz.rs` throws ten thousand programs per run at the sandbox across five fixed seeds. Half are random garbage biased toward the language vocabulary, half are structured random programs that actually reach the evaluator. Every run is executed with no capabilities granted, under tight limits, and the test asserts three invariants on every single iteration:

- host panics equal zero. A panic would surface as an `Internal` trap, and the count must stay at zero.
- hangs equal zero. Each run must complete under a wall clock ceiling.
- escapes equal zero. With nothing granted, nothing may ever reach the host.

A representative run:

```
fuzz report: seeds=5 iterations=10000 host_panics=0 hangs=0 escapes=0 traps=9933 values=67
```

Alongside the fuzzer, `tests/containment.rs` pins each hostile scenario to its exact trap: infinite loop to `OutOfFuel`, unbounded recursion to `StackOverflow`, string and array bombs to `OutOfMemory`, ungranted effects to `CapabilityDenied`, division by zero to `DivByZero`, deep nesting to a parse or stack trap, and garbage to a parse trap. The escape test grants nothing, tries every known effect, and asserts every one is denied with nothing reaching the host.

## The language

Small on purpose. Integers, booleans, strings, nil. Variables with `let` and assignment. Arithmetic, comparison, and short-circuit logic. `if` and `else`, `while` loops, and functions with recursion. Arrays and maps with indexing and index assignment. A handful of builtins that are only reachable through capabilities.

```
fn fib(n) {
  if n < 2 { return n; }
  return fib(n - 1) + fib(n - 2);
}
print(fib(20));
```

## Using it

As a library:

```rust
use terrarium::{Limits, Sandbox, Trap};

let mut sb = Sandbox::new(Limits::default()).grant("print");
let out = sb.run("let x = 2 + 3; print(x); x;", &[]);
assert_eq!(out.result.unwrap().to_string(), "5");
assert_eq!(sb.host().output, vec!["5".to_string()]);

let mut sb = Sandbox::new(Limits::default().with_fuel(10_000));
assert!(matches!(sb.run("while true {}", &[]).result, Err(Trap::OutOfFuel)));
```

As a CLI:

```
terrarium run <file> [--fuel N] [--mem BYTES] [--depth D] [--grant cap,cap] [-- args...]

terrarium run examples/loop.tr --fuel 100000
  trap: OutOfFuel: step budget exhausted  [fuel 100000 / mem 20 bytes]

terrarium run examples/escape.tr
  trap: CapabilityDenied: 'print' was not granted  [fuel 2 / mem 20 bytes]

terrarium run examples/normal.tr --grant print
  6765
  => 6765  [fuel 481603 / mem 6304647 bytes]
```

## Build and test

```
cargo build
cargo test
cargo clippy --all-targets
```

Zero external dependencies. Pure standard library.

## Playground

`docs/index.html` is a self-contained page that ports the sandbox to JavaScript. Type a script, set the fuel, memory, and depth limits, toggle which capabilities are granted, run it, and watch the fuel and memory meters and the trap when a limit is hit. It ships presets for a normal program, an infinite loop, a memory bomb, a deep recursion, and a capability escape.

## Author

Pavan Nallamothu (pavanchow)
