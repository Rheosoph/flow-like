//! The GenICam formula evaluator used by SwissKnife and Converter nodes.
//!
//! Tokenize, shunting-yard into RPN (parsed once per formula), evaluate against a variable slice. Values are
//! dual-typed (int64/double) with C-like promotion: integer ops stay
//! integral, any double operand promotes, trig/log/pow always compute in
//! double.

#[cfg_attr(feature = "valuable", derive(valuable::Valuable))]
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    I(i64),
    F(f64),
}

impl Value {
    pub fn as_i64(self) -> i64 {
        match self {
            Value::I(v) => v,
            Value::F(v) => v.round() as i64,
        }
    }

    pub fn as_f64(self) -> f64 {
        match self {
            Value::I(v) => v as f64,
            Value::F(v) => v,
        }
    }

    fn truthy(self) -> bool {
        match self {
            Value::I(v) => v != 0,
            Value::F(v) => v != 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    ConstI(i64),
    ConstF(f64),
    Var(u16),
    TernaryColon,
    Ternary,
    Or,
    And,
    BitOr,
    BitXor,
    BitAnd,
    Eq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
    Shr,
    Shl,
    Sub,
    Add,
    Rem,
    Div,
    Mul,
    Pow,
    UnaryMinus,
    UnaryPlus,
    BitNot,
    Sin,
    Cos,
    Sgn,
    NegFn,
    Atan,
    Tan,
    Abs,
    Exp,
    Ln,
    Lg,
    Sqrt,
    Trunc,
    Round,
    Floor,
    Ceil,
    Asin,
    Acos,
}

impl Op {
    /// (precedence, right_associative).
    fn binding(self) -> (i32, bool) {
        match self {
            Op::Ternary | Op::TernaryColon => (5, true),
            Op::Or => (10, false),
            Op::And => (20, false),
            Op::BitOr => (40, false),
            Op::BitXor => (50, false),
            Op::BitAnd => (60, false),
            Op::Eq | Op::Ne => (70, false),
            Op::Le | Op::Ge | Op::Lt | Op::Gt => (80, false),
            Op::Shr | Op::Shl => (90, false),
            Op::Sub | Op::Add => (100, false),
            Op::Rem | Op::Div | Op::Mul => (110, false),
            Op::Pow => (120, true),
            Op::UnaryMinus | Op::UnaryPlus | Op::BitNot => (130, true),
            _ => (200, false),
        }
    }
}

/// A compiled formula. `variables` is the name table; `eval` takes values in
/// the same order.
#[derive(Debug, Clone)]
pub struct Expr {
    rpn: Vec<Op>,
    variables: Vec<String>,
}

impl Expr {
    pub fn variables(&self) -> &[String] {
        &self.variables
    }

    /// Compile `src`. Identifiers that aren't functions or the constants
    /// `E`/`PI` become variables.
    pub fn parse(src: &str) -> Result<Self, String> {
        let mut variables: Vec<String> = Vec::new();
        let mut output: Vec<Op> = Vec::new();
        let mut stack: Vec<Option<Op>> = Vec::new(); // None = left parenthesis
        let bytes = src.as_bytes();
        let mut i = 0;
        let mut prev_was_operand = false;

        while i < bytes.len() {
            let c = bytes[i] as char;
            if c.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            if c.is_ascii_digit() || (c == '.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
            {
                let (value, len) = parse_number(&src[i..])?;
                output.push(value);
                i += len;
                prev_was_operand = true;
                continue;
            }
            if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                while i < bytes.len()
                    && ((bytes[i] as char).is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || bytes[i] == b'.')
                {
                    i += 1;
                }
                let word = &src[start..i];
                if let Some(op) = function_named(word) {
                    push_operator(op, &mut stack, &mut output);
                } else {
                    match word {
                        "PI" => output.push(Op::ConstF(std::f64::consts::PI)),
                        "E" => output.push(Op::ConstF(std::f64::consts::E)),
                        name => {
                            let index =
                                variables.iter().position(|v| v == name).unwrap_or_else(|| {
                                    variables.push(name.to_string());
                                    variables.len() - 1
                                });
                            output.push(Op::Var(index as u16));
                        }
                    }
                    prev_was_operand = true;
                }
                continue;
            }
            if c == '(' {
                stack.push(None);
                prev_was_operand = false;
                i += 1;
                continue;
            }
            if c == ')' {
                loop {
                    match stack.pop() {
                        Some(Some(op)) => output.push(op),
                        Some(None) => break,
                        None => return Err("unbalanced parenthesis".into()),
                    }
                }
                prev_was_operand = true;
                i += 1;
                continue;
            }

            let two = src.get(i..i + 2).unwrap_or("");
            let (op, len) = match two {
                "||" => (Op::Or, 2),
                "&&" => (Op::And, 2),
                "<>" => (Op::Ne, 2),
                "<=" => (Op::Le, 2),
                ">=" => (Op::Ge, 2),
                "<<" => (Op::Shl, 2),
                ">>" => (Op::Shr, 2),
                "**" => (Op::Pow, 2),
                _ => match c {
                    '?' => (Op::Ternary, 1),
                    ':' => (Op::TernaryColon, 1),
                    '|' => (Op::BitOr, 1),
                    '^' => (Op::BitXor, 1),
                    '&' => (Op::BitAnd, 1),
                    '=' => (Op::Eq, 1),
                    '<' => (Op::Lt, 1),
                    '>' => (Op::Gt, 1),
                    '+' if !prev_was_operand => (Op::UnaryPlus, 1),
                    '-' if !prev_was_operand => (Op::UnaryMinus, 1),
                    '+' => (Op::Add, 1),
                    '-' => (Op::Sub, 1),
                    '%' => (Op::Rem, 1),
                    '/' => (Op::Div, 1),
                    '*' => (Op::Mul, 1),
                    '~' => (Op::BitNot, 1),
                    other => return Err(format!("unexpected character '{other}'")),
                },
            };
            push_operator(op, &mut stack, &mut output);
            prev_was_operand = false;
            i += len;
        }

        while let Some(entry) = stack.pop() {
            match entry {
                Some(op) => output.push(op),
                None => return Err("unbalanced parenthesis".into()),
            }
        }
        Ok(Self {
            rpn: output,
            variables,
        })
    }

    /// Evaluate with `vars` matching [`variables`](Self::variables) by index.
    pub fn eval(&self, vars: &[Value]) -> Result<Value, String> {
        let mut stack: Vec<Value> = Vec::with_capacity(8);
        for &op in &self.rpn {
            match op {
                Op::ConstI(v) => stack.push(Value::I(v)),
                Op::ConstF(v) => stack.push(Value::F(v)),
                Op::Var(i) => stack.push(*vars.get(usize::from(i)).ok_or_else(|| {
                    format!("variable '{}' unbound", self.variables[usize::from(i)])
                })?),
                Op::TernaryColon => {}
                Op::Ternary => {
                    let c = pop(&mut stack)?;
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(if a.truthy() { b } else { c });
                }
                _ => {
                    let value = apply(op, &mut stack)?;
                    stack.push(value);
                }
            }
        }
        if stack.len() != 1 {
            return Err("malformed expression".into());
        }
        pop(&mut stack)
    }
}

fn pop(stack: &mut Vec<Value>) -> Result<Value, String> {
    stack
        .pop()
        .ok_or_else(|| "expression stack underflow".into())
}

fn apply(op: Op, stack: &mut Vec<Value>) -> Result<Value, String> {
    use Value::{F, I};
    let unary = |stack: &mut Vec<Value>| pop(stack);
    let binary = |stack: &mut Vec<Value>| -> Result<(Value, Value), String> {
        let b = pop(stack)?;
        let a = pop(stack)?;
        Ok((a, b))
    };
    let float_fn = |stack: &mut Vec<Value>, f: fn(f64) -> f64| -> Result<Value, String> {
        Ok(F(f(pop(stack)?.as_f64())))
    };

    Ok(match op {
        Op::Or => {
            let (a, b) = binary(stack)?;
            I(i64::from(a.truthy() || b.truthy()))
        }
        Op::And => {
            let (a, b) = binary(stack)?;
            I(i64::from(a.truthy() && b.truthy()))
        }
        Op::BitOr => int_binop(stack, |a, b| a | b)?,
        Op::BitXor => int_binop(stack, |a, b| a ^ b)?,
        Op::BitAnd => int_binop(stack, |a, b| a & b)?,
        Op::Shl => int_binop(stack, |a, b| a.wrapping_shl(b as u32))?,
        Op::Shr => int_binop(stack, |a, b| a.wrapping_shr(b as u32))?,
        Op::Rem => {
            let (a, b) = binary(stack)?;
            let divisor = b.as_i64();
            if divisor == 0 {
                return Err("remainder by zero".into());
            }
            I(a.as_i64().wrapping_rem(divisor))
        }
        Op::Eq => compare(stack, |o| o == std::cmp::Ordering::Equal)?,
        Op::Ne => compare(stack, |o| o != std::cmp::Ordering::Equal)?,
        Op::Le => compare(stack, |o| o != std::cmp::Ordering::Greater)?,
        Op::Ge => compare(stack, |o| o != std::cmp::Ordering::Less)?,
        Op::Lt => compare(stack, |o| o == std::cmp::Ordering::Less)?,
        Op::Gt => compare(stack, |o| o == std::cmp::Ordering::Greater)?,
        Op::Add => arith(stack, i64::wrapping_add, |a, b| a + b)?,
        Op::Sub => arith(stack, i64::wrapping_sub, |a, b| a - b)?,
        Op::Mul => arith(stack, i64::wrapping_mul, |a, b| a * b)?,
        Op::Div => {
            let (a, b) = binary(stack)?;
            match (a, b) {
                (I(x), I(y)) => {
                    if y == 0 {
                        return Err("division by zero".into());
                    }
                    I(x.wrapping_div(y))
                }
                (a, b) => F(a.as_f64() / b.as_f64()),
            }
        }
        Op::Pow => {
            let (a, b) = binary(stack)?;
            F(a.as_f64().powf(b.as_f64()))
        }
        Op::UnaryMinus | Op::NegFn => match unary(stack)? {
            I(v) => I(v.wrapping_neg()),
            F(v) => F(-v),
        },
        Op::UnaryPlus => unary(stack)?,
        Op::BitNot => I(!pop(stack)?.as_i64()),
        Op::Sgn => match unary(stack)? {
            I(v) => I(v.signum()),
            F(v) => I(if v > 0.0 {
                1
            } else if v < 0.0 {
                -1
            } else {
                0
            }),
        },
        Op::Abs => match unary(stack)? {
            I(v) => I(v.wrapping_abs()),
            F(v) => F(v.abs()),
        },
        Op::Sin => float_fn(stack, f64::sin)?,
        Op::Cos => float_fn(stack, f64::cos)?,
        Op::Tan => float_fn(stack, f64::tan)?,
        Op::Asin => float_fn(stack, f64::asin)?,
        Op::Acos => float_fn(stack, f64::acos)?,
        Op::Atan => float_fn(stack, f64::atan)?,
        Op::Exp => float_fn(stack, f64::exp)?,
        Op::Ln => float_fn(stack, f64::ln)?,
        Op::Lg => float_fn(stack, f64::log10)?,
        Op::Sqrt => float_fn(stack, f64::sqrt)?,
        Op::Trunc => float_fn(stack, f64::trunc)?,
        Op::Round => float_fn(stack, f64::round)?,
        Op::Floor => float_fn(stack, f64::floor)?,
        Op::Ceil => float_fn(stack, f64::ceil)?,
        _ => return Err("internal: operand in operator position".into()),
    })
}

fn int_binop(stack: &mut Vec<Value>, f: fn(i64, i64) -> i64) -> Result<Value, String> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    Ok(Value::I(f(a.as_i64(), b.as_i64())))
}

fn arith(
    stack: &mut Vec<Value>,
    int_f: fn(i64, i64) -> i64,
    float_f: fn(f64, f64) -> f64,
) -> Result<Value, String> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    Ok(match (a, b) {
        (Value::I(x), Value::I(y)) => Value::I(int_f(x, y)),
        (a, b) => Value::F(float_f(a.as_f64(), b.as_f64())),
    })
}

fn compare(stack: &mut Vec<Value>, f: fn(std::cmp::Ordering) -> bool) -> Result<Value, String> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let ordering = match (a, b) {
        (Value::I(x), Value::I(y)) => x.cmp(&y),
        (a, b) => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            .ok_or("NaN in comparison")?,
    };
    Ok(Value::I(i64::from(f(ordering))))
}

fn push_operator(op: Op, stack: &mut Vec<Option<Op>>, output: &mut Vec<Op>) {
    let (precedence, right_assoc) = op.binding();
    while let Some(Some(top)) = stack.last() {
        let (top_precedence, _) = top.binding();
        let pops = if right_assoc {
            top_precedence > precedence
        } else {
            top_precedence >= precedence
        };
        if !pops {
            break;
        }
        if let Some(Some(top)) = stack.pop() {
            output.push(top);
        }
    }
    stack.push(Some(op));
}

fn function_named(word: &str) -> Option<Op> {
    Some(match word.to_ascii_uppercase().as_str() {
        "SIN" => Op::Sin,
        "COS" => Op::Cos,
        "SGN" => Op::Sgn,
        "NEG" => Op::NegFn,
        "ATAN" => Op::Atan,
        "TAN" => Op::Tan,
        "ABS" => Op::Abs,
        "EXP" => Op::Exp,
        "LN" => Op::Ln,
        "LG" => Op::Lg,
        "SQRT" => Op::Sqrt,
        "TRUNC" => Op::Trunc,
        "ROUND" => Op::Round,
        "FLOOR" => Op::Floor,
        "CEIL" => Op::Ceil,
        "ASIN" => Op::Asin,
        "ACOS" => Op::Acos,
        _ => return None,
    })
}

fn parse_number(src: &str) -> Result<(Op, usize), String> {
    let bytes = src.as_bytes();
    if src.len() > 2 && (src.starts_with("0x") || src.starts_with("0X")) {
        let end = bytes[2..]
            .iter()
            .position(|b| !b.is_ascii_hexdigit())
            .map_or(src.len(), |p| p + 2);
        let value =
            i64::from_str_radix(&src[2..end], 16).map_err(|e| format!("bad hex literal: {e}"))?;
        return Ok((Op::ConstI(value), end));
    }
    let mut end = 0;
    let mut is_float = false;
    while end < bytes.len() {
        match bytes[end] {
            b'0'..=b'9' => end += 1,
            b'.' => {
                is_float = true;
                end += 1;
            }
            b'e' | b'E'
                if end + 1 < bytes.len()
                    && (bytes[end + 1].is_ascii_digit()
                        || bytes[end + 1] == b'-'
                        || bytes[end + 1] == b'+') =>
            {
                is_float = true;
                end += 2;
            }
            _ => break,
        }
    }
    if is_float {
        let value: f64 = src[..end]
            .parse()
            .map_err(|e| format!("bad float literal: {e}"))?;
        Ok((Op::ConstF(value), end))
    } else {
        let value: i64 = src[..end]
            .parse()
            .map_err(|e| format!("bad int literal: {e}"))?;
        Ok((Op::ConstI(value), end))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_int(src: &str) -> i64 {
        Expr::parse(src).unwrap().eval(&[]).unwrap().as_i64()
    }

    fn eval_f64(src: &str) -> f64 {
        Expr::parse(src).unwrap().eval(&[]).unwrap().as_f64()
    }

    #[test]
    fn precedence_and_parens() {
        assert_eq!(eval_int("1+2*3"), 7);
        assert_eq!(eval_int("(1+2)*3"), 9);
        assert_eq!(eval_int("10-4-3"), 3);
        assert_eq!(eval_int("2**10"), 1024);
        assert_eq!(eval_int("2**3**2"), 512); // right associative
        assert_eq!(eval_int("1+2<<3"), 24); // shift binds looser than +
        assert_eq!(eval_int("7%3"), 1);
        assert_eq!(eval_int("7/2"), 3); // integer division
        assert!((eval_f64("7.0/2") - 3.5).abs() < 1e-12);
    }

    #[test]
    fn unary_and_bitwise() {
        assert_eq!(eval_int("-3+5"), 2);
        assert_eq!(eval_int("4*-2"), -8);
        assert_eq!(eval_int("~0"), -1);
        assert_eq!(eval_int("0xff & 0x0f"), 0x0f);
        assert_eq!(eval_int("1|2|4"), 7);
        assert_eq!(eval_int("5^1"), 4);
        assert_eq!(eval_int("1<<16"), 0x10000);
    }

    #[test]
    fn comparisons_and_logic() {
        assert_eq!(eval_int("1=1"), 1);
        assert_eq!(eval_int("1<>1"), 0);
        assert_eq!(eval_int("2>=3"), 0);
        assert_eq!(eval_int("2<3 && 3<4"), 1);
        assert_eq!(eval_int("0||0"), 0);
    }

    #[test]
    fn ternary() {
        assert_eq!(eval_int("1?2:3"), 2);
        assert_eq!(eval_int("0?2:3"), 3);
        assert_eq!(eval_int("1?2:0?3:4"), 2);
        assert_eq!(eval_int("0?2:0?3:4"), 4);
    }

    #[test]
    fn functions_and_constants() {
        assert!((eval_f64("SIN(0)")).abs() < 1e-12);
        assert!((eval_f64("COS(PI)") + 1.0).abs() < 1e-12);
        assert_eq!(eval_int("ABS(-5)"), 5);
        assert_eq!(eval_int("SGN(-3)"), -1);
        assert!((eval_f64("SQRT(16)") - 4.0).abs() < 1e-12);
        assert!((eval_f64("LN(E)") - 1.0).abs() < 1e-12);
        assert_eq!(eval_int("ROUND(2.5)"), 3);
        assert_eq!(eval_int("TRUNC(2.9)"), 2);
    }

    #[test]
    fn hex_and_float_literals() {
        assert_eq!(eval_int("0x10 + 0X0F"), 31);
        assert!((eval_f64("1.5E2") - 150.0).abs() < 1e-12);
        assert!((eval_f64(".5 * 4") - 2.0).abs() < 1e-12);
    }

    #[test]
    fn variables() {
        let expr = Expr::parse("(TO * GAIN) + 1").unwrap();
        assert_eq!(expr.variables(), ["TO", "GAIN"]);
        let v = expr.eval(&[Value::I(10), Value::I(3)]).unwrap();
        assert_eq!(v.as_i64(), 31);
        assert!(expr.eval(&[Value::I(10)]).is_err());
    }

    #[test]
    fn converter_style_formula() {
        // Typical exposure converter: register microseconds -> seconds.
        let from = Expr::parse("TO / 1000000.0").unwrap();
        let v = from.eval(&[Value::I(20_000)]).unwrap();
        assert!((v.as_f64() - 0.02).abs() < 1e-12);
    }
}
