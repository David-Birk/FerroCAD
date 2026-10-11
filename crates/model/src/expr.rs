//! Parameter expressions like `width / 2 + 3 mm` or `sqrt(a^2 + b^2)`.
//!
//! Lengths evaluate to millimetres (`mm`, `cm`, `m`, `in` suffixes are
//! converted). Angles evaluate to degrees (`deg` is 1, `rad` converts).
//! Trigonometric functions take degrees, like in Fusion.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum ExprError {
    Syntax(String),
    UnknownName(String),
    UnknownFunction(String),
    Cycle(String),
    NotFinite,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::Syntax(s) => write!(f, "syntax error: {s}"),
            ExprError::UnknownName(n) => write!(f, "unknown parameter '{n}'"),
            ExprError::UnknownFunction(n) => write!(f, "unknown function '{n}'"),
            ExprError::Cycle(n) => write!(f, "parameter '{n}' depends on itself"),
            ExprError::NotFinite => write!(f, "result is not a finite number"),
        }
    }
}

impl std::error::Error for ExprError {}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(char),
}

fn tokenize(s: &str) -> Result<Vec<Tok>, ExprError> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // exponent: 1e-3
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            let v = text
                .parse::<f64>()
                .map_err(|_| ExprError::Syntax(format!("bad number '{text}'")))?;
            out.push(Tok::Num(v));
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
        } else if "+-*/^(),".contains(c) {
            out.push(Tok::Op(c));
            i += 1;
        } else {
            return Err(ExprError::Syntax(format!("unexpected '{c}'")));
        }
    }
    Ok(out)
}

fn unit_factor(name: &str) -> Option<f64> {
    Some(match name {
        "mm" => 1.0,
        "cm" => 10.0,
        "m" => 1000.0,
        "in" => 25.4,
        "deg" => 1.0,
        "rad" => 180.0 / std::f64::consts::PI,
        _ => return None,
    })
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    lookup: &'a mut dyn FnMut(&str) -> Result<f64, ExprError>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn eat_op(&mut self, op: char) -> bool {
        if self.peek() == Some(&Tok::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expr(&mut self) -> Result<f64, ExprError> {
        let mut v = self.term()?;
        loop {
            if self.eat_op('+') {
                v += self.term()?;
            } else if self.eat_op('-') {
                v -= self.term()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn term(&mut self) -> Result<f64, ExprError> {
        let mut v = self.unary()?;
        loop {
            if self.eat_op('*') {
                v *= self.unary()?;
            } else if self.eat_op('/') {
                v /= self.unary()?;
            } else {
                return Ok(v);
            }
        }
    }

    fn unary(&mut self) -> Result<f64, ExprError> {
        if self.eat_op('-') {
            Ok(-self.unary()?)
        } else if self.eat_op('+') {
            self.unary()
        } else {
            self.power()
        }
    }

    fn power(&mut self) -> Result<f64, ExprError> {
        let mut base = self.atom()?;
        // postfix unit: "10 mm", "(a + b) cm", "pi rad"
        if let Some(Tok::Ident(u)) = self.peek().cloned() {
            if let Some(f) = unit_factor(&u) {
                self.pos += 1;
                base *= f;
            }
        }
        if self.eat_op('^') {
            Ok(base.powf(self.unary()?))
        } else {
            Ok(base)
        }
    }

    fn atom(&mut self) -> Result<f64, ExprError> {
        match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.pos += 1;
                Ok(v)
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                if self.eat_op('(') {
                    let mut args = Vec::new();
                    if !self.eat_op(')') {
                        loop {
                            args.push(self.expr()?);
                            if self.eat_op(')') {
                                break;
                            }
                            if !self.eat_op(',') {
                                return Err(ExprError::Syntax("expected ',' or ')'".into()));
                            }
                        }
                    }
                    call(&name, &args)
                } else {
                    (self.lookup)(&name)
                }
            }
            Some(Tok::Op('(')) => {
                self.pos += 1;
                let v = self.expr()?;
                if !self.eat_op(')') {
                    return Err(ExprError::Syntax("missing ')'".into()));
                }
                Ok(v)
            }
            Some(t) => Err(ExprError::Syntax(format!("unexpected {t:?}"))),
            None => Err(ExprError::Syntax("unexpected end".into())),
        }
    }
}

fn call(name: &str, a: &[f64]) -> Result<f64, ExprError> {
    let one = |f: fn(f64) -> f64| -> Result<f64, ExprError> {
        match a {
            [x] => Ok(f(*x)),
            _ => Err(ExprError::Syntax(format!("{name}() takes 1 argument"))),
        }
    };
    let two = |f: fn(f64, f64) -> f64| -> Result<f64, ExprError> {
        match a {
            [x, y] => Ok(f(*x, *y)),
            _ => Err(ExprError::Syntax(format!("{name}() takes 2 arguments"))),
        }
    };
    match name {
        "sqrt" => one(f64::sqrt),
        "abs" => one(f64::abs),
        "round" => one(f64::round),
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "sin" => one(|x| x.to_radians().sin()),
        "cos" => one(|x| x.to_radians().cos()),
        "tan" => one(|x| x.to_radians().tan()),
        "asin" => one(|x| x.asin().to_degrees()),
        "acos" => one(|x| x.acos().to_degrees()),
        "atan" => one(|x| x.atan().to_degrees()),
        "min" => two(f64::min),
        "max" => two(f64::max),
        _ => Err(ExprError::UnknownFunction(name.into())),
    }
}

/// Evaluate `src`, resolving parameter names through `lookup`.
pub fn eval(
    src: &str,
    lookup: &mut dyn FnMut(&str) -> Result<f64, ExprError>,
) -> Result<f64, ExprError> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err(ExprError::Syntax("empty expression".into()));
    }
    let mut p = Parser { toks, pos: 0, lookup };
    let v = p.expr()?;
    if p.pos != p.toks.len() {
        return Err(ExprError::Syntax(format!("unexpected {:?}", p.toks[p.pos])));
    }
    if v.is_finite() {
        Ok(v)
    } else {
        Err(ExprError::NotFinite)
    }
}

/// Evaluate an expression that may not reference any parameter.
pub fn eval_const(src: &str) -> Result<f64, ExprError> {
    eval(src, &mut |n| match n {
        "pi" => Ok(std::f64::consts::PI),
        _ => Err(ExprError::UnknownName(n.into())),
    })
}

/// Names referenced by an expression (not functions or units).
pub fn references(src: &str) -> Vec<String> {
    let Ok(toks) = tokenize(src) else { return vec![] };
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if let Tok::Ident(n) = t {
            let is_call = toks.get(i + 1) == Some(&Tok::Op('('));
            let is_unit = i > 0
                && matches!(&toks[i - 1], Tok::Num(_) | Tok::Ident(_) | Tok::Op(')'))
                && unit_factor(n).is_some();
            if !is_call && !is_unit && !out.contains(n) {
                out.push(n.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(eval_const("1 + 2 * 3").unwrap(), 7.0);
        assert_eq!(eval_const("(1 + 2) * 3").unwrap(), 9.0);
        assert_eq!(eval_const("-2^2").unwrap(), -4.0);
        assert_eq!(eval_const("2^3^2").unwrap(), 512.0);
        assert_eq!(eval_const("10 / 4").unwrap(), 2.5);
        assert_eq!(eval_const("1e-3 * 1000").unwrap(), 1.0);
    }

    #[test]
    fn units_and_functions() {
        assert_eq!(eval_const("2 cm + 5 mm").unwrap(), 25.0);
        assert_eq!(eval_const("1in").unwrap(), 25.4);
        assert!((eval_const("sin(30)").unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(eval_const("max(3, sqrt(16))").unwrap(), 4.0);
        assert!((eval_const("pi rad").unwrap() - 180.0).abs() < 1e-12);
    }

    #[test]
    fn errors() {
        assert!(matches!(eval_const("1 +"), Err(ExprError::Syntax(_))));
        assert!(matches!(eval_const("foo"), Err(ExprError::UnknownName(_))));
        assert!(matches!(eval_const("bar(1)"), Err(ExprError::UnknownFunction(_))));
        assert!(matches!(eval_const("1/0"), Err(ExprError::NotFinite)));
        assert!(matches!(eval_const("1 2"), Err(ExprError::Syntax(_))));
    }

    #[test]
    fn finds_references() {
        assert_eq!(references("width/2 + 3 mm + sqrt(h)"), vec!["width", "h"]);
    }
}
