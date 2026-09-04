//! Capabilities: the only door between a script and the outside world.
//!
//! A script has no ambient authority. The named builtins below are the only
//! way to reach a host effect, and each one fires only if the host granted that
//! exact name. Everything else the script can do is confined to its own values.

use std::collections::BTreeSet;

use crate::trap::Trap;
use crate::value::Value;

/// The complete set of effect names a script may ask for. Anything not in this
/// set is not a capability at all and simply does not exist as an effect.
pub const BUILTINS: &[&str] = &["print", "input", "now", "len"];

/// Returns whether `name` is a known capability builtin.
pub fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

/// The host side of an effect. The sandbox calls this only after confirming the
/// capability was granted, so an implementation never has to re-check grants.
pub trait Host {
    /// Perform the named effect. Returning `Err(String)` becomes a
    /// `Trap::Type`-style error surfaced to the script; it must never panic.
    fn call(&mut self, name: &str, args: &[Value]) -> Result<Value, Trap>;
}

/// A default host: `print` appends to an in-memory buffer, `len` measures a
/// value, `now` is a fixed clock (kept deterministic unless overridden), and
/// `input` yields queued lines or empty strings.
#[derive(Debug, Default)]
pub struct DefaultHost {
    /// Everything `print` produced, one entry per call.
    pub output: Vec<String>,
    /// Lines handed back by successive `input()` calls.
    pub inputs: std::collections::VecDeque<String>,
    /// The value `now()` returns. Fixed by default for determinism.
    pub clock: i64,
}

impl DefaultHost {
    pub fn new() -> Self {
        DefaultHost::default()
    }

    /// Queue a line for a future `input()` call.
    pub fn push_input(&mut self, line: impl Into<String>) {
        self.inputs.push_back(line.into());
    }
}

impl Host for DefaultHost {
    fn call(&mut self, name: &str, args: &[Value]) -> Result<Value, Trap> {
        match name {
            "print" => {
                let line = args
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ");
                self.output.push(line);
                Ok(Value::Nil)
            }
            "input" => Ok(Value::Str(std::sync::Arc::new(
                self.inputs.pop_front().unwrap_or_default(),
            ))),
            "now" => Ok(Value::Int(self.clock)),
            "len" => {
                if args.len() != 1 {
                    return Err(Trap::Arity(format!(
                        "len expects 1 argument, got {}",
                        args.len()
                    )));
                }
                let n = match &args[0] {
                    Value::Str(s) => s.chars().count() as i64,
                    Value::Array(a) => a.len() as i64,
                    Value::Map(m) => m.len() as i64,
                    other => {
                        return Err(Trap::Type(format!(
                            "len expects string, array, or map, got {}",
                            other.type_name()
                        )))
                    }
                };
                Ok(Value::Int(n))
            }
            other => Err(Trap::Internal(format!("no host handler for '{other}'"))),
        }
    }
}

/// The set of capability names granted for a run.
#[derive(Debug, Default, Clone)]
pub struct Grants {
    names: BTreeSet<String>,
}

impl Grants {
    pub fn new() -> Self {
        Grants::default()
    }

    pub fn grant(&mut self, name: impl Into<String>) {
        self.names.insert(name.into());
    }

    pub fn is_granted(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}
