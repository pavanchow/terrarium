//! The tree-walking evaluator, where enforcement actually happens.
//!
//! Every recursive entry point charges fuel and checks the evaluator recursion
//! guard first, so no script, however shaped, can run unbounded work or drive
//! host-stack recursion past the cap. Allocations are charged against the
//! memory budget before they happen. Host effects are reachable only through a
//! granted capability. Arithmetic never panics: overflow wraps, and division or
//! modulo by zero traps.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;

use crate::capability::{is_builtin, Grants, Host};
use crate::limits::Limits;
use crate::parser::{BinOp, Expr, FuncDef, Stmt, UnOp};
use crate::trap::Trap;
use crate::value::Value;

/// Per-frame fixed memory charge, plus per-local slot cost. Rough but
/// conservative: the point is to make deep recursion and wide locals cost
/// something measurable.
const FRAME_BYTES: usize = 128;
const SLOT_BYTES: usize = std::mem::size_of::<Value>();

/// Control-flow result of executing a statement or block.
enum Flow {
    Normal,
    Return(Value),
}

/// What a run consumed, reported regardless of success or trap.
#[derive(Debug, Clone, Copy)]
pub struct Meter {
    pub fuel_used: u64,
    pub mem_used: usize,
}

pub struct Interp<'h> {
    scopes: Vec<HashMap<String, Value>>,
    /// Index in `scopes` where the current function frame begins. Names visible
    /// to a running function are its own frame's scopes plus the global scope.
    frame_bases: Vec<usize>,
    fuel_left: u64,
    fuel_budget: u64,
    mem_used: usize,
    max_memory: usize,
    call_depth: usize,
    max_depth: usize,
    eval_depth: usize,
    max_eval_depth: usize,
    grants: &'h Grants,
    host: &'h mut dyn Host,
    /// Holds the value of the most recent top-level expression statement so
    /// `run_program` can report a program's result.
    last_expr_value: Option<Value>,
}

impl<'h> Interp<'h> {
    pub fn new(limits: &Limits, grants: &'h Grants, host: &'h mut dyn Host) -> Self {
        let mut scopes = Vec::with_capacity(8);
        scopes.push(HashMap::new()); // global scope
        Interp {
            scopes,
            frame_bases: vec![0],
            fuel_left: limits.fuel,
            fuel_budget: limits.fuel,
            mem_used: 0,
            max_memory: limits.max_memory,
            call_depth: 0,
            max_depth: limits.max_depth,
            eval_depth: 0,
            max_eval_depth: limits.max_eval_depth,
            grants,
            host,
            last_expr_value: None,
        }
    }

    pub fn meter(&self) -> Meter {
        Meter {
            fuel_used: self.fuel_budget - self.fuel_left,
            mem_used: self.mem_used,
        }
    }

    /// Bind a global variable before the program runs (used for `args`).
    pub fn set_global(&mut self, name: &str, value: Value) -> Result<(), Trap> {
        self.charge_mem(name.len() + SLOT_BYTES + value.heap_size())?;
        self.scopes[0].insert(name.to_string(), value);
        Ok(())
    }

    /// Run a whole program and return its final value (the value of the last
    /// expression statement, or nil).
    pub fn run_program(&mut self, program: &[Stmt]) -> Result<Value, Trap> {
        let mut last = Value::Nil;
        for stmt in program {
            match self.exec_stmt(stmt)? {
                Flow::Return(v) => return Ok(v),
                Flow::Normal => {
                    if let Stmt::Expr(_) = stmt {
                        last = self.last_expr_value.take().unwrap_or(Value::Nil);
                    }
                }
            }
        }
        Ok(last)
    }

    // --- metering -----------------------------------------------------------

    fn charge_fuel(&mut self, n: u64) -> Result<(), Trap> {
        match self.fuel_left.checked_sub(n) {
            Some(rem) => {
                self.fuel_left = rem;
                Ok(())
            }
            None => {
                self.fuel_left = 0;
                Err(Trap::OutOfFuel)
            }
        }
    }

    fn charge_mem(&mut self, bytes: usize) -> Result<(), Trap> {
        let next = self.mem_used.saturating_add(bytes);
        if next > self.max_memory {
            // Record the attempt so the meter reflects the ceiling was reached.
            self.mem_used = self.max_memory;
            Err(Trap::OutOfMemory)
        } else {
            self.mem_used = next;
            Ok(())
        }
    }

    fn enter_eval(&mut self) -> Result<(), Trap> {
        self.eval_depth += 1;
        if self.eval_depth > self.max_eval_depth {
            return Err(Trap::StackOverflow);
        }
        Ok(())
    }

    fn leave_eval(&mut self) {
        self.eval_depth -= 1;
    }

    // --- scopes -------------------------------------------------------------

    fn current_base(&self) -> usize {
        *self.frame_bases.last().unwrap()
    }

    fn resolve_idx(&self, name: &str) -> Option<usize> {
        let base = self.current_base();
        let mut i = self.scopes.len();
        while i > base {
            i -= 1;
            if self.scopes[i].contains_key(name) {
                return Some(i);
            }
        }
        if base > 0 && self.scopes[0].contains_key(name) {
            return Some(0);
        }
        None
    }

    fn lookup(&self, name: &str) -> Option<Value> {
        self.resolve_idx(name).map(|i| self.scopes[i][name].clone())
    }

    // --- statements ---------------------------------------------------------

    fn exec_block(&mut self, body: &[Stmt]) -> Result<Flow, Trap> {
        self.enter_eval()?;
        self.scopes.push(HashMap::new());
        let mut flow = Flow::Normal;
        let mut result = Ok(());
        for stmt in body {
            match self.exec_stmt(stmt) {
                Ok(Flow::Normal) => {}
                Ok(Flow::Return(v)) => {
                    flow = Flow::Return(v);
                    break;
                }
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.scopes.pop();
        self.leave_eval();
        result.map(|_| flow)
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> Result<Flow, Trap> {
        self.charge_fuel(1)?;
        self.last_expr_value = None;
        match stmt {
            Stmt::Let { name, value } => {
                let v = self.eval(value)?;
                self.charge_mem(name.len() + SLOT_BYTES)?;
                let top = self.scopes.len() - 1;
                self.scopes[top].insert(name.clone(), v);
                Ok(Flow::Normal)
            }
            Stmt::Expr(e) => {
                let v = self.eval(e)?;
                self.last_expr_value = Some(v);
                Ok(Flow::Normal)
            }
            Stmt::Func(def) => {
                self.charge_mem(def.name.len() + SLOT_BYTES)?;
                let top = self.scopes.len() - 1;
                self.scopes[top].insert(def.name.clone(), Value::Func(Arc::clone(def)));
                Ok(Flow::Normal)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                if self.eval_bool(cond)? {
                    self.exec_block(then_block)
                } else if let Some(eb) = else_block {
                    self.exec_block(eb)
                } else {
                    Ok(Flow::Normal)
                }
            }
            Stmt::While { cond, body } => {
                loop {
                    self.charge_fuel(1)?;
                    if !self.eval_bool(cond)? {
                        break;
                    }
                    match self.exec_block(body)? {
                        Flow::Normal => {}
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Return(opt) => {
                let v = match opt {
                    Some(e) => self.eval(e)?,
                    None => Value::Nil,
                };
                Ok(Flow::Return(v))
            }
        }
    }

    // --- expressions --------------------------------------------------------

    fn eval_bool(&mut self, e: &Expr) -> Result<bool, Trap> {
        match self.eval(e)? {
            Value::Bool(b) => Ok(b),
            other => Err(Trap::Type(format!(
                "condition must be bool, got {}",
                other.type_name()
            ))),
        }
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, Trap> {
        self.charge_fuel(1)?;
        self.enter_eval()?;
        let r = self.eval_inner(e);
        self.leave_eval();
        r
    }

    fn eval_inner(&mut self, e: &Expr) -> Result<Value, Trap> {
        match e {
            Expr::Nil => Ok(Value::Nil),
            Expr::Bool(b) => Ok(Value::Bool(*b)),
            Expr::Int(n) => Ok(Value::Int(*n)),
            Expr::Str(s) => {
                self.charge_mem(s.len())?;
                Ok(Value::Str(Arc::new(s.clone())))
            }
            Expr::Ident(name) => self
                .lookup(name)
                .ok_or_else(|| Trap::Name(format!("undefined name '{name}'"))),
            Expr::Array(items) => {
                self.charge_mem(items.len() * SLOT_BYTES)?;
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(self.eval(it)?);
                }
                Ok(Value::Array(Arc::new(out)))
            }
            Expr::Map(entries) => {
                let mut map = BTreeMap::new();
                for (k, ve) in entries {
                    self.charge_mem(k.len() + SLOT_BYTES)?;
                    let v = self.eval(ve)?;
                    map.insert(k.clone(), v);
                }
                Ok(Value::Map(Arc::new(map)))
            }
            Expr::Unary { op, expr } => {
                let v = self.eval(expr)?;
                self.eval_unary(*op, v)
            }
            Expr::Binary { op, lhs, rhs } => {
                let a = self.eval(lhs)?;
                let b = self.eval(rhs)?;
                self.eval_binary(*op, a, b)
            }
            Expr::Logical { and, lhs, rhs } => {
                let a = self.eval_bool(lhs)?;
                if *and {
                    if a {
                        Ok(Value::Bool(self.eval_bool(rhs)?))
                    } else {
                        Ok(Value::Bool(false))
                    }
                } else if a {
                    Ok(Value::Bool(true))
                } else {
                    Ok(Value::Bool(self.eval_bool(rhs)?))
                }
            }
            Expr::Index { target, index } => {
                let t = self.eval(target)?;
                let i = self.eval(index)?;
                self.eval_index(t, i)
            }
            Expr::Call { callee, args } => self.eval_call(callee, args),
            Expr::Assign { target, value } => {
                let v = self.eval(value)?;
                self.assign_to(target, v.clone())?;
                Ok(v)
            }
        }
    }

    fn eval_unary(&mut self, op: UnOp, v: Value) -> Result<Value, Trap> {
        match (op, v) {
            (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(n.wrapping_neg())),
            (UnOp::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
            (UnOp::Neg, other) => Err(Trap::Type(format!("cannot negate {}", other.type_name()))),
            (UnOp::Not, other) => Err(Trap::Type(format!(
                "cannot apply '!' to {}",
                other.type_name()
            ))),
        }
    }

    fn eval_binary(&mut self, op: BinOp, a: Value, b: Value) -> Result<Value, Trap> {
        use BinOp::*;
        match op {
            Add => self.eval_add(a, b),
            Sub => int_arith(a, b, |x, y| Ok(x.wrapping_sub(y)), "-"),
            Mul => int_arith(a, b, |x, y| Ok(x.wrapping_mul(y)), "*"),
            Div => int_arith(
                a,
                b,
                |x, y| {
                    if y == 0 {
                        Err(Trap::DivByZero)
                    } else {
                        Ok(x.wrapping_div(y))
                    }
                },
                "/",
            ),
            Rem => int_arith(
                a,
                b,
                |x, y| {
                    if y == 0 {
                        Err(Trap::DivByZero)
                    } else {
                        Ok(x.wrapping_rem(y))
                    }
                },
                "%",
            ),
            Eq => Ok(Value::Bool(a.value_eq(&b))),
            Ne => Ok(Value::Bool(!a.value_eq(&b))),
            Lt => order(a, b, |o| o == std::cmp::Ordering::Less),
            Gt => order(a, b, |o| o == std::cmp::Ordering::Greater),
            Le => order(a, b, |o| o != std::cmp::Ordering::Greater),
            Ge => order(a, b, |o| o != std::cmp::Ordering::Less),
        }
    }

    fn eval_add(&mut self, a: Value, b: Value) -> Result<Value, Trap> {
        match (a, b) {
            (Value::Int(x), Value::Int(y)) => Ok(Value::Int(x.wrapping_add(y))),
            (Value::Str(x), Value::Str(y)) => {
                // Charge the full result size before building it, so a string
                // that would exceed the budget is never actually allocated.
                let new_len = x.len().saturating_add(y.len());
                self.charge_mem(new_len)?;
                let mut s = String::with_capacity(new_len);
                s.push_str(&x);
                s.push_str(&y);
                Ok(Value::Str(Arc::new(s)))
            }
            (Value::Array(x), Value::Array(y)) => {
                let new_len = x.len().saturating_add(y.len());
                self.charge_mem(new_len * SLOT_BYTES)?;
                let mut out = Vec::with_capacity(new_len);
                out.extend(x.iter().cloned());
                out.extend(y.iter().cloned());
                Ok(Value::Array(Arc::new(out)))
            }
            (a, b) => Err(Trap::Type(format!(
                "cannot add {} and {}",
                a.type_name(),
                b.type_name()
            ))),
        }
    }

    fn eval_index(&mut self, target: Value, index: Value) -> Result<Value, Trap> {
        match (target, index) {
            (Value::Array(a), Value::Int(i)) => {
                let idx = normalize_index(i, a.len())?;
                Ok(a[idx].clone())
            }
            (Value::Str(s), Value::Int(i)) => {
                let chars: Vec<char> = s.chars().collect();
                let idx = normalize_index(i, chars.len())?;
                self.charge_mem(chars[idx].len_utf8())?;
                Ok(Value::Str(Arc::new(chars[idx].to_string())))
            }
            (Value::Map(m), Value::Str(k)) => m
                .get(k.as_str())
                .cloned()
                .ok_or_else(|| Trap::Index(format!("missing key '{k}'"))),
            (t, i) => Err(Trap::Type(format!(
                "cannot index {} with {}",
                t.type_name(),
                i.type_name()
            ))),
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Expr]) -> Result<Value, Trap> {
        // A bare identifier that names a capability builtin and is not shadowed
        // by a bound variable routes through the capability gate.
        if let Expr::Ident(name) = callee {
            if is_builtin(name) && self.resolve_idx(name).is_none() {
                return self.eval_capability(name, args);
            }
        }
        let f = self.eval(callee)?;
        match f {
            Value::Func(def) => {
                let mut argv = Vec::with_capacity(args.len());
                for a in args {
                    argv.push(self.eval(a)?);
                }
                self.call_func(&def, argv)
            }
            other => Err(Trap::Type(format!("{} is not callable", other.type_name()))),
        }
    }

    fn eval_capability(&mut self, name: &str, args: &[Expr]) -> Result<Value, Trap> {
        // Deny before evaluating arguments: an ungranted effect does nothing.
        if !self.grants.is_granted(name) {
            return Err(Trap::CapabilityDenied(name.to_string()));
        }
        self.charge_fuel(1)?;
        let mut argv = Vec::with_capacity(args.len());
        for a in args {
            argv.push(self.eval(a)?);
        }
        let result = self.host.call(name, &argv)?;
        self.charge_mem(result.heap_size())?;
        Ok(result)
    }

    fn call_func(&mut self, def: &Arc<FuncDef>, args: Vec<Value>) -> Result<Value, Trap> {
        if args.len() != def.params.len() {
            return Err(Trap::Arity(format!(
                "function '{}' expects {} argument(s), got {}",
                def.name,
                def.params.len(),
                args.len()
            )));
        }
        self.call_depth += 1;
        if self.call_depth > self.max_depth {
            self.call_depth -= 1;
            return Err(Trap::StackOverflow);
        }
        let outcome = self.run_frame(def, args);
        self.call_depth -= 1;
        outcome
    }

    fn run_frame(&mut self, def: &Arc<FuncDef>, args: Vec<Value>) -> Result<Value, Trap> {
        self.charge_mem(FRAME_BYTES + def.params.len() * SLOT_BYTES)?;
        self.enter_eval()?;
        self.scopes.push(HashMap::new());
        let base = self.scopes.len() - 1;
        self.frame_bases.push(base);
        for (p, a) in def.params.iter().zip(args) {
            self.scopes[base].insert(p.clone(), a);
        }
        let res = self.exec_block(&def.body);
        self.frame_bases.pop();
        self.scopes.pop();
        self.leave_eval();
        match res {
            Ok(Flow::Return(v)) => Ok(v),
            Ok(Flow::Normal) => Ok(Value::Nil),
            Err(e) => Err(e),
        }
    }

    // --- assignment ---------------------------------------------------------

    fn assign_to(&mut self, target: &Expr, val: Value) -> Result<(), Trap> {
        match target {
            Expr::Ident(name) => match self.resolve_idx(name) {
                Some(i) => {
                    self.scopes[i].insert(name.clone(), val);
                    Ok(())
                }
                None => Err(Trap::Name(format!(
                    "cannot assign to undefined name '{name}'"
                ))),
            },
            Expr::Index {
                target: base,
                index,
            } => {
                let idx = self.eval(index)?;
                // Copy-on-write: read the current container, mutate the copy,
                // then write it back up the chain. Works for nested indices.
                let mut container = self.eval(base)?;
                self.set_index(&mut container, idx, val)?;
                self.assign_to(base, container)
            }
            _ => Err(Trap::Type("invalid assignment target".to_string())),
        }
    }

    fn set_index(&mut self, container: &mut Value, index: Value, val: Value) -> Result<(), Trap> {
        match (container, index) {
            (Value::Array(a), Value::Int(i)) => {
                let idx = normalize_index(i, a.len())?;
                let v = Arc::make_mut(a);
                v[idx] = val;
                Ok(())
            }
            (Value::Map(m), Value::Str(k)) => {
                let map = Arc::make_mut(m);
                if !map.contains_key(k.as_str()) {
                    self.charge_mem(k.len() + SLOT_BYTES)?;
                }
                map.insert(k.to_string(), val);
                Ok(())
            }
            (c, i) => Err(Trap::Type(format!(
                "cannot index-assign {} with {}",
                c.type_name(),
                i.type_name()
            ))),
        }
    }
}

fn int_arith<F>(a: Value, b: Value, f: F, sym: &str) -> Result<Value, Trap>
where
    F: Fn(i64, i64) -> Result<i64, Trap>,
{
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok(Value::Int(f(x, y)?)),
        (a, b) => Err(Trap::Type(format!(
            "cannot apply '{}' to {} and {}",
            sym,
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn order<F>(a: Value, b: Value, pick: F) -> Result<Value, Trap>
where
    F: Fn(std::cmp::Ordering) -> bool,
{
    let ord = match (&a, &b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Str(x), Value::Str(y)) => x.cmp(y),
        _ => {
            return Err(Trap::Type(format!(
                "cannot compare {} and {}",
                a.type_name(),
                b.type_name()
            )))
        }
    };
    Ok(Value::Bool(pick(ord)))
}

/// Turn a possibly-negative script index into a bounds-checked position.
/// Negative indices count from the end, Python style.
fn normalize_index(i: i64, len: usize) -> Result<usize, Trap> {
    let idx = if i < 0 { i + len as i64 } else { i };
    if idx < 0 || idx as usize >= len {
        return Err(Trap::Index(format!(
            "index {i} out of range for length {len}"
        )));
    }
    Ok(idx as usize)
}
