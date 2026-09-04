//! The host-facing API: build a policy, grant capabilities, run untrusted code.
//!
//! `run` is the load-bearing entry point. Its contract is absolute: for ANY
//! source, including adversarial and malformed input, it returns in bounded
//! time with either a value or a typed trap. It never panics the host process,
//! never hangs, never overflows the host stack, and never performs an effect
//! outside the granted capabilities.
//!
//! Two mechanisms back the "never overflow the host stack" half of that
//! contract together: the evaluator's own recursion guard (`max_eval_depth`)
//! and running the whole lex-parse-eval pipeline on a dedicated thread with a
//! large stack. The guard trips long before the large stack could fill, and
//! joining that thread also turns any stray panic into a typed `Trap` rather
//! than a dead process.

use std::sync::Arc;

use crate::capability::{DefaultHost, Grants, Host};
use crate::interp::Interp;
use crate::lexer::lex;
use crate::limits::Limits;
use crate::parser::parse;
use crate::trap::Trap;
use crate::value::Value;

/// The stack the evaluation thread runs on. Large enough that the evaluator's
/// recursion guard always trips first; only touched pages actually commit.
const EVAL_STACK_BYTES: usize = 256 * 1024 * 1024;

/// The result of a run: the value or trap, plus what it consumed.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// `Ok(value)` on success, `Err(trap)` for every failure mode.
    pub result: Result<Value, Trap>,
    /// Fuel (evaluation steps) consumed.
    pub fuel_used: u64,
    /// Bytes charged against the memory budget.
    pub mem_used: usize,
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }

    pub fn value(&self) -> Option<&Value> {
        self.result.as_ref().ok()
    }

    pub fn trap(&self) -> Option<&Trap> {
        self.result.as_ref().err()
    }
}

/// A configured sandbox: limits, granted capabilities, and a host for effects.
pub struct Sandbox<H: Host = DefaultHost> {
    limits: Limits,
    grants: Grants,
    host: H,
}

impl Sandbox<DefaultHost> {
    /// A sandbox with the given limits and the default in-memory host.
    pub fn new(limits: Limits) -> Self {
        Sandbox {
            limits,
            grants: Grants::new(),
            host: DefaultHost::new(),
        }
    }
}

impl<H: Host> Sandbox<H> {
    /// A sandbox with a custom host implementation.
    pub fn with_host(limits: Limits, host: H) -> Self {
        Sandbox {
            limits,
            grants: Grants::new(),
            host,
        }
    }

    /// Grant a named capability. Builder style.
    pub fn grant(mut self, name: impl Into<String>) -> Self {
        self.grants.grant(name);
        self
    }

    /// Grant several capabilities at once.
    pub fn grant_all<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for n in names {
            self.grants.grant(n);
        }
        self
    }

    /// Borrow the host, e.g. to read captured `print` output after a run.
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Mutably borrow the host, e.g. to queue `input` lines before a run.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Run `source` with `args` exposed to the script as the global `args`
    /// array. Deterministic given a deterministic host.
    pub fn run(&mut self, source: &str, args: &[String]) -> Outcome
    where
        H: Send,
    {
        let limits = self.limits;
        let grants = &self.grants;
        let host = &mut self.host;

        std::thread::scope(|scope| {
            let builder = std::thread::Builder::new()
                .name("terrarium-eval".to_string())
                .stack_size(EVAL_STACK_BYTES);
            let handle = builder
                .spawn_scoped(scope, move || eval_all(source, args, limits, grants, host))
                .expect("failed to spawn evaluation thread");
            match handle.join() {
                Ok(outcome) => outcome,
                // A panic cannot escape the process: it stops at this join and
                // becomes a typed trap.
                Err(_) => Outcome {
                    result: Err(Trap::Internal("evaluation panicked".to_string())),
                    fuel_used: 0,
                    mem_used: 0,
                },
            }
        })
    }
}

/// The full pipeline, run inside the large-stack thread.
fn eval_all<H: Host>(
    source: &str,
    args: &[String],
    limits: Limits,
    grants: &Grants,
    host: &mut H,
) -> Outcome {
    let toks = match lex(source) {
        Ok(t) => t,
        Err(e) => {
            return Outcome {
                result: Err(e),
                fuel_used: 0,
                mem_used: 0,
            }
        }
    };
    let program = match parse(&toks, limits.max_parse_depth) {
        Ok(p) => p,
        Err(e) => {
            return Outcome {
                result: Err(e),
                fuel_used: 0,
                mem_used: 0,
            }
        }
    };

    let mut interp = Interp::new(&limits, grants, host);
    let argv: Vec<Value> = args
        .iter()
        .map(|s| Value::Str(Arc::new(s.clone())))
        .collect();
    if let Err(e) = interp.set_global("args", Value::Array(Arc::new(argv))) {
        let m = interp.meter();
        return Outcome {
            result: Err(e),
            fuel_used: m.fuel_used,
            mem_used: m.mem_used,
        };
    }

    let result = interp.run_program(&program);
    let m = interp.meter();
    Outcome {
        result,
        fuel_used: m.fuel_used,
        mem_used: m.mem_used,
    }
}
