# Terrarium design

The design exists to serve one property. Everything below is in service of keeping the host safe from code it did not write.

## The containment property

For ANY input script, `run` returns in bounded time with either a Value or a typed Trap. It never panics the host process, never hangs, never overflows the host stack, and never performs an effect outside the granted capabilities.

The rest of this document walks each half of that sentence and shows what enforces it.

## Pipeline

Source text flows through four stages, all inside a single call to `Sandbox::run`.

1. Lexer (`lexer.rs`). A hand-written scanner, no recursion, turns bytes into tokens. Every malformed byte becomes a parse trap. Because it never recurses, input size alone cannot exhaust the host stack here.
2. Parser (`parser.rs`). A recursive-descent parser builds the AST. Its recursion depth is capped by `max_parse_depth`, so nested parentheses, arrays, or blocks yield a parse trap instead of overflowing the parser.
3. Evaluator (`interp.rs`). A tree-walker executes the AST while charging fuel and memory and counting depth.
4. Sandbox (`sandbox.rs`). Wraps the pipeline, injects `args`, and returns an `Outcome` carrying the result and the fuel and memory report.

## Bounded time

Two guards together bound total work.

Fuel is a step budget. Every statement and every expression charges one unit before it runs, and each loop iteration charges again for its condition test. An infinite loop like `while true {}` burns a unit per iteration and stops at `Trap::OutOfFuel`. Because every recursive entry point charges fuel first, there is no evaluation path that runs without drawing down the budget, so termination does not depend on the shape of the program.

The parser is linear in input length and bounded in depth, so parsing itself always finishes.

## Never overflow the host stack

This is the subtle one. There are two distinct ways a script can drive host recursion.

Function calls. Each script call increments a call-depth counter. Past `max_depth` the evaluator returns `Trap::StackOverflow`. Unbounded recursion lands here.

Expression shape. A left-associative chain like `1+1+1+...` parses iteratively, so `max_parse_depth` never sees it, yet the resulting AST is a deep left spine that the tree-walker descends recursively. A second guard, `max_eval_depth`, counts raw evaluator recursion and traps before the spine can reach the host stack limit.

Those guards decide when to stop. To make stopping safe on any thread, the whole lex, parse, and evaluate pipeline runs on a dedicated thread with a large stack (256 MiB). The evaluator guard trips at a depth whose real host-stack cost is a small fraction of that stack, so the guard always fires first and a genuine overflow never happens. Joining that thread has a second benefit: if anything ever did panic, the panic stops at the join boundary and becomes a typed `Internal` trap rather than a dead process. Stack overflow is not catchable by unwinding, which is exactly why the design prevents it by construction rather than trying to recover from it.

Running on that thread requires the result to cross a thread boundary, so values use `Arc` rather than `Rc` and are therefore `Send`.

## Bounded memory

`max_memory` is a byte budget. The evaluator charges for strings, arrays, maps, map keys, and call frames. The important discipline is charge before allocate: for a string or array concatenation the prospective result size is charged first, so a value that would exceed the budget is never actually built. A doubling loop like `s = s + s` is stopped while the string is still small.

The charge is cumulative and never decremented. That is deliberate and conservative. Cumulative charged bytes are always greater than or equal to live bytes, so capping the cumulative total at N guarantees live host memory from script values stays under N. The tradeoff is that a long-running program that churns temporaries is charged for all of them, but fuel bounds how many steps such a program gets, so the two budgets reinforce each other. Pure integer work allocates nothing and charges nothing.

## No ambient authority

A script cannot reach the outside world on its own. The only doors are the named builtins in `capability.rs`, and each fires only if the host granted that exact name. A call to an ungranted effect returns `Trap::CapabilityDenied` before its arguments are even evaluated, so a denied effect does nothing at all.

Two smuggling routes are closed. A builtin is not a first-class value, so `let p = print;` fails to resolve a name rather than capturing the effect. Wrapping an effect inside a helper function does not help either, since the gate is checked at the call site every time regardless of how the call was reached. Effects flow through a host-provided handler, never straight to stdout, the filesystem, or the clock, which also keeps a run deterministic when the host is deterministic.

## Defined arithmetic

Integer overflow wraps, using the two's complement result, and this is documented rather than left to chance. `i64::MIN / -1`, which would panic under a raw division in Rust, wraps to `i64::MIN`. Division or modulo by zero returns `Trap::DivByZero`. There is no arithmetic path that panics the host.

## Module map

- `trap.rs` the typed failure channel, every way a script can stop.
- `limits.rs` the four dials plus the evaluator recursion guard.
- `value.rs` script values, their sizes for accounting, and structural equality.
- `error.rs` source positions for diagnostics.
- `lexer.rs` bytes to tokens, non-recursive.
- `parser.rs` tokens to a depth-bounded AST.
- `capability.rs` the effect gate and the default host.
- `interp.rs` the evaluator where fuel, memory, depth, and capabilities are enforced.
- `sandbox.rs` the host API, the large-stack thread, and the outcome report.
- `main.rs` the command line.

## What is intentionally left out

The language is deliberately small. No closures over caller locals, no modules, no floats, no exceptions. Adding those would grow the surface the sandbox has to defend without teaching anything new about containment. The point was never the language.

## Author

Pavan Nallamothu (pavanchow)
