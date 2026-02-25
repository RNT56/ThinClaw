use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
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
    /// Optional named variables, e.g. {"x": 3, "rate": 0.05}
    #[serde(default)]
    pub variables: Option<HashMap<String, f64>>,
}

// =============================================================================
// Evaluation output with trace
// =============================================================================

/// Full result of evaluating a math expression.
#[derive(Debug, Clone)]
pub struct EvalOutput {
    /// The numeric result.
    pub result: f64,
    /// Human-readable formatted result (e.g. "42" not "42.0").
    pub formatted: String,
    /// Step-by-step trace of operations performed, in order.
    pub trace: Vec<String>,
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
            description: "Evaluate mathematical expressions with high precision and show work. \
                Supports basic arithmetic (+, -, *, /, ^, %), parentheses, \
                functions (sqrt, abs, round, ceil, floor, log, ln, log2, sin, cos, tan, \
                asin, acos, atan, min, max, pow, exp), constants (pi, e, tau), and \
                named variables. Returns a step-by-step trace of all operations. \
                Variables can be passed via the 'variables' parameter or defined inline \
                with semicolons: 'x = 3; y = 5; 2*x^2 + y'. \
                Use this for ANY calculation — currency conversions, percentages, \
                unit conversions, tip calculations, compound interest, etc. \
                Example: '(1299.99 * 0.85) + (1299.99 * 0.85 * 0.19)' for discount + tax."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "The math expression to evaluate. Supports inline variable assignments separated by ';', e.g. 'x = 3; 2*x^2 + 5*x - 7'"
                    },
                    "variables": {
                        "type": "object",
                        "description": "Optional named variables as key-value pairs, e.g. {\"x\": 3, \"rate\": 0.05}",
                        "additionalProperties": { "type": "number" }
                    }
                },
                "required": ["expression"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let vars = args.variables.unwrap_or_default();
        match evaluate_with_vars(&args.expression, vars) {
            Ok(output) => Ok(format_eval_output(&output)),
            Err(e) => Err(CalcError::Eval(e)),
        }
    }
}

// =============================================================================
// Public evaluator — also called directly by the Rhai sandbox binding
// =============================================================================

/// Evaluate a mathematical expression string and return the numeric result.
/// This is the simple backward-compatible interface — no variables, no trace.
pub fn evaluate(expr: &str) -> Result<f64, String> {
    let output = evaluate_full(expr, HashMap::new())?;
    Ok(output.result)
}

/// Evaluate with named variables and return full output including step-by-step trace.
pub fn evaluate_with_vars(
    expr: &str,
    variables: HashMap<String, f64>,
) -> Result<EvalOutput, String> {
    evaluate_full(expr, variables)
}

/// Format an EvalOutput into a human-readable string with trace.
pub fn format_eval_output(output: &EvalOutput) -> String {
    let mut result = String::new();
    if !output.trace.is_empty() {
        for (i, step) in output.trace.iter().enumerate() {
            result.push_str(&format!("Step {}: {}\n", i + 1, step));
        }
    }
    result.push_str(&format!("Result: {}", output.formatted));
    result
}

/// Internal full evaluator supporting variables, inline assignments, and trace.
fn evaluate_full(expr: &str, mut variables: HashMap<String, f64>) -> Result<EvalOutput, String> {
    // Support inline variable assignments separated by ';'
    // e.g. "x = 3; y = 5; 2*x + y"
    let segments: Vec<&str> = expr.split(';').collect();
    let mut all_trace = Vec::new();

    if segments.len() > 1 {
        // Process all but the last segment as variable assignments
        for segment in &segments[..segments.len() - 1] {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            // Parse "name = expression"
            if let Some(eq_pos) = segment.find('=') {
                let var_name = segment[..eq_pos].trim().to_lowercase();
                let var_expr = segment[eq_pos + 1..].trim();

                // Validate variable name
                if var_name.is_empty()
                    || !var_name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    return Err(format!("Invalid variable name: '{}'", var_name));
                }
                if var_name.chars().next().map_or(true, |c| c.is_ascii_digit()) {
                    return Err(format!(
                        "Variable name cannot start with a digit: '{}'",
                        var_name
                    ));
                }
                // Prevent shadowing built-in constants
                if matches!(var_name.as_str(), "pi" | "e" | "tau" | "inf" | "infinity") {
                    return Err(format!(
                        "Cannot use reserved constant name as variable: '{}'",
                        var_name
                    ));
                }

                // Evaluate the right-hand side with current variables
                let tokens = tokenize(var_expr)?;
                let mut parser = Parser::new(tokens, variables.clone());
                let value = parser.parse_expr()?;
                if parser.pos < parser.tokens.len() {
                    return Err(format!(
                        "Unexpected token in assignment for '{}': {:?}",
                        var_name, parser.tokens[parser.pos]
                    ));
                }

                // Collect inner trace first, then the assignment itself
                all_trace.extend(parser.trace);
                all_trace.push(format!("let {} = {}", var_name, format_number(value)));
                variables.insert(var_name, value);
            } else {
                return Err(format!(
                    "Expected variable assignment (name = value), got: '{}'",
                    segment
                ));
            }
        }
    }

    // Evaluate the final (or only) expression
    let final_expr = segments.last().unwrap_or(&"").trim();
    if final_expr.is_empty() {
        return Err("Empty expression".to_string());
    }

    let tokens = tokenize(final_expr)?;
    let mut parser = Parser::new(tokens, variables);
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

    all_trace.extend(parser.trace);

    Ok(EvalOutput {
        result,
        formatted: format_number(result),
        trace: all_trace,
    })
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
// Recursive-descent parser with variable context and trace
// =============================================================================

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    variables: HashMap<String, f64>,
    trace: Vec<String>,
}

impl Parser {
    fn new(tokens: Vec<Token>, variables: HashMap<String, f64>) -> Self {
        Self {
            tokens,
            pos: 0,
            variables,
            trace: Vec::new(),
        }
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
                    let right = self.parse_term()?;
                    let result = left + right;
                    self.trace.push(format!(
                        "{} + {} = {}",
                        format_number(left),
                        format_number(right),
                        format_number(result)
                    ));
                    left = result;
                }
                Token::Minus => {
                    self.advance();
                    let right = self.parse_term()?;
                    let result = left - right;
                    self.trace.push(format!(
                        "{} − {} = {}",
                        format_number(left),
                        format_number(right),
                        format_number(result)
                    ));
                    left = result;
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
                    let right = self.parse_power()?;
                    let result = left * right;
                    self.trace.push(format!(
                        "{} × {} = {}",
                        format_number(left),
                        format_number(right),
                        format_number(result)
                    ));
                    left = result;
                }
                Token::Slash => {
                    self.advance();
                    let right = self.parse_power()?;
                    if right == 0.0 {
                        return Err("Division by zero".to_string());
                    }
                    let result = left / right;
                    self.trace.push(format!(
                        "{} ÷ {} = {}",
                        format_number(left),
                        format_number(right),
                        format_number(result)
                    ));
                    left = result;
                }
                Token::Percent => {
                    self.advance();
                    let right = self.parse_power()?;
                    if right == 0.0 {
                        return Err("Modulo by zero".to_string());
                    }
                    let result = left % right;
                    self.trace.push(format!(
                        "{} mod {} = {}",
                        format_number(left),
                        format_number(right),
                        format_number(result)
                    ));
                    left = result;
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
            let result = base.powf(exp);
            self.trace.push(format!(
                "{} ^ {} = {}",
                format_number(base),
                format_number(exp),
                format_number(result)
            ));
            Ok(result)
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
                    let result = eval_function(&name, &args)?;
                    // Build trace for function call
                    let args_str: Vec<String> = args.iter().map(|a| format_number(*a)).collect();
                    self.trace.push(format!(
                        "{}({}) = {}",
                        name,
                        args_str.join(", "),
                        format_number(result)
                    ));
                    Ok(result)
                } else {
                    // Check variables first, then constants
                    if let Some(&value) = self.variables.get(&name) {
                        Ok(value)
                    } else {
                        eval_constant(&name)
                    }
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

    // =========================================================================
    // NEW: Variable support tests
    // =========================================================================

    fn eval_vars(expr: &str, vars: Vec<(&str, f64)>) -> f64 {
        let variables: HashMap<String, f64> =
            vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        evaluate_with_vars(expr, variables)
            .unwrap_or_else(|e| panic!("Failed to evaluate '{}': {}", expr, e))
            .result
    }

    #[test]
    fn variable_simple() {
        assert_eq!(eval_vars("x + 1", vec![("x", 5.0)]), 6.0);
    }

    #[test]
    fn variable_quadratic() {
        // 2x² + 5x - 7 with x=3 → 2*9 + 15 - 7 = 26
        assert_eq!(eval_vars("2*x^2 + 5*x - 7", vec![("x", 3.0)]), 26.0);
    }

    #[test]
    fn variable_multiple() {
        // a*x + b with a=2, x=5, b=3 → 13
        assert_eq!(
            eval_vars("a*x + b", vec![("a", 2.0), ("x", 5.0), ("b", 3.0)]),
            13.0
        );
    }

    #[test]
    fn variable_in_function() {
        assert_eq!(eval_vars("sqrt(x)", vec![("x", 144.0)]), 12.0);
    }

    #[test]
    fn variable_with_constant() {
        // r * pi with r=5 → 5π ≈ 15.707963
        let result = eval_vars("r * pi", vec![("r", 5.0)]);
        assert!((result - 5.0 * std::f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn variable_compound_interest() {
        // P * (1 + r/n)^(n*t) with P=1000, r=0.05, n=12, t=10
        let result = eval_vars(
            "p * (1 + r/n) ^ (n*t)",
            vec![("p", 1000.0), ("r", 0.05), ("n", 12.0), ("t", 10.0)],
        );
        let expected = 1000.0 * (1.0 + 0.05 / 12.0_f64).powf(120.0);
        assert!((result - expected).abs() < 0.01);
    }

    #[test]
    fn variable_unknown_still_errors() {
        let vars: HashMap<String, f64> = HashMap::new();
        let err = evaluate_with_vars("x + 1", vars).unwrap_err();
        assert!(err.contains("Unknown identifier"), "got: {}", err);
    }

    // =========================================================================
    // NEW: Inline variable assignment tests
    // =========================================================================

    #[test]
    fn inline_single_var() {
        assert_eq!(eval("x = 5; x + 1"), 6.0);
    }

    #[test]
    fn inline_multiple_vars() {
        assert_eq!(eval("x = 3; y = 5; x + y"), 8.0);
    }

    #[test]
    fn inline_var_references_previous() {
        // x = 3; y = x + 2 (= 5); x * y = 15
        assert_eq!(eval("x = 3; y = x + 2; x * y"), 15.0);
    }

    #[test]
    fn inline_quadratic() {
        // x = 3; 2*x^2 + 5*x - 7 = 26
        assert_eq!(eval("x = 3; 2*x^2 + 5*x - 7"), 26.0);
    }

    #[test]
    fn inline_with_functions() {
        // r = 5; area = pi * r^2
        let result = eval("r = 5; pi * r^2");
        let expected = std::f64::consts::PI * 25.0;
        assert!((result - expected).abs() < 1e-10);
    }

    #[test]
    fn inline_empty_segments_skipped() {
        // Extra semicolons should be handled gracefully
        assert_eq!(eval("x = 5; ; x + 1"), 6.0);
    }

    #[test]
    fn inline_invalid_var_name_digit() {
        let err = evaluate("3x = 5; 3x + 1").unwrap_err();
        assert!(
            err.contains("digit") || err.contains("Invalid"),
            "got: {}",
            err
        );
    }

    #[test]
    fn inline_cannot_shadow_constant() {
        let err = evaluate("pi = 5; pi + 1").unwrap_err();
        assert!(
            err.contains("reserved") || err.contains("constant"),
            "got: {}",
            err
        );
    }

    // =========================================================================
    // NEW: Trace output tests
    // =========================================================================

    fn get_trace(expr: &str) -> Vec<String> {
        evaluate_full(expr, HashMap::new()).unwrap().trace
    }

    fn get_trace_with_vars(expr: &str, vars: Vec<(&str, f64)>) -> Vec<String> {
        let variables: HashMap<String, f64> =
            vars.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        evaluate_full(expr, variables).unwrap().trace
    }

    #[test]
    fn trace_simple_addition() {
        let trace = get_trace("2 + 3");
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0], "2 + 3 = 5");
    }

    #[test]
    fn trace_compound_expression() {
        let trace = get_trace("(2 + 3) * 4");
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0], "2 + 3 = 5");
        assert_eq!(trace[1], "5 × 4 = 20");
    }

    #[test]
    fn trace_function_call() {
        let trace = get_trace("sqrt(144)");
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0], "sqrt(144) = 12");
    }

    #[test]
    fn trace_power() {
        let trace = get_trace("2 ^ 10");
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0], "2 ^ 10 = 1024");
    }

    #[test]
    fn trace_no_steps_for_literal() {
        // A single number has nothing to trace
        let trace = get_trace("42");
        assert!(trace.is_empty());
    }

    #[test]
    fn trace_inline_variables() {
        let trace = get_trace("x = 3; y = 5; x + y");
        // Should have: "let x = 3", "let y = 5", "3 + 5 = 8"
        assert!(trace.len() >= 2);
        assert!(trace.contains(&"let x = 3".to_string()));
        assert!(trace.contains(&"let y = 5".to_string()));
        assert!(trace.contains(&"3 + 5 = 8".to_string()));
    }

    #[test]
    fn trace_variables_from_params() {
        let trace = get_trace_with_vars("2*x + 1", vec![("x", 5.0)]);
        // Should trace: "2 × 5 = 10", "10 + 1 = 11"
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0], "2 × 5 = 10");
        assert_eq!(trace[1], "10 + 1 = 11");
    }

    #[test]
    fn trace_quadratic_with_vars() {
        // 2*x^2 + 5*x - 7 with x=3
        // Steps: 3^2=9, 2×9=18, 5×3=15, 18+15=33, 33−7=26
        let trace = get_trace_with_vars("2*x^2 + 5*x - 7", vec![("x", 3.0)]);
        assert_eq!(trace.len(), 5);
        assert_eq!(trace[0], "3 ^ 2 = 9");
        assert_eq!(trace[1], "2 × 9 = 18");
        assert_eq!(trace[2], "5 × 3 = 15");
        assert_eq!(trace[3], "18 + 15 = 33");
        assert_eq!(trace[4], "33 − 7 = 26");
    }

    #[test]
    fn trace_discount_plus_tax() {
        let trace = get_trace("(100 * 0.85) + (100 * 0.85 * 0.19)");
        // 100×0.85=85, 100×0.85=85, 85×0.19=16.15, 85+16.15=101.15
        assert_eq!(trace.len(), 4);
    }

    #[test]
    fn format_eval_output_simple() {
        let output = evaluate_full("2 + 3", HashMap::new()).unwrap();
        let formatted = format_eval_output(&output);
        assert!(formatted.contains("Step 1:"));
        assert!(formatted.contains("Result: 5"));
    }

    #[test]
    fn format_eval_output_no_trace() {
        let output = evaluate_full("42", HashMap::new()).unwrap();
        let formatted = format_eval_output(&output);
        assert_eq!(formatted, "Result: 42");
    }
}
