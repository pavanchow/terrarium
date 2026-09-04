//! Script values.
//!
//! Values are owned and cloneable. Heap-backed values (strings, arrays, maps,
//! functions) carry a size estimate the evaluator uses for memory accounting.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::parser::FuncDef;

/// A value produced or manipulated by a script.
///
/// Heap-backed variants use `Arc` for cheap clones and so a finished value can
/// cross the large-stack evaluation thread boundary (which requires `Send`).
#[derive(Debug, Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Str(Arc<String>),
    Array(Arc<Vec<Value>>),
    Map(Arc<BTreeMap<String, Value>>),
    /// A user-defined function. Shared so passing it around is cheap.
    Func(Arc<FuncDef>),
}

impl Value {
    /// The type name used in error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Map(_) => "map",
            Value::Func(_) => "function",
        }
    }

    /// A rough number of bytes this value occupies, used for memory accounting.
    /// Nested containers are counted recursively but shallowly enough to stay
    /// cheap; the point is a conservative signal, not exact bookkeeping.
    pub fn heap_size(&self) -> usize {
        match self {
            Value::Nil | Value::Bool(_) | Value::Int(_) => 0,
            Value::Str(s) => s.len(),
            Value::Array(items) => {
                let mut total = items.len() * std::mem::size_of::<Value>();
                for v in items.iter() {
                    total += v.heap_size();
                }
                total
            }
            Value::Map(m) => {
                let mut total = 0;
                for (k, v) in m.iter() {
                    total += k.len() + std::mem::size_of::<Value>() + v.heap_size();
                }
                total
            }
            Value::Func(_) => std::mem::size_of::<Value>(),
        }
    }

    /// Structural equality used by `==` and `!=`.
    pub fn value_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.value_eq(y))
            }
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).is_some_and(|w| v.value_eq(w)))
            }
            // Functions compare by identity of their shared definition.
            (Value::Func(a), Value::Func(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.value_eq(other)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Func(def) => write!(f, "<fn {}>", def.name),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write_repr(f, v)?;
                }
                write!(f, "]")
            }
            Value::Map(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k:?}: ")?;
                    write_repr(f, v)?;
                }
                write!(f, "}}")
            }
        }
    }
}

/// Like `Display`, but strings inside containers are quoted so nested output is
/// unambiguous.
fn write_repr(f: &mut fmt::Formatter<'_>, v: &Value) -> fmt::Result {
    match v {
        Value::Str(s) => write!(f, "{s:?}"),
        other => write!(f, "{other}"),
    }
}
