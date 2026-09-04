//! The typed failure channel of the sandbox.
//!
//! Every way a script can fail, whether it is the script's own fault (a type
//! error) or the sandbox pulling the plug (out of fuel), is one of these
//! variants. A `Trap` is a value, never a panic: the host process is never at
//! risk from anything a script does.

use std::fmt;

/// A recoverable, typed reason a script stopped short of a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trap {
    /// Fuel (the step budget) was exhausted. Bounds CPU and stops loops that
    /// would otherwise never return.
    OutOfFuel,
    /// The memory budget was exhausted by strings, arrays, maps, or frames.
    OutOfMemory,
    /// Call depth exceeded the cap. Deep recursion lands here instead of
    /// overflowing the host stack.
    StackOverflow,
    /// The script named a capability that the host did not grant. This is the
    /// core containment guarantee: no ambient authority.
    CapabilityDenied(String),
    /// Division or modulo by zero.
    DivByZero,
    /// A source that could not be lexed or parsed. Bounded parse depth means
    /// even hostile nesting lands here rather than crashing the parser.
    Parse(String),
    /// An operation applied to the wrong type of value.
    Type(String),
    /// A reference to a name that was never bound.
    Name(String),
    /// A call with the wrong number of arguments.
    Arity(String),
    /// An out-of-range index or a missing map key.
    Index(String),
    /// A `return` used where nothing can catch it (defensive; the evaluator
    /// handles returns internally, so this should not surface normally).
    Internal(String),
}

impl Trap {
    /// A short, stable machine-facing tag for the trap kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Trap::OutOfFuel => "OutOfFuel",
            Trap::OutOfMemory => "OutOfMemory",
            Trap::StackOverflow => "StackOverflow",
            Trap::CapabilityDenied(_) => "CapabilityDenied",
            Trap::DivByZero => "DivByZero",
            Trap::Parse(_) => "Parse",
            Trap::Type(_) => "Type",
            Trap::Name(_) => "Name",
            Trap::Arity(_) => "Arity",
            Trap::Index(_) => "Index",
            Trap::Internal(_) => "Internal",
        }
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trap::OutOfFuel => write!(f, "OutOfFuel: step budget exhausted"),
            Trap::OutOfMemory => write!(f, "OutOfMemory: memory budget exhausted"),
            Trap::StackOverflow => write!(f, "StackOverflow: call depth cap exceeded"),
            Trap::CapabilityDenied(c) => write!(f, "CapabilityDenied: '{c}' was not granted"),
            Trap::DivByZero => write!(f, "DivByZero: division or modulo by zero"),
            Trap::Parse(m) => write!(f, "Parse: {m}"),
            Trap::Type(m) => write!(f, "Type: {m}"),
            Trap::Name(m) => write!(f, "Name: {m}"),
            Trap::Arity(m) => write!(f, "Arity: {m}"),
            Trap::Index(m) => write!(f, "Index: {m}"),
            Trap::Internal(m) => write!(f, "Internal: {m}"),
        }
    }
}

impl std::error::Error for Trap {}
