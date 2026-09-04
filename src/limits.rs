//! The four dials that bound every run.
//!
//! A `Limits` is the whole enforcement policy in one place. Defaults are
//! deliberately generous enough for real scripts yet small enough that an
//! adversarial script trips a limit quickly.

/// Resource caps applied to a single `run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Total evaluation steps allowed. Each step costs one fuel. Zero fuel
    /// means the very first step traps, which is a valid (if useless) policy.
    pub fuel: u64,
    /// Maximum bytes the script may charge for its heap values and frames.
    /// This is a conservative cumulative charge, so it is an upper bound on
    /// live host memory used by script values.
    pub max_memory: usize,
    /// Maximum function-call depth before `Trap::StackOverflow`.
    pub max_depth: usize,
    /// Maximum nesting depth the parser will accept before giving up with a
    /// parse error. Bounding this keeps the recursive-descent parser (and the
    /// tree-walking evaluator that mirrors its shape) off the host stack edge.
    pub max_parse_depth: usize,
    /// Maximum evaluator recursion depth. This is a second, independent guard
    /// against host stack exhaustion: left-associative operator chains like
    /// `1+1+1+...` parse iteratively (so `max_parse_depth` does not catch them)
    /// yet evaluate recursively down the left spine. Hitting this yields
    /// `Trap::StackOverflow`. It is not a CLI dial; the default is safe against
    /// the large evaluation stack the sandbox runs on.
    pub max_eval_depth: usize,
}

impl Limits {
    /// A small, safe starting policy suitable for running fully untrusted code.
    pub const DEFAULT_FUEL: u64 = 5_000_000;
    /// 16 MiB of charged allocations.
    pub const DEFAULT_MEMORY: usize = 16 * 1024 * 1024;
    /// Deep enough for real recursion, shallow enough to never reach the host
    /// stack limit (each script frame is a handful of host frames).
    pub const DEFAULT_DEPTH: usize = 512;
    /// Comfortably handles hand-written code; stops pathological nesting.
    pub const DEFAULT_PARSE_DEPTH: usize = 256;
    /// Deep enough for any real expression; far under what the large evaluation
    /// stack (see `sandbox`) can hold, so it always trips before a real
    /// overflow could.
    pub const DEFAULT_EVAL_DEPTH: usize = 10_000;

    /// Builder-style override of the fuel budget.
    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = fuel;
        self
    }

    /// Builder-style override of the memory budget in bytes.
    pub fn with_memory(mut self, bytes: usize) -> Self {
        self.max_memory = bytes;
        self
    }

    /// Builder-style override of the call-depth cap.
    pub fn with_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Builder-style override of the parser nesting cap.
    pub fn with_parse_depth(mut self, depth: usize) -> Self {
        self.max_parse_depth = depth;
        self
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            fuel: Limits::DEFAULT_FUEL,
            max_memory: Limits::DEFAULT_MEMORY,
            max_depth: Limits::DEFAULT_DEPTH,
            max_parse_depth: Limits::DEFAULT_PARSE_DEPTH,
            max_eval_depth: Limits::DEFAULT_EVAL_DEPTH,
        }
    }
}
