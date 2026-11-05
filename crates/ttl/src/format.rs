//! `sprintf` / `sprintf2` の書式（C の printf の部分集合：c d i o u x X s と % の後のフラグ・幅・精度）。

use crate::Value;

pub(crate) fn sprintf(fmt: &str, args: &[Value]) -> Result<String, String> {
    let mut out = String::new();
    let mut args = args.iter();
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }
        let mut left = false;
        let mut zero = false;
        let mut plus = false;
        let mut space = false;
        let mut alt = false;
        while let Some(&f) = chars.peek() {
            match f {
                '-' => left = true,
                '0' => zero = true,
                '+' => plus = true,
                ' ' => space = true,
                '#' => alt = true,
                _ => break,
            }
            chars.next();
        }
        let mut width = String::new();
        while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
            width.push(d);
            chars.next();
        }
        let mut precision: Option<usize> = None;
        if chars.peek() == Some(&'.') {
            chars.next();
            let mut p = String::new();
            while let Some(&d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                p.push(d);
                chars.next();
            }
            precision = Some(p.parse().unwrap_or(0));
        }
        let width: usize = width.parse().unwrap_or(0);
        let conv = chars.next().ok_or("incomplete format")?;
        let arg = args.next().ok_or("not enough arguments for format")?;
        let int = || match arg {
            Value::Int(n) => Ok(*n),
            Value::Str(_) => Err(format!("%{conv} needs an integer")),
        };
        // (前置き（符号や 0x）, 本体, 0 で埋めてよいか)
        let (prefix, body, numeric): (String, String, bool) = match conv {
            'd' | 'i' => {
                let n = int()?;
                let mut digits = n.unsigned_abs().to_string();
                if let Some(p) = precision {
                    while digits.len() < p {
                        digits.insert(0, '0');
                    }
                }
                let sign = if n < 0 {
                    "-"
                } else if plus {
                    "+"
                } else if space {
                    " "
                } else {
                    ""
                };
                (sign.into(), digits, true)
            }
            'u' => (String::new(), (int()? as u32).to_string(), true),
            'x' => (
                (if alt { "0x" } else { "" }).into(),
                format!("{:x}", int()? as u32),
                true,
            ),
            'X' => (
                (if alt { "0X" } else { "" }).into(),
                format!("{:X}", int()? as u32),
                true,
            ),
            'o' => (
                (if alt { "0" } else { "" }).into(),
                format!("{:o}", int()? as u32),
                true,
            ),
            'c' => (
                String::new(),
                char::from_u32(int()? as u32)
                    .map(String::from)
                    .unwrap_or_default(),
                false,
            ),
            's' => {
                let s = match arg {
                    Value::Str(s) => s.clone(),
                    Value::Int(n) => n.to_string(),
                };
                let s = match precision {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                };
                (String::new(), s, false)
            }
            other => return Err(format!("unsupported format %{other}")),
        };
        let len = prefix.chars().count() + body.chars().count();
        let pad = width.saturating_sub(len);
        if left {
            out.push_str(&prefix);
            out.push_str(&body);
            out.extend(std::iter::repeat_n(' ', pad));
        } else if zero && numeric && precision.is_none() {
            out.push_str(&prefix);
            out.extend(std::iter::repeat_n('0', pad));
            out.push_str(&body);
        } else {
            out.extend(std::iter::repeat_n(' ', pad));
            out.push_str(&prefix);
            out.push_str(&body);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Value::{Int, Str};

    #[test]
    fn formats() {
        let f = |s: &str, a: &[Value]| sprintf(s, a).unwrap();
        assert_eq!(
            f("%d-%5d|%-4d|%05d", &[Int(1), Int(-2), Int(3), Int(-4)]),
            "1-   -2|3   |-0004"
        );
        assert_eq!(
            f(
                "%x %X %#x %08X %o",
                &[Int(255), Int(255), Int(255), Int(-1), Int(8)]
            ),
            "ff FF 0xff FFFFFFFF 10"
        );
        assert_eq!(
            f(
                "[%s][%5s][%-3s][%.2s] %c%%",
                &[
                    Str("a".into()),
                    Str("b".into()),
                    Str("c".into()),
                    Str("xyz".into()),
                    Int(65)
                ]
            ),
            "[a][    b][c  ][xy] A%"
        );
        assert!(sprintf("%d", &[Str("x".into())]).is_err());
        assert!(sprintf("%d %d", &[Int(1)]).is_err());
    }
}
