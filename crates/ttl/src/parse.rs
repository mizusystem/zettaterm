//! TTL の字句解析と構文解析。1 行 1 文で、ブロック（if / while / for など）の対応はここで解く。

use std::collections::HashMap;
use std::fmt;

/// 構文エラー（行番号つき）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
    BitNot,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Mul,
    Div,
    Mod,
    Add,
    Sub,
    Shr,
    Shl,
    UShr,
    BitAnd,
    BitXor,
    BitOr,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Int(i32),
    Str(String),
    Var(String),
    Index(String, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
}

/// do / loop の条件。`true` なら while、`false` なら until。
pub type LoopCond = Option<(bool, Expr)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Label(String),
    Assign {
        name: String,
        index: Option<Expr>,
        value: Expr,
    },
    Command {
        name: String,
        args: Vec<Expr>,
    },
    /// `then` が None ならブロックの if（endif まで）
    If {
        cond: Expr,
        then: Option<Box<Stmt>>,
    },
    ElseIf(Expr),
    Else,
    EndIf,
    While(Expr),
    EndWhile,
    Until(Expr),
    EndUntil,
    Do(LoopCond),
    Loop(LoopCond),
    For {
        var: String,
        start: Expr,
        end: Expr,
    },
    Next,
    Break,
    Continue,
    Goto(String),
    Call(String),
    Return,
    End,
    Exit,
    Include(Expr),
}

#[derive(Debug, Clone)]
pub struct Line {
    pub no: usize,
    pub stmt: Stmt,
}

/// 解析済みのマクロ 1 ファイル分。
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub lines: Vec<Line>,
    /// ラベル名（小文字）→ 行の添字
    pub labels: HashMap<String, usize>,
    /// if / elseif の次の節、ループの開始⇔終了
    pub(crate) next: Vec<usize>,
    pub(crate) end: Vec<usize>,
    /// break / continue が属するループの開始行
    pub(crate) owner: Vec<usize>,
}

// ---------------------------------------------------------------- 字句解析

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Int(i32),
    Str(String),
    Op(&'static str),
    LBracket,
    RBracket,
    LParen,
    RParen,
}

const OPS: &[&str] = &[
    ">>>", "<<", ">>", "<=", ">=", "<>", "!=", "==", "&&", "||", "+", "-", "*", "/", "%", "&", "|",
    "^", "~", "!", "=", "<", ">",
];

/// `/* ... */` を取り除く（文字列の中は残す）。改行は残して行番号を保つ。
fn strip_block_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut quote: Option<char> = None;
    let mut in_comment = false;
    let mut line_comment = false;
    while let Some(c) = chars.next() {
        if in_comment {
            if c == '*' && chars.peek() == Some(&'/') {
                chars.next();
                in_comment = false;
            } else if c == '\n' {
                out.push('\n');
            }
            continue;
        }
        if c == '\n' {
            quote = None;
            line_comment = false;
            out.push(c);
            continue;
        }
        if line_comment {
            out.push(c);
            continue;
        }
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                out.push(c);
            }
            None => {
                if c == '/' && chars.peek() == Some(&'*') {
                    chars.next();
                    in_comment = true;
                } else {
                    if c == '\'' || c == '"' {
                        quote = Some(c);
                    } else if c == ';' {
                        line_comment = true;
                    }
                    out.push(c);
                }
            }
        }
    }
    out
}

fn lex(line: &str, no: usize) -> Result<Vec<Tok>, ParseError> {
    let err = |m: String| ParseError {
        line: no,
        message: m,
    };
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut toks = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c == ';' {
            break;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // 文字列：'..' "..." #13 #$0d が隙間なく続く間はひとつの文字列
        if c == '\'' || c == '"' || c == '#' {
            let mut s = String::new();
            while i < chars.len() {
                let c = chars[i];
                if c == '\'' || c == '"' {
                    let end = chars[i + 1..]
                        .iter()
                        .position(|&d| d == c)
                        .ok_or_else(|| err("unterminated string".into()))?;
                    s.extend(&chars[i + 1..i + 1 + end]);
                    i += end + 2;
                } else if c == '#' {
                    i += 1;
                    let (n, used) = lex_number(&chars[i..])
                        .ok_or_else(|| err("bad character code after #".into()))?;
                    i += used;
                    let ch = char::from_u32(n as u32)
                        .ok_or_else(|| err(format!("bad character code #{n}")))?;
                    s.push(ch);
                } else {
                    break;
                }
            }
            toks.push(Tok::Str(s));
            continue;
        }
        if c.is_ascii_digit() || c == '$' {
            let (n, used) = lex_number(&chars[i..]).ok_or_else(|| err("bad number".into()))?;
            toks.push(Tok::Int(n));
            i += used;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let op = match word.to_ascii_lowercase().as_str() {
                "and" => Some("&"),
                "or" => Some("|"),
                "xor" => Some("^"),
                "not" => Some("~"),
                _ => None,
            };
            toks.push(match op {
                Some(o) => Tok::Op(o),
                None => Tok::Ident(word),
            });
            continue;
        }
        match c {
            '[' => toks.push(Tok::LBracket),
            ']' => toks.push(Tok::RBracket),
            '(' => toks.push(Tok::LParen),
            ')' => toks.push(Tok::RParen),
            _ => {
                let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
                let op = OPS
                    .iter()
                    .find(|o| rest.starts_with(**o))
                    .ok_or_else(|| err(format!("unexpected character '{c}'")))?;
                toks.push(Tok::Op(op));
                i += op.len();
                continue;
            }
        }
        i += 1;
    }
    Ok(toks)
}

/// 10 進か `$` 付きの 16 進。値と使った文字数。
fn lex_number(chars: &[char]) -> Option<(i32, usize)> {
    let (radix, skip) = if chars.first() == Some(&'$') {
        (16, 1)
    } else {
        (10, 0)
    };
    let digits: String = chars[skip..]
        .iter()
        .take_while(|c| c.is_digit(radix))
        .collect();
    if digits.is_empty() {
        return None;
    }
    // TTL の整数は 32 ビット。大きな 16 進は符号付きに折り返す
    let n = i64::from_str_radix(&digits, radix).ok()?;
    Some((n as u32 as i32, skip + digits.len()))
}

// ---------------------------------------------------------------- 構文解析

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
    line: usize,
}

impl Parser {
    fn err(&self, m: impl Into<String>) -> ParseError {
        ParseError {
            line: self.line,
            message: m.into(),
        }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn ident(&mut self) -> Result<String, ParseError> {
        match self.next() {
            Some(Tok::Ident(s)) => Ok(s),
            _ => Err(self.err("name expected")),
        }
    }

    fn keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if s.eq_ignore_ascii_case(word))
    }

    fn expect_end(&self) -> Result<(), ParseError> {
        if self.at_end() {
            Ok(())
        } else {
            Err(self.err("unexpected text at end of line"))
        }
    }

    fn expr(&mut self) -> Result<Expr, ParseError> {
        self.binary(0)
    }

    fn binary(&mut self, level: usize) -> Result<Expr, ParseError> {
        const LEVELS: &[&[(&str, BinOp)]] = &[
            &[("||", BinOp::Or)],
            &[("&&", BinOp::And)],
            &[
                ("=", BinOp::Eq),
                ("==", BinOp::Eq),
                ("<>", BinOp::Ne),
                ("!=", BinOp::Ne),
            ],
            &[
                ("<", BinOp::Lt),
                (">", BinOp::Gt),
                ("<=", BinOp::Le),
                (">=", BinOp::Ge),
            ],
            &[("|", BinOp::BitOr)],
            &[("^", BinOp::BitXor)],
            &[("&", BinOp::BitAnd)],
            &[(">>", BinOp::Shr), ("<<", BinOp::Shl), (">>>", BinOp::UShr)],
            &[("+", BinOp::Add), ("-", BinOp::Sub)],
            &[("*", BinOp::Mul), ("/", BinOp::Div), ("%", BinOp::Mod)],
        ];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        loop {
            let op = match self.peek() {
                Some(Tok::Op(o)) => LEVELS[level].iter().find(|(s, _)| s == o).map(|x| x.1),
                _ => None,
            };
            let Some(op) = op else { return Ok(lhs) };
            self.pos += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        let op = match self.peek() {
            Some(Tok::Op("-")) => Some(UnOp::Neg),
            Some(Tok::Op("+")) => Some(UnOp::Plus),
            Some(Tok::Op("~")) => Some(UnOp::BitNot),
            Some(Tok::Op("!")) => Some(UnOp::Not),
            _ => None,
        };
        if let Some(op) = op {
            self.pos += 1;
            let e = self.unary()?;
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.next() {
            Some(Tok::Int(n)) => Ok(Expr::Int(n)),
            Some(Tok::Str(s)) => Ok(Expr::Str(s)),
            Some(Tok::Ident(name)) => {
                let name = name.to_ascii_lowercase();
                if self.peek() == Some(&Tok::LBracket) {
                    self.pos += 1;
                    let i = self.expr()?;
                    if self.next() != Some(Tok::RBracket) {
                        return Err(self.err("']' expected"));
                    }
                    Ok(Expr::Index(name, Box::new(i)))
                } else {
                    Ok(Expr::Var(name))
                }
            }
            Some(Tok::LParen) => {
                let e = self.expr()?;
                if self.next() != Some(Tok::RParen) {
                    return Err(self.err("')' expected"));
                }
                Ok(e)
            }
            _ => Err(self.err("expression expected")),
        }
    }

    fn loop_cond(&mut self) -> Result<LoopCond, ParseError> {
        if self.at_end() {
            return Ok(None);
        }
        let is_while = if self.keyword("while") {
            true
        } else if self.keyword("until") {
            false
        } else {
            return Err(self.err("'while' or 'until' expected"));
        };
        self.pos += 1;
        let e = self.expr()?;
        Ok(Some((is_while, e)))
    }

    fn stmt(&mut self) -> Result<Stmt, ParseError> {
        let first = match self.peek() {
            Some(Tok::Ident(s)) => s.to_ascii_lowercase(),
            _ => return Err(self.err("command expected")),
        };
        // 代入：name = expr / name[i] = expr
        if matches!(
            self.toks.get(self.pos + 1),
            Some(Tok::Op("=")) | Some(Tok::LBracket)
        ) {
            self.pos += 1;
            let index = if self.peek() == Some(&Tok::LBracket) {
                self.pos += 1;
                let i = self.expr()?;
                if self.next() != Some(Tok::RBracket) {
                    return Err(self.err("']' expected"));
                }
                Some(i)
            } else {
                None
            };
            if self.next() != Some(Tok::Op("=")) {
                return Err(self.err("'=' expected"));
            }
            let value = self.expr()?;
            self.expect_end()?;
            return Ok(Stmt::Assign {
                name: first,
                index,
                value,
            });
        }
        self.pos += 1;
        let s = match first.as_str() {
            "if" => {
                let cond = self.expr()?;
                if self.keyword("then") {
                    self.pos += 1;
                    self.expect_end()?;
                    Stmt::If { cond, then: None }
                } else {
                    if self.at_end() {
                        return Err(self.err("'then' or a command expected"));
                    }
                    let inner = self.stmt()?;
                    if matches!(
                        inner,
                        Stmt::If { then: None, .. }
                            | Stmt::While(_)
                            | Stmt::For { .. }
                            | Stmt::Do(_)
                            | Stmt::Until(_)
                    ) {
                        return Err(self.err("a block cannot follow a one-line if"));
                    }
                    return Ok(Stmt::If {
                        cond,
                        then: Some(Box::new(inner)),
                    });
                }
            }
            "elseif" => {
                let cond = self.expr()?;
                if self.keyword("then") {
                    self.pos += 1;
                }
                Stmt::ElseIf(cond)
            }
            "else" => Stmt::Else,
            "endif" => Stmt::EndIf,
            "while" => Stmt::While(self.expr()?),
            "endwhile" => Stmt::EndWhile,
            "until" => Stmt::Until(self.expr()?),
            "enduntil" => Stmt::EndUntil,
            "do" => Stmt::Do(self.loop_cond()?),
            "loop" => Stmt::Loop(self.loop_cond()?),
            "for" => {
                let var = self.ident()?.to_ascii_lowercase();
                let start = self.expr()?;
                let end = self.expr()?;
                Stmt::For { var, start, end }
            }
            "next" => Stmt::Next,
            "break" => Stmt::Break,
            "continue" => Stmt::Continue,
            "goto" => Stmt::Goto(self.ident()?.to_ascii_lowercase()),
            "call" => Stmt::Call(self.ident()?.to_ascii_lowercase()),
            "return" => Stmt::Return,
            "end" => Stmt::End,
            "exit" => Stmt::Exit,
            "include" => Stmt::Include(self.expr()?),
            _ => {
                let mut args = Vec::new();
                while !self.at_end() {
                    args.push(self.expr()?);
                }
                return Ok(Stmt::Command { name: first, args });
            }
        };
        self.expect_end()?;
        Ok(s)
    }
}

/// マクロのソースを解析する。
pub fn parse(src: &str) -> Result<Program, ParseError> {
    let src = strip_block_comments(src.strip_prefix('\u{feff}').unwrap_or(src));
    let mut prog = Program::default();
    for (i, raw) in src.lines().enumerate() {
        let no = i + 1;
        let text = raw.trim();
        if let Some(label) = text.strip_prefix(':') {
            let name: String = label
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                return Err(ParseError {
                    line: no,
                    message: "label name expected".into(),
                });
            }
            let name = name.to_ascii_lowercase();
            prog.labels.insert(name.clone(), prog.lines.len());
            prog.lines.push(Line {
                no,
                stmt: Stmt::Label(name),
            });
            continue;
        }
        let toks = lex(text, no)?;
        if toks.is_empty() {
            continue;
        }
        let mut p = Parser {
            toks,
            pos: 0,
            line: no,
        };
        let stmt = p.stmt()?;
        prog.lines.push(Line { no, stmt });
    }
    link_blocks(&mut prog)?;
    Ok(prog)
}

/// ブロックの対応を解く。
fn link_blocks(prog: &mut Program) -> Result<(), ParseError> {
    let n = prog.lines.len();
    prog.next = vec![usize::MAX; n];
    prog.end = vec![usize::MAX; n];
    prog.owner = vec![usize::MAX; n];
    // (開始行, 直前の節の行)
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let err = |line: usize, m: &str| ParseError {
        line,
        message: m.into(),
    };
    for i in 0..n {
        let no = prog.lines[i].no;
        let open = |s: &Stmt| -> Option<&'static str> {
            match s {
                Stmt::If { then: None, .. } => Some("if"),
                Stmt::While(_) => Some("while"),
                Stmt::Until(_) => Some("until"),
                Stmt::Do(_) => Some("do"),
                Stmt::For { .. } => Some("for"),
                _ => None,
            }
        };
        match &prog.lines[i].stmt {
            s if open(s).is_some() => stack.push((i, i)),
            Stmt::ElseIf(_) | Stmt::Else => {
                let Some(&mut (start, ref mut prev)) = stack.last_mut() else {
                    return Err(err(no, "else without if"));
                };
                if !matches!(prog.lines[start].stmt, Stmt::If { .. }) {
                    return Err(err(no, "else without if"));
                }
                if matches!(prog.lines[*prev].stmt, Stmt::Else) {
                    return Err(err(no, "clause after else"));
                }
                prog.next[*prev] = i;
                *prev = i;
            }
            Stmt::EndIf | Stmt::EndWhile | Stmt::EndUntil | Stmt::Loop(_) | Stmt::Next => {
                let want = match prog.lines[i].stmt {
                    Stmt::EndIf => "if",
                    Stmt::EndWhile => "while",
                    Stmt::EndUntil => "until",
                    Stmt::Loop(_) => "do",
                    _ => "for",
                };
                let Some((start, prev)) = stack.pop() else {
                    return Err(err(no, &format!("unexpected end of {want}")));
                };
                if open(&prog.lines[start].stmt) != Some(want) {
                    return Err(err(no, &format!("unexpected end of {want}")));
                }
                if want == "if" {
                    prog.next[prev] = i;
                    // 各節から endif へ
                    let mut j = start;
                    while j != i {
                        prog.end[j] = i;
                        j = prog.next[j];
                    }
                } else {
                    if matches!(prog.lines[start].stmt, Stmt::Do(Some(_)))
                        && matches!(prog.lines[i].stmt, Stmt::Loop(Some(_)))
                    {
                        return Err(err(no, "do and loop cannot both have a condition"));
                    }
                    prog.end[start] = i;
                    prog.end[i] = start;
                }
            }
            Stmt::Break | Stmt::Continue => {
                let owner = stack
                    .iter()
                    .rev()
                    .find(|(s, _)| !matches!(prog.lines[*s].stmt, Stmt::If { .. }))
                    .ok_or_else(|| err(no, "break or continue outside a loop"))?;
                prog.owner[i] = owner.0;
            }
            Stmt::If {
                then: Some(inner), ..
            } if matches!(**inner, Stmt::Break | Stmt::Continue) => {
                let owner = stack
                    .iter()
                    .rev()
                    .find(|(s, _)| !matches!(prog.lines[*s].stmt, Stmt::If { .. }))
                    .ok_or_else(|| err(no, "break or continue outside a loop"))?;
                prog.owner[i] = owner.0;
            }
            _ => {}
        }
    }
    if let Some((start, _)) = stack.pop() {
        return Err(err(prog.lines[start].no, "block is not closed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_and_char_codes() {
        let p = parse("send 'abc'#13#10\"d'e\" #$41").unwrap();
        assert_eq!(
            p.lines[0].stmt,
            Stmt::Command {
                name: "send".into(),
                args: vec![Expr::Str("abc\r\nd'e".into()), Expr::Str("A".into())],
            }
        );
    }

    #[test]
    fn precedence_and_assignment() {
        let p = parse("x = 1 + 2 * 3 = 7 && not 0").unwrap();
        let Stmt::Assign { value, .. } = &p.lines[0].stmt else {
            panic!()
        };
        assert!(matches!(value, Expr::Binary(BinOp::And, _, _)));
    }

    #[test]
    fn comments_and_labels() {
        let p = parse("; c\n/* a\nb */ :top\n:top2 ; x\nsendln 'a;b' ; c").unwrap();
        assert_eq!(p.labels["top"], 0);
        assert_eq!(p.labels["top2"], 1);
        assert_eq!(p.lines[2].no, 5);
        assert!(
            matches!(&p.lines[2].stmt, Stmt::Command { args, .. } if args[0] == Expr::Str("a;b".into()))
        );
    }

    #[test]
    fn blocks_link() {
        let p =
            parse("if a then\nx=1\nelseif b then\nx=2\nelse\nx=3\nendif\nwhile 1\nbreak\nendwhile")
                .unwrap();
        assert_eq!(p.next[0], 2);
        assert_eq!(p.next[2], 4);
        assert_eq!(p.next[4], 6);
        assert_eq!(p.end[0], 6);
        assert_eq!(p.end[7], 9);
        assert_eq!(p.owner[8], 7);
        assert!(parse("if 1 then\nx=1").is_err());
        assert!(parse("endwhile").is_err());
        assert!(parse("break").is_err());
    }
}
