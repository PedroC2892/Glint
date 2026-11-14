// SPDX-License-Identifier: GPL-3.0-or-later
//! Calculator plus unit and currency conversion for the search bar.

use std::collections::HashMap;

/// Evaluate an arithmetic expression.
///
/// Returns `None` unless the text is a real calculation (it holds an operator,
/// a parenthesis or a function), so plain numbers and words stay normal searches.
pub fn calculate(input: &str) -> Option<f64> {
    let s = input.trim().trim_end_matches('=').trim();
    let tokens = tokenize(s)?;
    let is_calc = tokens
        .iter()
        .any(|t| matches!(t, Tok::Op(_) | Tok::LParen | Tok::Ident(_)));
    if !is_calc {
        return None;
    }
    let mut p = Parser { t: &tokens, i: 0 };
    let v = p.expr()?;
    (p.i == tokens.len() && v.is_finite()).then_some(v)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Op(char),
    LParen,
    RParen,
    Ident(String),
}

fn tokenize(s: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',')
            {
                i += 1;
            }
            let text: String = chars[start..i].iter().filter(|c| **c != ',').collect();
            out.push(Tok::Num(text.parse().ok()?));
        } else if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && chars[i].is_alphanumeric() {
                i += 1;
            }
            out.push(Tok::Ident(
                chars[start..i].iter().collect::<String>().to_lowercase(),
            ));
        } else {
            out.push(match c {
                '+' | '-' | '*' | '/' | '%' | '^' => Tok::Op(c),
                '×' => Tok::Op('*'),
                '÷' => Tok::Op('/'),
                '−' => Tok::Op('-'),
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                _ => return None,
            });
            i += 1;
        }
    }
    (!out.is_empty()).then_some(out)
}

struct Parser<'a> {
    t: &'a [Tok],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }

    fn eat_op(&mut self, ops: &[char]) -> Option<char> {
        match self.peek() {
            Some(Tok::Op(c)) if ops.contains(c) => {
                let c = *c;
                self.i += 1;
                Some(c)
            }
            _ => None,
        }
    }

    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(op) = self.eat_op(&['+', '-']) {
            let r = self.term()?;
            v = if op == '+' { v + r } else { v - r };
        }
        Some(v)
    }

    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        loop {
            if let Some(op) = self.eat_op(&['*', '/', '%']) {
                let r = self.unary()?;
                v = match op {
                    '*' => v * r,
                    '/' => v / r,
                    _ => v % r,
                };
            } else if matches!(self.peek(), Some(Tok::LParen | Tok::Ident(_))) {
                // Implicit multiplication: 2pi, 3(4+1).
                v *= self.unary()?;
            } else {
                return Some(v);
            }
        }
    }

    fn unary(&mut self) -> Option<f64> {
        match self.eat_op(&['-', '+']) {
            Some('-') => Some(-self.unary()?),
            Some(_) => self.unary(),
            None => self.power(),
        }
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.primary()?;
        if self.eat_op(&['^']).is_some() {
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    fn primary(&mut self) -> Option<f64> {
        let tok = self.peek()?.clone();
        self.i += 1;
        match tok {
            Tok::Num(n) => Some(n),
            Tok::LParen => {
                let v = self.expr()?;
                (self.peek() == Some(&Tok::RParen)).then(|| self.i += 1)?;
                Some(v)
            }
            Tok::Ident(name) => match name.as_str() {
                "pi" | "π" => Some(std::f64::consts::PI),
                "e" => Some(std::f64::consts::E),
                "tau" => Some(std::f64::consts::TAU),
                _ => {
                    let f: fn(f64) -> f64 = match name.as_str() {
                        "sqrt" => f64::sqrt,
                        "sin" => f64::sin,
                        "cos" => f64::cos,
                        "tan" => f64::tan,
                        "asin" => f64::asin,
                        "acos" => f64::acos,
                        "atan" => f64::atan,
                        "ln" => f64::ln,
                        "log" => f64::log10,
                        "exp" => f64::exp,
                        "abs" => f64::abs,
                        "round" => f64::round,
                        "floor" => f64::floor,
                        "ceil" => f64::ceil,
                        _ => return None,
                    };
                    Some(f(self.power()?))
                }
            },
            _ => None,
        }
    }
}

/// Format a number the way a calculator display would: grouped thousands,
/// at most 10 decimals, scientific notation for very large or tiny values.
pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if !(1e-6..1e15).contains(&a) {
        return format!("{v:e}");
    }
    let mut s = format!("{v:.10}");
    if s.contains('.') {
        s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let (sign, rest) = s.strip_prefix('-').map_or(("", s.as_str()), |r| ("-", r));
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    let mut grouped = String::new();
    for (n, c) in int.chars().enumerate() {
        if n > 0 && (int.len() - n) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Dim {
    Length,
    Mass,
    Volume,
    Time,
    Data,
    Speed,
    Area,
    Temp,
}

struct Unit {
    names: &'static [&'static str],
    label: &'static str,
    dim: Dim,
    /// Multiply by this to get the dimension's base unit (temperature: unused).
    factor: f64,
    /// Default unit to convert to when the query names no target.
    pair: &'static str,
}

const UNITS: &[Unit] = &[
    Unit {
        names: &[
            "mm",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
        ],
        label: "mm",
        dim: Dim::Length,
        factor: 0.001,
        pair: "in",
    },
    Unit {
        names: &[
            "cm",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
        ],
        label: "cm",
        dim: Dim::Length,
        factor: 0.01,
        pair: "in",
    },
    Unit {
        names: &["m", "meter", "meters", "metre", "metres"],
        label: "m",
        dim: Dim::Length,
        factor: 1.0,
        pair: "ft",
    },
    Unit {
        names: &["km", "kilometer", "kilometers", "kilometre", "kilometres"],
        label: "km",
        dim: Dim::Length,
        factor: 1000.0,
        pair: "mi",
    },
    Unit {
        names: &["in", "inch", "inches", "\""],
        label: "in",
        dim: Dim::Length,
        factor: 0.0254,
        pair: "cm",
    },
    Unit {
        names: &["ft", "foot", "feet", "'"],
        label: "ft",
        dim: Dim::Length,
        factor: 0.3048,
        pair: "m",
    },
    Unit {
        names: &["yd", "yard", "yards"],
        label: "yd",
        dim: Dim::Length,
        factor: 0.9144,
        pair: "m",
    },
    Unit {
        names: &["mi", "mile", "miles"],
        label: "mi",
        dim: Dim::Length,
        factor: 1609.344,
        pair: "km",
    },
    Unit {
        names: &["mg", "milligram", "milligrams"],
        label: "mg",
        dim: Dim::Mass,
        factor: 0.000_001,
        pair: "oz",
    },
    Unit {
        names: &["g", "gram", "grams"],
        label: "g",
        dim: Dim::Mass,
        factor: 0.001,
        pair: "oz",
    },
    Unit {
        names: &["kg", "kilo", "kilos", "kilogram", "kilograms"],
        label: "kg",
        dim: Dim::Mass,
        factor: 1.0,
        pair: "lb",
    },
    Unit {
        names: &["t", "tonne", "tonnes"],
        label: "t",
        dim: Dim::Mass,
        factor: 1000.0,
        pair: "lb",
    },
    Unit {
        names: &["oz", "ounce", "ounces"],
        label: "oz",
        dim: Dim::Mass,
        factor: 0.028_349_523_125,
        pair: "g",
    },
    Unit {
        names: &["lb", "lbs", "pound", "pounds"],
        label: "lb",
        dim: Dim::Mass,
        factor: 0.453_592_37,
        pair: "kg",
    },
    Unit {
        names: &["st", "stone", "stones"],
        label: "st",
        dim: Dim::Mass,
        factor: 6.350_293_18,
        pair: "kg",
    },
    Unit {
        names: &[
            "ml",
            "milliliter",
            "milliliters",
            "millilitre",
            "millilitres",
        ],
        label: "ml",
        dim: Dim::Volume,
        factor: 0.001,
        pair: "fl oz",
    },
    Unit {
        names: &["l", "liter", "liters", "litre", "litres"],
        label: "L",
        dim: Dim::Volume,
        factor: 1.0,
        pair: "gal",
    },
    Unit {
        names: &["gal", "gallon", "gallons"],
        label: "gal",
        dim: Dim::Volume,
        factor: 3.785_411_784,
        pair: "l",
    },
    Unit {
        names: &["qt", "quart", "quarts"],
        label: "qt",
        dim: Dim::Volume,
        factor: 0.946_352_946,
        pair: "l",
    },
    Unit {
        names: &["pt", "pint", "pints"],
        label: "pt",
        dim: Dim::Volume,
        factor: 0.473_176_473,
        pair: "ml",
    },
    Unit {
        names: &["cup", "cups"],
        label: "cup",
        dim: Dim::Volume,
        factor: 0.236_588_236_5,
        pair: "ml",
    },
    Unit {
        names: &["fl oz", "floz", "fluid ounce", "fluid ounces"],
        label: "fl oz",
        dim: Dim::Volume,
        factor: 0.029_573_529_562_5,
        pair: "ml",
    },
    Unit {
        names: &["ms", "millisecond", "milliseconds"],
        label: "ms",
        dim: Dim::Time,
        factor: 0.001,
        pair: "s",
    },
    Unit {
        names: &["s", "sec", "secs", "second", "seconds"],
        label: "s",
        dim: Dim::Time,
        factor: 1.0,
        pair: "min",
    },
    Unit {
        names: &["min", "mins", "minute", "minutes"],
        label: "min",
        dim: Dim::Time,
        factor: 60.0,
        pair: "s",
    },
    Unit {
        names: &["h", "hr", "hrs", "hour", "hours"],
        label: "h",
        dim: Dim::Time,
        factor: 3600.0,
        pair: "min",
    },
    Unit {
        names: &["d", "day", "days"],
        label: "days",
        dim: Dim::Time,
        factor: 86_400.0,
        pair: "h",
    },
    Unit {
        names: &["wk", "week", "weeks"],
        label: "weeks",
        dim: Dim::Time,
        factor: 604_800.0,
        pair: "days",
    },
    Unit {
        names: &["yr", "year", "years"],
        label: "years",
        dim: Dim::Time,
        factor: 31_557_600.0,
        pair: "days",
    },
    Unit {
        names: &["b", "byte", "bytes"],
        label: "B",
        dim: Dim::Data,
        factor: 1.0,
        pair: "kb",
    },
    Unit {
        names: &["kb", "kilobyte", "kilobytes"],
        label: "KB",
        dim: Dim::Data,
        factor: 1e3,
        pair: "b",
    },
    Unit {
        names: &["mb", "megabyte", "megabytes"],
        label: "MB",
        dim: Dim::Data,
        factor: 1e6,
        pair: "kb",
    },
    Unit {
        names: &["gb", "gigabyte", "gigabytes"],
        label: "GB",
        dim: Dim::Data,
        factor: 1e9,
        pair: "mb",
    },
    Unit {
        names: &["tb", "terabyte", "terabytes"],
        label: "TB",
        dim: Dim::Data,
        factor: 1e12,
        pair: "gb",
    },
    Unit {
        names: &["kmh", "km/h", "kph"],
        label: "km/h",
        dim: Dim::Speed,
        factor: 1.0 / 3.6,
        pair: "mph",
    },
    Unit {
        names: &["mph"],
        label: "mph",
        dim: Dim::Speed,
        factor: 0.447_04,
        pair: "km/h",
    },
    Unit {
        names: &["m/s", "mps"],
        label: "m/s",
        dim: Dim::Speed,
        factor: 1.0,
        pair: "km/h",
    },
    Unit {
        names: &["knot", "knots", "kn"],
        label: "knots",
        dim: Dim::Speed,
        factor: 0.514_444,
        pair: "km/h",
    },
    Unit {
        names: &["m2", "sqm", "square meter", "square meters"],
        label: "m²",
        dim: Dim::Area,
        factor: 1.0,
        pair: "sqft",
    },
    Unit {
        names: &["sqft", "ft2", "square foot", "square feet"],
        label: "sq ft",
        dim: Dim::Area,
        factor: 0.092_903_04,
        pair: "m2",
    },
    Unit {
        names: &["km2", "sqkm", "square kilometer", "square kilometers"],
        label: "km²",
        dim: Dim::Area,
        factor: 1e6,
        pair: "sqmi",
    },
    Unit {
        names: &["sqmi", "mi2", "square mile", "square miles"],
        label: "sq mi",
        dim: Dim::Area,
        factor: 2_589_988.11,
        pair: "km2",
    },
    Unit {
        names: &["ha", "hectare", "hectares"],
        label: "ha",
        dim: Dim::Area,
        factor: 10_000.0,
        pair: "acre",
    },
    Unit {
        names: &["acre", "acres", "ac"],
        label: "acres",
        dim: Dim::Area,
        factor: 4_046.856_422_4,
        pair: "ha",
    },
    Unit {
        names: &["c", "°c", "celsius", "degc"],
        label: "°C",
        dim: Dim::Temp,
        factor: 0.0,
        pair: "f",
    },
    Unit {
        names: &["f", "°f", "fahrenheit", "degf"],
        label: "°F",
        dim: Dim::Temp,
        factor: 0.0,
        pair: "c",
    },
    Unit {
        names: &["k", "kelvin"],
        label: "K",
        dim: Dim::Temp,
        factor: 0.0,
        pair: "c",
    },
];

fn unit(name: &str) -> Option<&'static Unit> {
    UNITS.iter().find(|u| u.names.contains(&name))
}

fn to_kelvin(label: &str, v: f64) -> f64 {
    match label {
        "°C" => v + 273.15,
        "°F" => (v - 32.0) * 5.0 / 9.0 + 273.15,
        _ => v,
    }
}

fn from_kelvin(label: &str, k: f64) -> f64 {
    match label {
        "°C" => k - 273.15,
        "°F" => (k - 273.15) * 9.0 / 5.0 + 32.0,
        _ => k,
    }
}

/// Common currency codes, recognised before the live rate table has loaded.
const CURRENCIES: &[&str] = &[
    "usd", "eur", "gbp", "jpy", "cny", "chf", "cad", "aud", "nzd", "brl", "inr", "mxn", "sek",
    "nok", "dkk", "pln", "czk", "huf", "krw", "sgd", "hkd", "zar", "try", "ils", "aed", "thb",
];

fn currency_code(word: &str, rates: &HashMap<String, f64>) -> Option<String> {
    let code = match word {
        "$" | "dollar" | "dollars" => "usd",
        "€" | "euro" | "euros" => "eur",
        "£" | "pound sterling" => "gbp",
        "¥" | "yen" => "jpy",
        "real" | "reais" => "brl",
        w => w,
    };
    let upper = code.to_uppercase();
    (code.len() == 3 && (CURRENCIES.contains(&code) || rates.contains_key(&upper))).then_some(upper)
}

pub enum Conversion {
    Done {
        result: String,
        detail: String,
    },
    /// A currency conversion that needs the live rate table first.
    NeedsRates,
}

/// Split "12.5 km to mi", "$20 in eur", "100f" into (value, from, optional to).
fn split_query(input: &str) -> Option<(f64, String, Option<String>)> {
    let q = input.trim().to_lowercase();
    // A currency symbol may lead the number: "$20".
    let (sym, q) = match q.chars().next()? {
        c @ ('$' | '€' | '£' | '¥') => (
            Some(c.to_string()),
            q[c.len_utf8()..].trim_start().to_string(),
        ),
        _ => (None, q),
    };
    let num_end = q
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || *c == '.' || *c == ',' || (*i == 0 && *c == '-')))
        .map_or(q.len(), |(i, _)| i);
    let value: f64 = q[..num_end].replace(',', "").parse().ok()?;
    let rest = q[num_end..].trim();
    let (from, to) = [" to ", " in ", " as ", " = ", " into "]
        .iter()
        .find_map(|sep| {
            rest.split_once(sep)
                .map(|(a, b)| (a.trim().to_string(), Some(b.trim().to_string())))
        })
        .or_else(|| {
            // "$20 to eur" leaves rest = "to eur".
            ["to ", "in ", "into "].iter().find_map(|sep| {
                rest.strip_prefix(sep)
                    .map(|b| (String::new(), Some(b.trim().to_string())))
            })
        })
        .unwrap_or((rest.to_string(), None));
    let from = match sym {
        Some(s) if from.is_empty() => s,
        Some(_) => return None,
        None => from,
    };
    (!from.is_empty()).then_some((value, from, to))
}

/// Convert units or currency. `rates` holds units per US dollar (may be empty).
pub fn convert(input: &str, rates: &HashMap<String, f64>) -> Option<Conversion> {
    let (value, from, to) = split_query(input)?;

    if let Some(src) = currency_code(&from, rates) {
        let dst = match &to {
            Some(t) => currency_code(t, rates)?,
            None if src == "USD" => "EUR".to_string(),
            None => "USD".to_string(),
        };
        if rates.is_empty() {
            return Some(Conversion::NeedsRates);
        }
        let (a, b) = (rates.get(&src)?, rates.get(&dst)?);
        let out = value / a * b;
        let one = b / a;
        return Some(Conversion::Done {
            result: format!("{} {dst}", format_money(out)),
            detail: format!(
                "{} {src} = {} {dst}   ·   1 {src} = {} {dst}",
                format_number(value),
                format_money(out),
                format_number((one * 10_000.0).round() / 10_000.0)
            ),
        });
    }

    let src = unit(&from)?;
    let dst = unit(to.as_deref().unwrap_or(src.pair))?;
    if src.dim != dst.dim {
        return None;
    }
    let out = if src.dim == Dim::Temp {
        from_kelvin(dst.label, to_kelvin(src.label, value))
    } else {
        value * src.factor / dst.factor
    };
    let rounded = (out * 1e6).round() / 1e6;
    Some(Conversion::Done {
        result: format!("{} {}", format_number(rounded), dst.label),
        detail: format!(
            "{} {} = {} {}",
            format_number(value),
            src.label,
            format_number(rounded),
            dst.label
        ),
    })
}

fn format_money(v: f64) -> String {
    let s = format_number((v * 100.0).round() / 100.0);
    match s.split_once('.') {
        Some((_, f)) if f.len() == 1 => format!("{s}0"),
        Some(_) => s,
        None => format!("{s}.00"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_and_conversions() {
        assert_eq!(calculate("2+3*4"), Some(14.0));
        assert_eq!(calculate("(2+3)*4"), Some(20.0));
        assert_eq!(calculate("-2^2"), Some(-4.0));
        assert_eq!(calculate("2^3^2"), Some(512.0));
        assert_eq!(calculate("sqrt(16)+1"), Some(5.0));
        assert_eq!(calculate("1,000 / 4"), Some(250.0));
        assert!((calculate("2pi").unwrap() - std::f64::consts::TAU).abs() < 1e-12);
        assert_eq!(calculate("42"), None);
        assert_eq!(calculate("firefox"), None);
        assert_eq!(calculate("1/0"), None);
        assert_eq!(calculate("2+"), None);
        assert_eq!(format_number(1234567.5), "1,234,567.5");
        assert_eq!(format_number(-1000.0), "-1,000");
        assert_eq!(format_number(0.1 + 0.2), "0.3");

        let none = HashMap::new();
        let done = |q: &str, r: &HashMap<String, f64>| match convert(q, r) {
            Some(Conversion::Done { result, .. }) => result,
            _ => String::from("-"),
        };
        assert_eq!(done("10 km to mi", &none), "6.213712 mi");
        assert_eq!(done("100 c in f", &none), "212 °F");
        assert_eq!(done("32f", &none), "0 °C");
        assert_eq!(done("2 kg", &none), "4.409245 lb");
        assert_eq!(done("5 kg to km", &none), "-");
        assert!(matches!(
            convert("10 usd to eur", &none),
            Some(Conversion::NeedsRates)
        ));
        let rates = HashMap::from([("USD".to_string(), 1.0), ("EUR".to_string(), 0.5)]);
        assert_eq!(done("10 usd to eur", &rates), "5.00 EUR");
        assert_eq!(done("$10 to eur", &rates), "5.00 EUR");
        assert_eq!(done("€3", &rates), "6.00 USD");
        assert!(convert("hello world", &none).is_none());
    }
}
