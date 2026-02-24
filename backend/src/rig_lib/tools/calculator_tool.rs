use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

// =============================================================================
// Error type
// =============================================================================

#[derive(Debug, Error)]
pub enum CalcError {
    #[error("Calculation error: {0}")]
    Eval(String),
}

// =============================================================================
// Tool args
// =============================================================================

#[derive(Deserialize)]
pub struct CalcArgs {
    pub expression: String,
}

// =============================================================================
// Tool struct (zero-size — no state needed)
// =============================================================================

pub struct CalculatorTool;

impl Tool for CalculatorTool {
    const NAME: &'static str = "calculator";

    type Error = CalcError;
    type Args = CalcArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "calculator".to_string(),
            description: "Evaluate mathematical expressions with high precision. \
                Supports basic arithmetic (+, -, *, /, ^, %), parentheses, \
                functions (sqrt, abs, round, ceil, floor, log, ln, sin, cos, tan, min, max), \
                and constants (pi, e). Use this for ANY calculation — currency conversions, \
                percentages, unit conversions, tip calculations, etc. \
                Example: '(1299.99 * 0.85) + (1299.99 * 0.85 * 0.19)' for discount + tax."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "The math expression to evaluate, e.g. '(100 * 1.35) + 20'"
                    }
                },
                "required": ["expression"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match evaluate(&args.expression) {
            Ok(result) => {
                // Format nicely: strip trailing zeros for clean display
                let formatted = format_number(result);
                Ok(format!("Result: {}", formatted))
            }
            Err(e) => Err(CalcError::Eval(e)),
        }
    }
}

// =============================================================================
// Public evaluator — also called directly by the Rhai sandbox binding
// =============================================================================

/// Evaluate a mathematical expression string and return the numeric result.
pub fn evaluate(expr: &str) -> Result<f64, String> {
    let tokens = tokenize(expr)?;
    let mut parser = Parser::new(tokens);
    let result = parser.parse_expr()?;
    if parser.pos < parser.tokens.len() {
        return Err(format!(
            "Unexpected token after expression: {:?}",
            parser.tokens[parser.pos]
        ));
    }
    if result.is_nan() {
        return Err("Result is NaN (undefined)".to_string());
    }
    if result.is_infinite() {
        return Err("Result is infinite (division by zero or overflow)".to_string());
    }
    Ok(result)
}

// =============================================================================
// Number formatting
// =============================================================================

fn format_number(n: f64) -> String {
    if n == n.floor() && n.abs() < 1e15 {
        // Integer-valued — display without decimal point
        format!("{}", n as i64)
    } else {
        // Use up to 10 decimal places, strip trailing zeros
        let s = format!("{:.10}", n);
        let s = s.trim_end_matches('0');
        let s = s.trim_end_matches('.');
        s.to_string()
    }
}

// =============================================================================
// Tokenizer
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
    Ident(String),
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                // Handle ** as power (Python-style)
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    tokens.push(Token::Caret);
                    i += 2;
                } else {
                    tokens.push(Token::Star);
                    i += 1;
                }
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                i += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // Handle scientific notation: 1e5, 2.5E-3
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    i += 1;
                    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                        i += 1;
                    }
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let num_str: String = chars[start..i].iter().collect();
                let num: f64 = num_str
                    .parse()
                    .map_err(|_| format!("Invalid number: '{}'", num_str))?;
                tokens.push(Token::Number(num));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                tokens.push(Token::Ident(ident.to_lowercase()));
            }
            c => return Err(format!("Unexpected character: '{}'", c)),
        }
    }

    Ok(tokens)
}

// =============================================================================
// Recursive-descent parser
// =============================================================================

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(tok)
        } else {
            None
        }
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        match self.advance() {
            Some(ref tok) if tok == expected => Ok(()),
            Some(tok) => Err(format!("Expected {:?}, got {:?}", expected, tok)),
            None => Err(format!("Expected {:?}, got end of expression", expected)),
        }
    }

    // Grammar (precedence: low → high):
    //   expr     = term (('+' | '-') term)*
    //   term     = power (('*' | '/' | '%') power)*
    //   power    = unary ('^' power)?        ← right-associative
    //   unary    = ('-' | '+')? primary
    //   primary  = NUMBER | IDENT '(' args ')' | IDENT | '(' expr ')'

    fn parse_expr(&mut self) -> Result<f64, String> {
        let mut left = self.parse_term()?;
        while let Some(tok) = self.peek() {
            match tok {
                Token::Plus => {
                    self.advance();
                    left += self.parse_term()?;
                }
                Token::Minus => {
                    self.advance();
                    left -= self.parse_term()?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<f64, String> {
        let mut left = self.parse_power()?;
        while let Some(tok) = self.peek() {
            match tok {
                Token::Star => {
                    self.advance();
                    left *= self.parse_power()?;
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_power()?;
                    if right == 0.0 {
                        return Err("Division by zero".to_string());
                    }
                    left /= right;
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_power()?;
                    if right == 0.0 {
                        return Err("Modulo by zero".to_string());
                    }
                    left %= right;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_power(&mut self) -> Result<f64, String> {
        let base = self.parse_unary()?;
        if let Some(Token::Caret) = self.peek() {
            self.advance();
            let exp = self.parse_power()?; // right-associative recursion
            Ok(base.powf(exp))
        } else {
            Ok(base)
        }
    }

    fn parse_unary(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(Token::Minus) => {
                self.advance();
                Ok(-self.parse_unary()?)
            }
            Some(Token::Plus) => {
                self.advance();
                self.parse_unary()
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<f64, String> {
        match self.advance() {
            Some(Token::Number(n)) => Ok(n),

            Some(Token::Ident(name)) => {
                // Check if it's a function call: IDENT '(' args ')'
                if let Some(Token::LParen) = self.peek() {
                    self.advance(); // consume '('
                    let args = self.parse_args()?;
                    self.expect(&Token::RParen)?;
                    eval_function(&name, &args)
                } else {
                    // It's a constant
                    eval_constant(&name)
                }
            }

            Some(Token::LParen) => {
                let val = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(val)
            }

            Some(tok) => Err(format!("Unexpected token: {:?}", tok)),
            None => Err("Unexpected end of expression".to_string()),
        }
    }

    fn parse_args(&mut self) -> Result<Vec<f64>, String> {
        let mut args = Vec::new();

        // Handle empty argument list
        if let Some(Token::RParen) = self.peek() {
            return Ok(args);
        }

        args.push(self.parse_expr()?);
        while let Some(Token::Comma) = self.peek() {
            self.advance();
            args.push(self.parse_expr()?);
        }
        Ok(args)
    }
}

// =============================================================================
// Built-in functions
// =============================================================================

fn eval_function(name: &str, args: &[f64]) -> Result<f64, String> {
    match name {
        "sqrt" => {
            require_args(name, args, 1)?;
            if args[0] < 0.0 {
                return Err("Cannot take square root of a negative number".to_string());
            }
            Ok(args[0].sqrt())
        }
        "abs" => {
            require_args(name, args, 1)?;
            Ok(args[0].abs())
        }
        "round" => {
            require_args(name, args, 1)?;
            Ok(args[0].round())
        }
        "ceil" => {
            require_args(name, args, 1)?;
            Ok(args[0].ceil())
        }
        "floor" => {
            require_args(name, args, 1)?;
            Ok(args[0].floor())
        }
        "log" | "log10" => {
            require_args(name, args, 1)?;
            if args[0] <= 0.0 {
                return Err("Logarithm of non-positive number".to_string());
            }
            Ok(args[0].log10())
        }
        "ln" => {
            require_args(name, args, 1)?;
            if args[0] <= 0.0 {
                return Err("Logarithm of non-positive number".to_string());
            }
            Ok(args[0].ln())
        }
        "log2" => {
            require_args(name, args, 1)?;
            if args[0] <= 0.0 {
                return Err("Logarithm of non-positive number".to_string());
            }
            Ok(args[0].log2())
        }
        "sin" => {
            require_args(name, args, 1)?;
            Ok(args[0].sin())
        }
        "cos" => {
            require_args(name, args, 1)?;
            Ok(args[0].cos())
        }
        "tan" => {
            require_args(name, args, 1)?;
            Ok(args[0].tan())
        }
        "asin" => {
            require_args(name, args, 1)?;
            Ok(args[0].asin())
        }
        "acos" => {
            require_args(name, args, 1)?;
            Ok(args[0].acos())
        }
        "atan" => {
            require_args(name, args, 1)?;
            Ok(args[0].atan())
        }
        "min" => {
            require_min_args(name, args, 2)?;
            Ok(args.iter().cloned().fold(f64::INFINITY, f64::min))
        }
        "max" => {
            require_min_args(name, args, 2)?;
            Ok(args.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
        }
        "pow" => {
            require_args(name, args, 2)?;
            Ok(args[0].powf(args[1]))
        }
        "exp" => {
            require_args(name, args, 1)?;
            Ok(args[0].exp())
        }
        _ => Err(format!("Unknown function: '{}'", name)),
    }
}

fn eval_constant(name: &str) -> Result<f64, String> {
    match name {
        "pi" => Ok(std::f64::consts::PI),
        "e" => Ok(std::f64::consts::E),
        "tau" => Ok(std::f64::consts::TAU),
        "inf" | "infinity" => Ok(f64::INFINITY),
        _ => Err(format!(
            "Unknown identifier: '{}'. Did you mean a function call like '{}(…)'?",
            name, name
        )),
    }
}

fn require_args(name: &str, args: &[f64], expected: usize) -> Result<(), String> {
    if args.len() != expected {
        Err(format!(
            "Function '{}' expects {} argument(s), got {}",
            name,
            expected,
            args.len()
        ))
    } else {
        Ok(())
    }
}

fn require_min_args(name: &str, args: &[f64], min: usize) -> Result<(), String> {
    if args.len() < min {
        Err(format!(
            "Function '{}' expects at least {} argument(s), got {}",
            name,
            min,
            args.len()
        ))
    } else {
        Ok(())
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> f64 {
        evaluate(expr).unwrap_or_else(|e| panic!("Failed to evaluate '{}': {}", expr, e))
    }

    fn eval_err(expr: &str) -> String {
        evaluate(expr).unwrap_err()
    }

    // -- Basic arithmetic --

    #[test]
    fn basic_addition() {
        assert_eq!(eval("2 + 3"), 5.0);
    }

    #[test]
    fn basic_subtraction() {
        assert_eq!(eval("10 - 4"), 6.0);
    }

    #[test]
    fn basic_multiplication() {
        assert_eq!(eval("6 * 7"), 42.0);
    }

    #[test]
    fn basic_division() {
        assert_eq!(eval("20 / 4"), 5.0);
    }

    #[test]
    fn basic_modulo() {
        assert_eq!(eval("10 % 3"), 1.0);
    }

    // -- Order of operations --

    #[test]
    fn order_of_operations() {
        assert_eq!(eval("2 + 3 * 4"), 14.0);
    }

    #[test]
    fn left_associativity() {
        assert_eq!(eval("10 - 3 - 2"), 5.0);
    }

    #[test]
    fn parentheses() {
        assert_eq!(eval("(2 + 3) * 4"), 20.0);
    }

    #[test]
    fn nested_parentheses() {
        assert_eq!(eval("((2 + 3) * (4 - 1))"), 15.0);
    }

    // -- Power --

    #[test]
    fn power() {
        assert_eq!(eval("2 ^ 10"), 1024.0);
    }

    #[test]
    fn power_right_assoc() {
        // 2^3^2 = 2^(3^2) = 2^9 = 512
        assert_eq!(eval("2 ^ 3 ^ 2"), 512.0);
    }

    #[test]
    fn double_star_power() {
        assert_eq!(eval("2 ** 10"), 1024.0);
    }

    // -- Unary / negation --

    #[test]
    fn negative_number() {
        assert_eq!(eval("-5 + 3"), -2.0);
    }

    #[test]
    fn unary_plus() {
        assert_eq!(eval("+5"), 5.0);
    }

    #[test]
    fn double_negative() {
        assert_eq!(eval("--5"), 5.0);
    }

    // -- Functions --

    #[test]
    fn sqrt_function() {
        assert_eq!(eval("sqrt(144)"), 12.0);
    }

    #[test]
    fn abs_function() {
        assert_eq!(eval("abs(-42)"), 42.0);
    }

    #[test]
    fn round_function() {
        assert_eq!(eval("round(3.7)"), 4.0);
    }

    #[test]
    fn ceil_function() {
        assert_eq!(eval("ceil(3.1)"), 4.0);
    }

    #[test]
    fn floor_function() {
        assert_eq!(eval("floor(3.9)"), 3.0);
    }

    #[test]
    fn log10_function() {
        assert_eq!(eval("log(1000)"), 3.0);
    }

    #[test]
    fn ln_function() {
        assert!((eval("ln(e)") - 1.0).abs() < 1e-10);
    }

    #[test]
    fn sin_cos_tan() {
        assert!((eval("sin(0)")).abs() < 1e-10);
        assert!((eval("cos(0)") - 1.0).abs() < 1e-10);
        assert!((eval("tan(0)")).abs() < 1e-10);
    }

    #[test]
    fn min_max() {
        assert_eq!(eval("min(10, 20)"), 10.0);
        assert_eq!(eval("max(10, 20)"), 20.0);
    }

    #[test]
    fn nested_functions() {
        assert_eq!(eval("max(10, min(20, 15))"), 15.0);
    }

    #[test]
    fn pow_function() {
        assert_eq!(eval("pow(2, 8)"), 256.0);
    }

    #[test]
    fn exp_function() {
        assert!((eval("exp(1)") - std::f64::consts::E).abs() < 1e-10);
    }

    // -- Constants --

    #[test]
    fn pi_constant() {
        assert!((eval("pi * 2") - std::f64::consts::TAU).abs() < 1e-10);
    }

    #[test]
    fn e_constant() {
        assert!((eval("e") - std::f64::consts::E).abs() < 1e-10);
    }

    #[test]
    fn tau_constant() {
        assert!((eval("tau") - std::f64::consts::TAU).abs() < 1e-10);
    }

    // -- Scientific notation --

    #[test]
    fn scientific_notation() {
        assert_eq!(eval("1e3"), 1000.0);
        assert_eq!(eval("2.5E-3"), 0.0025);
    }

    // -- Real-world scenarios --

    #[test]
    fn currency_conversion() {
        // $1299.99 at rate 0.85 EUR/USD
        let result = eval("1299.99 * 0.85");
        assert!((result - 1104.9915).abs() < 1e-6);
    }

    #[test]
    fn discount_plus_tax() {
        // 15% discount on $1299.99, then 19% VAT
        let result = eval("(1299.99 * 0.85) + (1299.99 * 0.85 * 0.19)");
        let expected = (1299.99 * 0.85) + (1299.99 * 0.85 * 0.19);
        assert!((result - expected).abs() < 1e-6);
    }

    #[test]
    fn tip_calculation() {
        // 18% tip on $85.50
        let result = eval("85.50 * 1.18");
        assert!((result - 100.89).abs() < 1e-6);
    }

    #[test]
    fn compound_interest() {
        // P * (1 + r/n)^(n*t) : $1000 at 5% for 10 years compounded monthly
        let result = eval("1000 * (1 + 0.05/12) ^ (12 * 10)");
        let expected: f64 = 1000.0 * (1.0 + 0.05 / 12.0_f64).powf(120.0);
        assert!((result - expected).abs() < 0.01);
    }

    // -- Error handling --

    #[test]
    fn division_by_zero() {
        let err = eval_err("10 / 0");
        assert!(err.contains("zero"), "got: {}", err);
    }

    #[test]
    fn modulo_by_zero() {
        let err = eval_err("10 % 0");
        assert!(err.contains("zero"), "got: {}", err);
    }

    #[test]
    fn sqrt_negative() {
        let err = eval_err("sqrt(-1)");
        assert!(err.contains("negative"), "got: {}", err);
    }

    #[test]
    fn unknown_function() {
        let err = eval_err("foobar(1)");
        assert!(err.contains("Unknown function"), "got: {}", err);
    }

    #[test]
    fn unknown_identifier() {
        let err = eval_err("xyz");
        assert!(err.contains("Unknown identifier"), "got: {}", err);
    }

    #[test]
    fn mismatched_parentheses() {
        assert!(evaluate("(2 + 3").is_err());
    }

    #[test]
    fn empty_expression() {
        assert!(evaluate("").is_err());
    }

    #[test]
    fn wrong_arg_count() {
        let err = eval_err("sqrt(1, 2)");
        assert!(err.contains("expects"), "got: {}", err);
    }

    // -- format_number --

    #[test]
    fn format_integer() {
        assert_eq!(format_number(42.0), "42");
    }

    #[test]
    fn format_decimal() {
        assert_eq!(format_number(3.14), "3.14");
    }

    #[test]
    fn format_trailing_zeros() {
        assert_eq!(format_number(2.50), "2.5");
    }
}
