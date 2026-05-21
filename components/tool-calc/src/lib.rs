//! calc skill — evaluates a small arithmetic expression.
//!
//! Compiled against the slim `mycelium:skill` world. No NATS imports; pure
//! `tool-provider.invoke(req) -> result<resp, domain-error>`. The
//! mycelium-tool-runner is what receives the NATS request, instantiates
//! this component in a fresh wasmtime store, and publishes the result.
//!
//! Input: `args_json = {"expr": "(17*23)+sqrt(81)"}`
//! Supports: + - * / ( ) numbers, unary minus, sqrt(x), abs(x).

wit_bindgen::generate!({
    path: "wit",
    world: "tool-calc",
    generate_all,
});

use serde_json::{json, Value};

use exports::mycelium::tool::tool_provider::Guest;
use mycelium::types::types::{DomainError, ToolCallRequest, ToolCallResult};

// ── tiny recursive-descent evaluator ─────────────────────────────────────
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self { s: s.as_bytes(), i: 0 }
    }
    fn peek(&self) -> u8 {
        if self.i < self.s.len() { self.s[self.i] } else { 0 }
    }
    fn skip(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn expr(&mut self) -> Result<f64, String> {
        let mut lhs = self.term()?;
        loop {
            self.skip();
            let c = self.peek();
            if c == b'+' || c == b'-' {
                self.i += 1;
                let rhs = self.term()?;
                lhs = if c == b'+' { lhs + rhs } else { lhs - rhs };
            } else {
                break;
            }
        }
        Ok(lhs)
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut lhs = self.factor()?;
        loop {
            self.skip();
            let c = self.peek();
            if c == b'*' || c == b'/' {
                self.i += 1;
                let rhs = self.factor()?;
                if c == b'*' { lhs *= rhs; } else { lhs /= rhs; }
            } else {
                break;
            }
        }
        Ok(lhs)
    }
    fn factor(&mut self) -> Result<f64, String> {
        self.skip();
        let c = self.peek();
        if c == b'-' { self.i += 1; return Ok(-self.factor()?); }
        if c == b'+' { self.i += 1; return self.factor(); }
        if c == b'(' {
            self.i += 1;
            let v = self.expr()?;
            self.skip();
            if self.peek() != b')' { return Err("expected )".into()); }
            self.i += 1;
            return Ok(v);
        }
        if c.is_ascii_alphabetic() {
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
                self.i += 1;
            }
            let name = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
            self.skip();
            if self.peek() != b'(' { return Err(format!("expected ( after {name}")); }
            self.i += 1;
            let arg = self.expr()?;
            self.skip();
            if self.peek() != b')' { return Err("expected )".into()); }
            self.i += 1;
            return match name {
                "sqrt" => Ok(arg.sqrt()),
                "abs"  => Ok(arg.abs()),
                _ => Err(format!("unknown fn {name}")),
            };
        }
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') {
            self.i += 1;
        }
        if start == self.i { return Err(format!("unexpected '{}'", c as char)); }
        std::str::from_utf8(&self.s[start..self.i])
            .map_err(|e| e.to_string())?
            .parse::<f64>()
            .map_err(|e| e.to_string())
    }
}

fn eval(expr: &str) -> Result<f64, String> {
    let mut p = Parser::new(expr);
    let v = p.expr()?;
    p.skip();
    if p.i < p.s.len() {
        return Err(format!("trailing input at pos {}", p.i));
    }
    Ok(v)
}

struct Component;

impl Guest for Component {
    fn invoke(call: ToolCallRequest) -> Result<ToolCallResult, DomainError> {
        let args: Value =
            serde_json::from_str(&call.args_json).unwrap_or_else(|_| json!({}));
        let expr = args.get("expr").and_then(|v| v.as_str()).unwrap_or("");
        let (output_json, is_error) = match eval(expr) {
            Ok(n) => (json!({"value": n}).to_string(), false),
            Err(e) => (json!({"error": e}).to_string(), true),
        };
        Ok(ToolCallResult {
            call_id: call.call_id,
            tool_id: call.tool_id,
            output_json,
            is_error,
        })
    }
}

export!(Component);
