//! Terrarium: a from-scratch scripting sandbox.
//!
//! Writing the little language is the easy part. The product is the
//! enforcement around it: running untrusted scripts under hard limits on time
//! (fuel), memory, call depth, and authority (capabilities), so that no script,
//! however hostile or malformed, can hang the host, exhaust its memory, blow
//! its stack, or reach any effect it was not explicitly handed.
//!
//! The containment contract lives on [`Sandbox::run`]: for ANY input it returns
//! in bounded time with a [`Value`] or a typed [`Trap`], and never panics the
//! host, never hangs, never overflows the host stack, and never performs an
//! ungranted effect.
//!
//! ```
//! use terrarium::{Limits, Sandbox};
//!
//! let mut sb = Sandbox::new(Limits::default()).grant("print");
//! let out = sb.run("let x = 2 + 3; print(x); x;", &[]);
//! assert_eq!(out.result.unwrap().to_string(), "5");
//! assert_eq!(sb.host().output, vec!["5".to_string()]);
//!
//! // An infinite loop is contained by the fuel budget.
//! let mut sb = Sandbox::new(Limits::default().with_fuel(10_000));
//! let out = sb.run("while true {}", &[]);
//! assert!(matches!(out.result, Err(terrarium::Trap::OutOfFuel)));
//! ```

mod capability;
mod error;
mod interp;
mod lexer;
mod limits;
mod parser;
mod sandbox;
mod trap;
mod value;

pub use capability::{DefaultHost, Grants, Host, BUILTINS};
pub use limits::Limits;
pub use sandbox::{Outcome, Sandbox};
pub use trap::Trap;
pub use value::Value;
