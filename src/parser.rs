//! Recursive-descent parser producing the AST the evaluator walks.
//!
//! The one job that matters for containment here: recursion depth is bounded by
//! `max_parse_depth`, so a source made entirely of nested parentheses or nested
//! blocks yields a `Trap::Parse` instead of overflowing the host stack. Because
//! the AST can be no deeper than that bound, the tree-walking evaluator that
//! mirrors this shape is bounded too.

use crate::error::Pos;
use crate::lexer::{Tok, Token};
use crate::trap::Trap;

/// Expression nodes.
#[derive(Debug, Clone)]
pub enum Expr {
    Nil,
    Bool(bool),
    Int(i64),
    Str(String),
    Ident(String),
    Array(Vec<Expr>),
    Map(Vec<(String, Expr)>),
    Unary {
        op: UnOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// Short-circuiting logical `&&` / `||`.
    Logical {
        and: bool,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// Assignment to a name or an index target. Value of the expression is the
    /// assigned value.
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

/// Statement nodes.
#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        value: Expr,
    },
    Expr(Expr),
    If {
        cond: Expr,
        then_block: Vec<Stmt>,
        else_block: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Func(std::sync::Arc<FuncDef>),
    Return(Option<Expr>),
}

/// A user-defined function definition.
#[derive(Debug, Clone)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
    depth: usize,
    max_depth: usize,
}

/// Parse a full program.
pub fn parse(toks: &[Token], max_depth: usize) -> Result<Vec<Stmt>, Trap> {
    let mut p = Parser {
        toks,
        pos: 0,
        depth: 0,
        max_depth,
    };
    let mut program = Vec::new();
    while !p.check(&Tok::Eof) {
        program.push(p.statement()?);
    }
    Ok(program)
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_pos(&self) -> Pos {
        self.toks[self.pos].pos
    }

    fn advance(&mut self) -> &Token {
        let t = &self.toks[self.pos];
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn check(&self, t: &Tok) -> bool {
        self.peek() == t
    }

    fn eat(&mut self, t: &Tok) -> bool {
        if self.check(t) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: &Tok, what: &str) -> Result<(), Trap> {
        if self.check(t) {
            self.advance();
            Ok(())
        } else {
            Err(Trap::Parse(format!(
                "expected {what} at {}, found {:?}",
                self.peek_pos(),
                self.peek()
            )))
        }
    }

    /// Enter one level of recursion, failing cleanly if too deep. Every
    /// recursive grammar rule calls this first and `leave` on the way out.
    fn enter(&mut self) -> Result<(), Trap> {
        self.depth += 1;
        if self.depth > self.max_depth {
            return Err(Trap::Parse(format!(
                "nesting too deep (limit {}) at {}",
                self.max_depth,
                self.peek_pos()
            )));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn statement(&mut self) -> Result<Stmt, Trap> {
        self.enter()?;
        let r = self.statement_inner();
        self.leave();
        r
    }

    fn statement_inner(&mut self) -> Result<Stmt, Trap> {
        match self.peek() {
            Tok::Let => {
                self.advance();
                let name = self.ident()?;
                self.expect(&Tok::Assign, "'='")?;
                let value = self.expression()?;
                self.expect(&Tok::Semicolon, "';'")?;
                Ok(Stmt::Let { name, value })
            }
            Tok::Fn => {
                self.advance();
                let name = self.ident()?;
                self.expect(&Tok::LParen, "'('")?;
                let mut params = Vec::new();
                if !self.check(&Tok::RParen) {
                    loop {
                        params.push(self.ident()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&Tok::RParen, "')'")?;
                let body = self.block()?;
                Ok(Stmt::Func(std::sync::Arc::new(FuncDef {
                    name,
                    params,
                    body,
                })))
            }
            Tok::If => {
                self.advance();
                let cond = self.expression()?;
                let then_block = self.block()?;
                let else_block = if self.eat(&Tok::Else) {
                    if self.check(&Tok::If) {
                        // `else if` chains as a single-statement block.
                        Some(vec![self.statement()?])
                    } else {
                        Some(self.block()?)
                    }
                } else {
                    None
                };
                Ok(Stmt::If {
                    cond,
                    then_block,
                    else_block,
                })
            }
            Tok::While => {
                self.advance();
                let cond = self.expression()?;
                let body = self.block()?;
                Ok(Stmt::While { cond, body })
            }
            Tok::Return => {
                self.advance();
                let value = if self.check(&Tok::Semicolon) {
                    None
                } else {
                    Some(self.expression()?)
                };
                self.expect(&Tok::Semicolon, "';'")?;
                Ok(Stmt::Return(value))
            }
            _ => {
                let e = self.expression()?;
                self.expect(&Tok::Semicolon, "';'")?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn block(&mut self) -> Result<Vec<Stmt>, Trap> {
        self.enter()?;
        let r = self.block_inner();
        self.leave();
        r
    }

    fn block_inner(&mut self) -> Result<Vec<Stmt>, Trap> {
        self.expect(&Tok::LBrace, "'{'")?;
        let mut body = Vec::new();
        while !self.check(&Tok::RBrace) && !self.check(&Tok::Eof) {
            body.push(self.statement()?);
        }
        self.expect(&Tok::RBrace, "'}'")?;
        Ok(body)
    }

    fn ident(&mut self) -> Result<String, Trap> {
        if let Tok::Ident(name) = self.peek() {
            let name = name.clone();
            self.advance();
            Ok(name)
        } else {
            Err(Trap::Parse(format!(
                "expected identifier at {}, found {:?}",
                self.peek_pos(),
                self.peek()
            )))
        }
    }

    fn expression(&mut self) -> Result<Expr, Trap> {
        self.enter()?;
        let r = self.assignment();
        self.leave();
        r
    }

    fn assignment(&mut self) -> Result<Expr, Trap> {
        let lhs = self.logic_or()?;
        if self.check(&Tok::Assign) {
            self.advance();
            let value = self.assignment()?;
            match &lhs {
                Expr::Ident(_) | Expr::Index { .. } => Ok(Expr::Assign {
                    target: Box::new(lhs),
                    value: Box::new(value),
                }),
                _ => Err(Trap::Parse(format!(
                    "invalid assignment target at {}",
                    self.peek_pos()
                ))),
            }
        } else {
            Ok(lhs)
        }
    }

    fn logic_or(&mut self) -> Result<Expr, Trap> {
        let mut lhs = self.logic_and()?;
        while self.check(&Tok::OrOr) {
            self.advance();
            let rhs = self.logic_and()?;
            lhs = Expr::Logical {
                and: false,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn logic_and(&mut self) -> Result<Expr, Trap> {
        let mut lhs = self.equality()?;
        while self.check(&Tok::AndAnd) {
            self.advance();
            let rhs = self.equality()?;
            lhs = Expr::Logical {
                and: true,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn equality(&mut self) -> Result<Expr, Trap> {
        let mut lhs = self.comparison()?;
        loop {
            let op = match self.peek() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                _ => break,
            };
            self.advance();
            let rhs = self.comparison()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn comparison(&mut self) -> Result<Expr, Trap> {
        let mut lhs = self.term()?;
        loop {
            let op = match self.peek() {
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                Tok::Le => BinOp::Le,
                Tok::Ge => BinOp::Ge,
                _ => break,
            };
            self.advance();
            let rhs = self.term()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn term(&mut self) -> Result<Expr, Trap> {
        let mut lhs = self.factor()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let rhs = self.factor()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn factor(&mut self) -> Result<Expr, Trap> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Rem,
                _ => break,
            };
            self.advance();
            let rhs = self.unary()?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Expr, Trap> {
        match self.peek() {
            Tok::Minus => {
                self.advance();
                self.enter()?;
                let e = self.unary();
                self.leave();
                Ok(Expr::Unary {
                    op: UnOp::Neg,
                    expr: Box::new(e?),
                })
            }
            Tok::Bang => {
                self.advance();
                self.enter()?;
                let e = self.unary();
                self.leave();
                Ok(Expr::Unary {
                    op: UnOp::Not,
                    expr: Box::new(e?),
                })
            }
            _ => self.call(),
        }
    }

    fn call(&mut self) -> Result<Expr, Trap> {
        let mut expr = self.primary()?;
        loop {
            match self.peek() {
                Tok::LParen => {
                    self.advance();
                    let mut args = Vec::new();
                    if !self.check(&Tok::RParen) {
                        loop {
                            args.push(self.expression()?);
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(&Tok::RParen, "')'")?;
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                    };
                }
                Tok::LBracket => {
                    self.advance();
                    let index = self.expression()?;
                    self.expect(&Tok::RBracket, "']'")?;
                    expr = Expr::Index {
                        target: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn primary(&mut self) -> Result<Expr, Trap> {
        self.enter()?;
        let r = self.primary_inner();
        self.leave();
        r
    }

    fn primary_inner(&mut self) -> Result<Expr, Trap> {
        match self.peek().clone() {
            Tok::Int(n) => {
                self.advance();
                Ok(Expr::Int(n))
            }
            Tok::Str(s) => {
                self.advance();
                Ok(Expr::Str(s))
            }
            Tok::True => {
                self.advance();
                Ok(Expr::Bool(true))
            }
            Tok::False => {
                self.advance();
                Ok(Expr::Bool(false))
            }
            Tok::Nil => {
                self.advance();
                Ok(Expr::Nil)
            }
            Tok::Ident(name) => {
                self.advance();
                Ok(Expr::Ident(name))
            }
            Tok::LParen => {
                self.advance();
                let e = self.expression()?;
                self.expect(&Tok::RParen, "')'")?;
                Ok(e)
            }
            Tok::LBracket => {
                self.advance();
                let mut items = Vec::new();
                if !self.check(&Tok::RBracket) {
                    loop {
                        items.push(self.expression()?);
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&Tok::RBracket, "']'")?;
                Ok(Expr::Array(items))
            }
            Tok::LBrace => {
                self.advance();
                let mut entries = Vec::new();
                if !self.check(&Tok::RBrace) {
                    loop {
                        let key = match self.peek().clone() {
                            Tok::Str(s) => {
                                self.advance();
                                s
                            }
                            Tok::Ident(s) => {
                                self.advance();
                                s
                            }
                            _ => {
                                return Err(Trap::Parse(format!(
                                    "expected map key at {}",
                                    self.peek_pos()
                                )))
                            }
                        };
                        self.expect(&Tok::Colon, "':'")?;
                        let val = self.expression()?;
                        entries.push((key, val));
                        if !self.eat(&Tok::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&Tok::RBrace, "'}'")?;
                Ok(Expr::Map(entries))
            }
            other => Err(Trap::Parse(format!(
                "unexpected {:?} at {}",
                other,
                self.peek_pos()
            ))),
        }
    }
}
