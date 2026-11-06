//! TTL のインタプリタ。Session に対して send / wait を行い、接続や対話は Host に任せる。

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mt_core::{ByteDivert, LogOptions, ReceiveTap, Session, SessionState};
use mt_xfer::{Direction, Protocol, Transfer};
use regex::Regex;

use crate::connect::{ConnectSpec, parse_connect};
use crate::format::sprintf;
use crate::parse::{BinOp, Expr, Program, Stmt, UnOp, parse};

/// 変数の値。TTL は整数（32 ビット）と文字列だけ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i32),
    Str(String),
}

/// マクロの外の世界（接続、ダイアログ、画面）。cli と各 OS の UI が実装する。
pub trait Host {
    /// `connect` で接続を開く。
    fn connect(&mut self, spec: &ConnectSpec) -> Result<Arc<Session>, String>;

    /// `disconnect` / `closett`。
    fn disconnect(&mut self, session: &Arc<Session>) {
        session.close();
    }

    /// `messagebox`。
    fn message_box(&mut self, _title: &str, _text: &str) {}

    /// `inputbox` / `passwordbox`。None は取り消し。
    fn input_box(
        &mut self,
        _title: &str,
        _text: &str,
        _default: &str,
        _password: bool,
    ) -> Option<String> {
        None
    }

    /// `yesnobox`。
    fn yes_no_box(&mut self, _title: &str, _text: &str) -> bool {
        false
    }

    /// `statusbox`（None は `closesbox`）。
    fn status_box(&mut self, _title: &str, _text: Option<&str>) {}

    /// `dispstr`。端末の画面にだけ出す。
    fn display(&mut self, _text: &str) {}

    /// `settitle`。
    fn set_title(&mut self, _title: &str) {}

    fn beep(&mut self) {}

    /// `ymodemrecv` などの受け取り先フォルダ。
    fn download_dir(&self) -> PathBuf {
        PathBuf::from(".")
    }

    /// ファイル転送が始まった・終わった（進み具合の表示用）。
    fn transfer_started(&mut self, _name: &str, _send: bool) {}
    fn transfer_finished(&mut self, _ok: bool) {}
}

/// マクロの失敗（ファイルと行つき）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroError {
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for MacroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}

impl std::error::Error for MacroError {}

/// どう終わったか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 最後まで、または `end` / `exit` で
    Finished,
    /// 外から止められた
    Stopped,
}

enum Fail {
    Msg(String),
    Stopped,
}

impl From<String> for Fail {
    fn from(s: String) -> Self {
        Fail::Msg(s)
    }
}

impl From<&str> for Fail {
    fn from(s: &str) -> Self {
        Fail::Msg(s.to_owned())
    }
}

type R<T> = Result<T, Fail>;

enum Flow {
    Next,
    Jump(usize),
    End,
    Exit,
    Include(PathBuf),
}

struct Frame {
    prog: Rc<Program>,
    file: PathBuf,
    pc: usize,
    calls: Vec<usize>,
    /// for の行 → (終わりの値, 増分)
    fors: HashMap<usize, (i32, i32)>,
}

struct Handle {
    file: File,
}

/// 受信バッファの上限（超えたら古い方を捨てる）
const RECV_LIMIT: usize = 1 << 20;
const MAX_INCLUDE: usize = 9;

pub struct Interp<'h> {
    host: &'h mut dyn Host,
    vars: HashMap<String, Value>,
    arrays: HashMap<String, Vec<Value>>,
    frames: Vec<Frame>,
    session: Option<Arc<Session>>,
    tap: Option<ReceiveTap>,
    recv: String,
    files: HashMap<i32, Handle>,
    next_handle: i32,
    stop: Arc<AtomicBool>,
    base_dir: PathBuf,
    dtr: bool,
    rts: bool,
    title: String,
    rng: u64,
}

impl<'h> Interp<'h> {
    /// `session` は既に開いている接続（UI のウィンドウから実行するとき）。cli では None から `connect` する。
    pub fn new(
        host: &'h mut dyn Host,
        session: Option<Arc<Session>>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let tap = session.as_ref().map(|s| s.tap());
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1)
            | 1;
        let mut me = Self {
            host,
            vars: HashMap::new(),
            arrays: HashMap::new(),
            frames: Vec::new(),
            session,
            tap,
            recv: String::new(),
            files: HashMap::new(),
            next_handle: 1,
            stop,
            base_dir: PathBuf::from("."),
            dtr: true,
            rts: true,
            title: String::new(),
            rng: seed,
        };
        for (k, v) in [
            ("result", Value::Int(0)),
            ("timeout", Value::Int(0)),
            ("mtimeout", Value::Int(0)),
            ("inputstr", Value::Str(String::new())),
            ("matchstr", Value::Str(String::new())),
        ] {
            me.vars.insert(k.into(), v);
        }
        for i in 1..=9 {
            me.vars
                .insert(format!("groupmatchstr{i}"), Value::Str(String::new()));
        }
        me
    }

    /// 接続（マクロが `connect` で開いたものを含む）。
    pub fn session(&self) -> Option<&Arc<Session>> {
        self.session.as_ref()
    }

    /// ファイルのマクロを実行する。`params` は param2 以降（param1 はマクロのパス）。
    pub fn run_file(&mut self, path: &Path, params: &[String]) -> Result<Outcome, MacroError> {
        let err = |line, message| MacroError {
            file: path.display().to_string(),
            line,
            message,
        };
        let src = std::fs::read(path).map_err(|e| err(0, e.to_string()))?;
        let src = String::from_utf8_lossy(&src);
        let prog = parse(&src).map_err(|e| err(e.line, e.message))?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            self.base_dir = dir.to_owned();
        }
        let mut all = vec![path.display().to_string()];
        all.extend(params.iter().cloned());
        self.set_params(&all);
        self.run_program(prog, path.to_owned())
    }

    /// ソースを直接実行する（テスト用）。相対パスは `base_dir` から。
    pub fn run_source(&mut self, src: &str, base_dir: &Path) -> Result<Outcome, MacroError> {
        let prog = parse(src).map_err(|e| MacroError {
            file: "<macro>".into(),
            line: e.line,
            message: e.message,
        })?;
        self.base_dir = base_dir.to_owned();
        self.set_params(&["<macro>".to_owned()]);
        self.run_program(prog, PathBuf::from("<macro>"))
    }

    fn set_params(&mut self, all: &[String]) {
        self.vars
            .insert("paramcnt".into(), Value::Int(all.len() as i32));
        for i in 1..=9 {
            let v = all.get(i - 1).cloned().unwrap_or_default();
            self.vars.insert(format!("param{i}"), Value::Str(v));
        }
        let mut params = vec![Value::Str(String::new())];
        params.extend(all.iter().cloned().map(Value::Str));
        self.arrays.insert("params".into(), params);
    }

    fn run_program(&mut self, prog: Program, file: PathBuf) -> Result<Outcome, MacroError> {
        self.frames.push(Frame {
            prog: Rc::new(prog),
            file,
            pc: 0,
            calls: Vec::new(),
            fors: HashMap::new(),
        });
        let result = self.main_loop();
        self.frames.clear();
        self.files.clear();
        result
    }

    fn main_loop(&mut self) -> Result<Outcome, MacroError> {
        loop {
            let Some(frame) = self.frames.last() else {
                return Ok(Outcome::Finished);
            };
            if frame.pc >= frame.prog.lines.len() {
                self.frames.pop();
                continue;
            }
            if self.stop.load(Ordering::SeqCst) {
                return Ok(Outcome::Stopped);
            }
            let prog = Rc::clone(&frame.prog);
            let pc = frame.pc;
            let line = &prog.lines[pc];
            let flow = match self.exec(&prog, pc, &line.stmt) {
                Ok(f) => f,
                Err(Fail::Stopped) => return Ok(Outcome::Stopped),
                Err(Fail::Msg(message)) => {
                    let frame = self.frames.last().expect("frame");
                    return Err(MacroError {
                        file: frame.file.display().to_string(),
                        line: line.no,
                        message,
                    });
                }
            };
            match flow {
                Flow::Next => self.frame().pc = pc + 1,
                Flow::Jump(to) => self.frame().pc = to,
                Flow::End => return Ok(Outcome::Finished),
                Flow::Exit => {
                    self.frames.pop();
                }
                Flow::Include(path) => {
                    self.frame().pc = pc + 1;
                    let err = |message: String| MacroError {
                        file: path.display().to_string(),
                        line: 0,
                        message,
                    };
                    if self.frames.len() > MAX_INCLUDE {
                        return Err(err("include is nested too deeply".into()));
                    }
                    let src = std::fs::read(&path).map_err(|e| err(e.to_string()))?;
                    let prog = parse(&String::from_utf8_lossy(&src)).map_err(|e| MacroError {
                        file: path.display().to_string(),
                        line: e.line,
                        message: e.message,
                    })?;
                    self.frames.push(Frame {
                        prog: Rc::new(prog),
                        file: path,
                        pc: 0,
                        calls: Vec::new(),
                        fors: HashMap::new(),
                    });
                }
            }
        }
    }

    fn frame(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("frame")
    }

    // ------------------------------------------------------------ 文

    fn exec(&mut self, prog: &Program, pc: usize, stmt: &Stmt) -> R<Flow> {
        Ok(match stmt {
            Stmt::Label(_) => Flow::Next,
            Stmt::Assign { name, index, value } => {
                let v = self.eval(value)?;
                match index {
                    Some(i) => {
                        let i = self.int(i)?;
                        self.set_elem(name, i, v)?;
                    }
                    None => self.assign(name, v)?,
                }
                Flow::Next
            }
            Stmt::Command { name, args } => self.command(name, args)?,
            Stmt::If { cond, then } => {
                let c = self.truth(cond)?;
                match then {
                    Some(inner) => {
                        if !c {
                            return Ok(Flow::Next);
                        }
                        return match &**inner {
                            Stmt::Break => Ok(Flow::Jump(prog.end[prog.owner[pc]] + 1)),
                            Stmt::Continue => Ok(Flow::Jump(prog.end[prog.owner[pc]])),
                            s => self.exec(prog, pc, s),
                        };
                    }
                    None if c => Flow::Next,
                    None => {
                        // 次の節を順に試す
                        let mut j = prog.next[pc];
                        loop {
                            match &prog.lines[j].stmt {
                                Stmt::ElseIf(cond) => {
                                    if self.truth(cond)? {
                                        break Flow::Jump(j + 1);
                                    }
                                    j = prog.next[j];
                                }
                                _ => break Flow::Jump(j + 1),
                            }
                        }
                    }
                }
            }
            // 前の節を実行し終えて来た
            Stmt::ElseIf(_) | Stmt::Else => Flow::Jump(prog.end[pc] + 1),
            Stmt::EndIf => Flow::Next,
            Stmt::While(cond) => {
                if self.truth(cond)? {
                    Flow::Next
                } else {
                    Flow::Jump(prog.end[pc] + 1)
                }
            }
            Stmt::Until(cond) => {
                if self.truth(cond)? {
                    Flow::Jump(prog.end[pc] + 1)
                } else {
                    Flow::Next
                }
            }
            Stmt::EndWhile | Stmt::EndUntil => Flow::Jump(prog.end[pc]),
            Stmt::Do(cond) => match cond {
                Some((w, e)) if self.truth(e)? != *w => Flow::Jump(prog.end[pc] + 1),
                _ => Flow::Next,
            },
            Stmt::Loop(cond) => match cond {
                Some((w, e)) => {
                    if self.truth(e)? == *w {
                        Flow::Jump(prog.end[pc] + 1)
                    } else {
                        Flow::Next
                    }
                }
                None => Flow::Jump(prog.end[pc]),
            },
            Stmt::For { var, start, end } => {
                let s = self.int(start)?;
                let e = self.int(end)?;
                let step = if s <= e { 1 } else { -1 };
                self.assign(var, Value::Int(s))?;
                self.frame().fors.insert(pc, (e, step));
                Flow::Next
            }
            Stmt::Next => {
                let start = prog.end[pc];
                let Stmt::For { var, .. } = &prog.lines[start].stmt else {
                    unreachable!()
                };
                let (e, step) = *self.frame().fors.get(&start).ok_or("next without for")?;
                let v = self.get_int(var)?.wrapping_add(step);
                self.assign(var, Value::Int(v))?;
                if (step > 0 && v <= e) || (step < 0 && v >= e) {
                    Flow::Jump(start + 1)
                } else {
                    Flow::Next
                }
            }
            Stmt::Break => Flow::Jump(prog.end[prog.owner[pc]] + 1),
            Stmt::Continue => Flow::Jump(prog.end[prog.owner[pc]]),
            Stmt::Goto(label) => Flow::Jump(
                *prog
                    .labels
                    .get(label)
                    .ok_or_else(|| format!("no label '{label}'"))?,
            ),
            Stmt::Call(label) => {
                let to = *prog
                    .labels
                    .get(label)
                    .ok_or_else(|| format!("no label '{label}'"))?;
                let f = self.frame();
                if f.calls.len() > 1000 {
                    return Err("call is nested too deeply".into());
                }
                f.calls.push(pc + 1);
                Flow::Jump(to)
            }
            Stmt::Return => {
                let back = self.frame().calls.pop().ok_or("return without call")?;
                Flow::Jump(back)
            }
            Stmt::End => Flow::End,
            Stmt::Exit => Flow::Exit,
            Stmt::Include(e) => {
                let p = self.str(e)?;
                Flow::Include(self.path(&p))
            }
        })
    }

    // ------------------------------------------------------------ 式と変数

    fn eval(&mut self, e: &Expr) -> R<Value> {
        Ok(match e {
            Expr::Int(n) => Value::Int(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Var(name) => self
                .vars
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown variable '{name}'"))?,
            Expr::Index(name, i) => {
                let i = self.int(i)?;
                let arr = self
                    .arrays
                    .get(name)
                    .ok_or_else(|| format!("unknown array '{name}'"))?;
                arr.get(usize::try_from(i).map_err(|_| "negative index")?)
                    .cloned()
                    .ok_or_else(|| format!("index {i} out of range for '{name}'"))?
            }
            Expr::Unary(op, x) => {
                let v = self.int(x)?;
                Value::Int(match op {
                    UnOp::Neg => v.wrapping_neg(),
                    UnOp::Plus => v,
                    UnOp::BitNot => !v,
                    UnOp::Not => (v == 0) as i32,
                })
            }
            Expr::Binary(op, a, b) => {
                // && と || は短絡
                if let BinOp::And | BinOp::Or = op {
                    let l = self.truth(a)?;
                    if (*op == BinOp::And) != l {
                        return Ok(Value::Int(l as i32));
                    }
                    return Ok(Value::Int(self.truth(b)? as i32));
                }
                let l = self.eval(a)?;
                let r = self.eval(b)?;
                match (l, r) {
                    (Value::Int(l), Value::Int(r)) => Value::Int(int_op(*op, l, r)?),
                    (Value::Str(l), Value::Str(r)) => Value::Int(match op {
                        BinOp::Eq => (l == r) as i32,
                        BinOp::Ne => (l != r) as i32,
                        BinOp::Lt => (l < r) as i32,
                        BinOp::Gt => (l > r) as i32,
                        BinOp::Le => (l <= r) as i32,
                        BinOp::Ge => (l >= r) as i32,
                        _ => return Err("this operator needs integers".into()),
                    }),
                    _ => return Err("type mismatch".into()),
                }
            }
        })
    }

    fn int(&mut self, e: &Expr) -> R<i32> {
        match self.eval(e)? {
            Value::Int(n) => Ok(n),
            Value::Str(_) => Err("integer expected".into()),
        }
    }

    fn str(&mut self, e: &Expr) -> R<String> {
        match self.eval(e)? {
            Value::Str(s) => Ok(s),
            Value::Int(_) => Err("string expected".into()),
        }
    }

    fn truth(&mut self, e: &Expr) -> R<bool> {
        Ok(self.int(e)? != 0)
    }

    fn get_int(&self, name: &str) -> R<i32> {
        match self.vars.get(name) {
            Some(Value::Int(n)) => Ok(*n),
            _ => Err(format!("'{name}' is not an integer variable").into()),
        }
    }

    /// 変数に代入する。最初の代入で型が決まり、違う型は入れられない。
    fn assign(&mut self, name: &str, v: Value) -> R<()> {
        if self.arrays.contains_key(name) {
            return Err(format!("'{name}' is an array").into());
        }
        if let Some(old) = self.vars.get(name)
            && std::mem::discriminant(old) != std::mem::discriminant(&v)
        {
            return Err(format!("type mismatch for '{name}'").into());
        }
        self.vars.insert(name.to_owned(), v);
        Ok(())
    }

    fn set_elem(&mut self, name: &str, i: i32, v: Value) -> R<()> {
        let arr = self
            .arrays
            .get_mut(name)
            .ok_or_else(|| format!("unknown array '{name}'"))?;
        let slot = usize::try_from(i)
            .ok()
            .and_then(|i| arr.get_mut(i))
            .ok_or_else(|| format!("index {i} out of range for '{name}'"))?;
        if std::mem::discriminant(slot) != std::mem::discriminant(&v) {
            return Err("type mismatch".into());
        }
        *slot = v;
        Ok(())
    }

    /// 結果を書き込む引数（変数か配列の要素）。
    fn set_out(&mut self, target: &Expr, v: Value) -> R<()> {
        match target {
            Expr::Var(name) => self.assign(name, v),
            Expr::Index(name, i) => {
                let i = self.int(i)?;
                self.set_elem(name, i, v)
            }
            _ => Err("variable expected".into()),
        }
    }

    fn set_result(&mut self, n: i32) {
        self.vars.insert("result".into(), Value::Int(n));
    }

    fn set_str(&mut self, name: &str, s: String) {
        self.vars.insert(name.into(), Value::Str(s));
    }

    fn path(&self, p: &str) -> PathBuf {
        let p = Path::new(p);
        if p.is_absolute() {
            p.to_owned()
        } else {
            self.base_dir.join(p)
        }
    }

    // ------------------------------------------------------------ 受信

    fn session_ref(&self) -> R<Arc<Session>> {
        self.session.clone().ok_or_else(|| "not connected".into())
    }

    /// `timeout` と `mtimeout` の合計。0 以下は無期限。
    fn deadline(&self) -> Option<Instant> {
        let t = match self.vars.get("timeout") {
            Some(Value::Int(n)) => *n as i64,
            _ => 0,
        };
        let m = match self.vars.get("mtimeout") {
            Some(Value::Int(n)) => *n as i64,
            _ => 0,
        };
        let ms = t * 1000 + m;
        (ms > 0).then(|| Instant::now() + Duration::from_millis(ms as u64))
    }

    /// 受信を待ちながら `check` が Some を返すまで繰り返す。時間切れは None。
    fn wait_until<T>(&mut self, mut check: impl FnMut(&mut String) -> Option<T>) -> R<Option<T>> {
        let deadline = self.deadline();
        if self.tap.is_none() && deadline.is_none() {
            return Err("not connected".into());
        }
        loop {
            if let Some(tap) = &self.tap {
                self.recv.push_str(&tap.drain());
            }
            self.trim_recv();
            if let Some(v) = check(&mut self.recv) {
                return Ok(Some(v));
            }
            if self.stop.load(Ordering::SeqCst) {
                return Err(Fail::Stopped);
            }
            let mut step = Duration::from_millis(100);
            if let Some(d) = deadline {
                let now = Instant::now();
                if now >= d {
                    return Ok(None);
                }
                step = step.min(d - now);
            }
            match &self.tap {
                Some(tap) => {
                    if let Some(s) = tap.recv(step) {
                        self.recv.push_str(&s);
                    }
                }
                None => std::thread::sleep(step),
            }
        }
    }

    fn trim_recv(&mut self) {
        if self.recv.len() > RECV_LIMIT {
            let mut cut = self.recv.len() - RECV_LIMIT / 2;
            while !self.recv.is_char_boundary(cut) {
                cut += 1;
            }
            self.recv.drain(..cut);
        }
    }

    fn sleep(&self, d: Duration) -> R<()> {
        let end = Instant::now() + d;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Err(Fail::Stopped);
            }
            let now = Instant::now();
            if now >= end {
                return Ok(());
            }
            std::thread::sleep((end - now).min(Duration::from_millis(50)));
        }
    }

    // ------------------------------------------------------------ コマンド

    fn command(&mut self, name: &str, args: &[Expr]) -> R<Flow> {
        let n = args.len();
        let argc = |min: usize, max: usize| -> R<()> {
            if n < min || n > max {
                Err(format!("{name}: wrong number of arguments").into())
            } else {
                Ok(())
            }
        };
        match name {
            // ---- 通信
            "connect" => {
                argc(1, 1)?;
                let spec = self.str(&args[0])?;
                if self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.state() != SessionState::Closed)
                {
                    // 接続済みなら何もしない（Tera Term と同じ）
                    return Ok(Flow::Next);
                }
                let spec = parse_connect(&spec)?;
                match self.host.connect(&spec) {
                    Ok(session) => {
                        // 接続してから写しを取り始めるまでに届いたプロンプトも見る
                        self.tap = Some(session.tap_with_history());
                        self.session = Some(session);
                        self.recv.clear();
                        self.set_result(2);
                    }
                    Err(e) => return Err(format!("connect: {e}").into()),
                }
            }
            "disconnect" | "closett" => {
                if let Some(s) = self.session.take() {
                    self.host.disconnect(&s);
                }
                self.tap = None;
            }
            "testlink" => {
                let connected = self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.state() == SessionState::Connected);
                self.set_result(if connected { 2 } else { 1 });
            }
            "send" | "sendln" => {
                let s = self.session_ref()?;
                for a in args {
                    match self.eval(a)? {
                        Value::Str(t) => s.send_text(&t),
                        Value::Int(c) => {
                            let c = c as u32;
                            if c < 0x100 {
                                s.write(vec![c as u8]);
                            } else if let Some(ch) = char::from_u32(c) {
                                s.send_text(ch.encode_utf8(&mut [0; 4]));
                            }
                        }
                    }
                }
                if name == "sendln" {
                    s.send_line("");
                }
            }
            "sendbreak" => {
                argc(0, 0)?;
                self.session_ref()?
                    .send_break(300)
                    .map_err(|e| format!("sendbreak: {e}"))?;
            }
            "setdtr" | "setrts" => {
                argc(1, 1)?;
                let on = self.int(&args[0])? != 0;
                if name == "setdtr" {
                    self.dtr = on;
                } else {
                    self.rts = on;
                }
                self.session_ref()?
                    .set_lines(self.dtr, self.rts)
                    .map_err(|e| format!("{name}: {e}"))?;
            }
            "flushrecv" => {
                argc(0, 0)?;
                if let Some(tap) = &self.tap {
                    tap.drain();
                }
                self.recv.clear();
            }
            "wait" => {
                if n == 0 {
                    return Err("wait: no string".into());
                }
                let pats: Vec<String> = args.iter().map(|a| self.str(a)).collect::<R<_>>()?;
                let hit = self.wait_until(|buf| {
                    let (i, pos, len) = earliest(buf, &pats)?;
                    buf.drain(..pos + len);
                    Some(i)
                })?;
                self.set_result(hit.map_or(0, |i| i as i32 + 1));
            }
            "waitln" => {
                if n == 0 {
                    return Err("waitln: no string".into());
                }
                let pats: Vec<String> = args.iter().map(|a| self.str(a)).collect::<R<_>>()?;
                let hit = self.wait_until(|buf| {
                    let (i, pos, len) = earliest(buf, &pats)?;
                    let nl = buf[pos + len..].find('\n')? + pos + len;
                    let start = buf[..pos].rfind('\n').map_or(0, |p| p + 1);
                    let line = buf[start..nl].trim_end_matches('\r').to_owned();
                    buf.drain(..=nl);
                    Some((i, line))
                })?;
                match hit {
                    Some((i, line)) => {
                        self.set_str("inputstr", line);
                        self.set_result(i as i32 + 1);
                    }
                    None => self.set_result(0),
                }
            }
            "recvln" => {
                argc(0, 0)?;
                let hit = self.wait_until(|buf| {
                    let nl = buf.find('\n')?;
                    let line = buf[..nl].trim_end_matches('\r').to_owned();
                    buf.drain(..=nl);
                    Some(line)
                })?;
                match hit {
                    Some(line) => {
                        self.set_str("inputstr", line);
                        self.set_result(1);
                    }
                    None => self.set_result(0),
                }
            }
            "waitregex" => {
                if n == 0 {
                    return Err("waitregex: no pattern".into());
                }
                let res: Vec<Regex> = args
                    .iter()
                    .map(|a| {
                        let p = self.str(a)?;
                        Regex::new(&p).map_err(|e| Fail::Msg(format!("waitregex: {e}")))
                    })
                    .collect::<R<_>>()?;
                let hit = self.wait_until(|buf| {
                    let (i, groups, end) = res
                        .iter()
                        .enumerate()
                        .filter_map(|(i, re)| {
                            let c = re.captures(buf)?;
                            let m = c.get(0)?;
                            let groups: Vec<String> = (0..c.len())
                                .map(|g| c.get(g).map_or(String::new(), |m| m.as_str().to_owned()))
                                .collect();
                            Some((m.start(), i, groups, m.end()))
                        })
                        .min_by_key(|x| (x.0, x.1))
                        .map(|(_, i, g, e)| (i, g, e))?;
                    buf.drain(..end);
                    Some((i, groups))
                })?;
                match hit {
                    Some((i, groups)) => {
                        self.set_groups(&groups);
                        self.set_result(i as i32 + 1);
                    }
                    None => self.set_result(0),
                }
            }
            "logopen" => {
                argc(1, 8)?;
                let file = self.str(&args[0])?;
                let append = match args.get(2) {
                    Some(a) => self.int(a)? != 0,
                    None => false,
                };
                let mut opts = LogOptions::new(self.path(&file));
                opts.append = append;
                if let Some(a) = args.get(1)
                    && self.int(a)? != 0
                {
                    opts.format = mt_core::LogFormat::Binary;
                }
                self.session_ref()?
                    .log_start(&opts)
                    .map_err(|e| format!("logopen: {e}"))?;
            }
            "logclose" => {
                if let Some(s) = &self.session {
                    s.log_stop();
                }
            }
            "sendfile" => {
                argc(1, 2)?;
                let file = self.str(&args[0])?;
                let binary = match args.get(1) {
                    Some(a) => self.int(a)? != 0,
                    None => false,
                };
                let data = std::fs::read(self.path(&file))
                    .map_err(|e| format!("sendfile: {file}: {e}"))?;
                let s = self.session_ref()?;
                if binary {
                    s.write(data);
                } else {
                    let text = String::from_utf8_lossy(&data);
                    let mut lines = text.split('\n').peekable();
                    while let Some(l) = lines.next() {
                        let l = l.strip_suffix('\r').unwrap_or(l);
                        if lines.peek().is_some() {
                            s.send_line(l);
                        } else if !l.is_empty() {
                            s.send_text(l);
                        }
                    }
                }
            }

            // ---- ファイル転送
            "xmodemsend" => {
                argc(1, 2)?;
                let file = self.str(&args[0])?;
                let opt = match args.get(1) {
                    Some(a) => self.int(a)?,
                    None => 2,
                };
                let p = Protocol::XModem {
                    crc: opt != 1,
                    one_k: opt == 3,
                };
                self.transfer(p, Direction::Send(vec![self.path(&file)]))?;
            }
            "xmodemrecv" => {
                argc(1, 3)?;
                let file = self.str(&args[0])?;
                let opt = match args.get(2) {
                    Some(a) => self.int(a)?,
                    None => 2,
                };
                let p = Protocol::XModem {
                    crc: opt != 1,
                    one_k: false,
                };
                self.transfer(p, Direction::Receive(self.path(&file)))?;
            }
            "ymodemsend" | "zmodemsend" | "kermitsend" => {
                argc(1, 2)?;
                let file = self.str(&args[0])?;
                let p = match name {
                    "ymodemsend" => Protocol::YModem,
                    "zmodemsend" => Protocol::ZModem,
                    _ => Protocol::Kermit,
                };
                self.transfer(p, Direction::Send(vec![self.path(&file)]))?;
            }
            "ymodemrecv" | "zmodemrecv" | "kermitrecv" => {
                let p = match name {
                    "ymodemrecv" => Protocol::YModem,
                    "zmodemrecv" => Protocol::ZModem,
                    _ => Protocol::Kermit,
                };
                let dir = self.host.download_dir();
                self.transfer(p, Direction::Receive(dir))?;
            }

            // ---- 文字列
            "strlen" => {
                argc(1, 1)?;
                let s = self.str(&args[0])?;
                self.set_result(s.len() as i32);
            }
            "strconcat" => {
                argc(2, 2)?;
                let mut a = self.str(&args[0])?;
                a.push_str(&self.str(&args[1])?);
                self.set_out(&args[0], Value::Str(a))?;
            }
            "strcompare" => {
                argc(2, 2)?;
                let a = self.str(&args[0])?;
                let b = self.str(&args[1])?;
                self.set_result(match a.cmp(&b) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                });
            }
            "strcopy" => {
                argc(4, 4)?;
                let s = self.str(&args[0])?;
                let start = self.int(&args[1])?.max(1) as usize - 1;
                let len = self.int(&args[2])?.max(0) as usize;
                let b = s.as_bytes();
                let part = if start < b.len() {
                    String::from_utf8_lossy(&b[start..(start + len).min(b.len())]).into_owned()
                } else {
                    String::new()
                };
                self.set_out(&args[3], Value::Str(part))?;
            }
            "strscan" => {
                argc(2, 2)?;
                let s = self.str(&args[0])?;
                let sub = self.str(&args[1])?;
                self.set_result(s.find(&sub).map_or(0, |p| p as i32 + 1));
            }
            "strinsert" => {
                argc(3, 3)?;
                let mut s = self.str(&args[0])?.into_bytes();
                let at = (self.int(&args[1])?.max(1) as usize - 1).min(s.len());
                let ins = self.str(&args[2])?;
                s.splice(at..at, ins.bytes());
                self.set_out(
                    &args[0],
                    Value::Str(String::from_utf8_lossy(&s).into_owned()),
                )?;
            }
            "strremove" => {
                argc(3, 3)?;
                let mut s = self.str(&args[0])?.into_bytes();
                let at = (self.int(&args[1])?.max(1) as usize - 1).min(s.len());
                let len = (self.int(&args[2])?.max(0) as usize).min(s.len() - at);
                s.drain(at..at + len);
                self.set_out(
                    &args[0],
                    Value::Str(String::from_utf8_lossy(&s).into_owned()),
                )?;
            }
            "strtrim" => {
                argc(2, 2)?;
                let s = self.str(&args[0])?;
                let chars: Vec<char> = self.str(&args[1])?.chars().collect();
                let t = s.trim_matches(|c| chars.contains(&c)).to_owned();
                self.set_out(&args[0], Value::Str(t))?;
            }
            "strreplace" => {
                argc(4, 4)?;
                let s = self.str(&args[0])?;
                let from = (self.int(&args[1])?.max(1) as usize - 1).min(s.len());
                let re =
                    Regex::new(&self.str(&args[2])?).map_err(|e| format!("strreplace: {e}"))?;
                let new = self.str(&args[3])?;
                if !s.is_char_boundary(from) {
                    return Err("strreplace: index is inside a character".into());
                }
                let (head, tail) = s.split_at(from);
                let replaced = re.replace(tail, new.as_str());
                let changed = replaced != tail;
                let out = format!("{head}{replaced}");
                self.set_out(&args[0], Value::Str(out))?;
                self.set_result(changed as i32);
            }
            "strsplit" => {
                argc(2, 3)?;
                let s = self.str(&args[0])?;
                let sep = self.str(&args[1])?;
                let max = match args.get(2) {
                    Some(a) => self.int(a)?.clamp(1, 9) as usize,
                    None => 9,
                };
                let parts: Vec<String> = if sep.is_empty() {
                    vec![s]
                } else {
                    s.splitn(max, sep.as_str()).map(str::to_owned).collect()
                };
                for i in 1..=9 {
                    self.set_str(
                        &format!("groupmatchstr{i}"),
                        parts.get(i - 1).cloned().unwrap_or_default(),
                    );
                }
                self.set_result(parts.len() as i32);
            }
            "strmatch" => {
                argc(2, 2)?;
                let s = self.str(&args[0])?;
                let re = Regex::new(&self.str(&args[1])?).map_err(|e| format!("strmatch: {e}"))?;
                match re.captures(&s) {
                    Some(c) => {
                        let groups: Vec<String> = (0..c.len())
                            .map(|g| c.get(g).map_or(String::new(), |m| m.as_str().to_owned()))
                            .collect();
                        let pos = c.get(0).map_or(0, |m| m.start());
                        self.set_groups(&groups);
                        self.set_result(pos as i32 + 1);
                    }
                    None => self.set_result(0),
                }
            }
            "str2int" => {
                argc(2, 2)?;
                let s = self.str(&args[1])?;
                let t = s.trim();
                let parsed = match t.strip_prefix('$') {
                    Some(h) => i64::from_str_radix(h, 16).ok().map(|n| n as u32 as i32),
                    None => t.parse::<i32>().ok(),
                };
                match parsed {
                    Some(v) => {
                        self.set_out(&args[0], Value::Int(v))?;
                        self.set_result(1);
                    }
                    None => self.set_result(0),
                }
            }
            "int2str" => {
                argc(2, 2)?;
                let v = self.int(&args[1])?;
                self.set_out(&args[0], Value::Str(v.to_string()))?;
            }
            "sprintf" | "sprintf2" => {
                let (out, rest) = if name == "sprintf2" {
                    argc(2, 99)?;
                    (Some(&args[0]), &args[1..])
                } else {
                    argc(1, 99)?;
                    (None, args)
                };
                let fmt = self.str(&rest[0])?;
                let vals: Vec<Value> = rest[1..].iter().map(|a| self.eval(a)).collect::<R<_>>()?;
                let s = sprintf(&fmt, &vals).map_err(|e| format!("{name}: {e}"))?;
                match out {
                    Some(t) => self.set_out(t, Value::Str(s))?,
                    None => self.set_str("inputstr", s),
                }
                self.set_result(0);
            }
            "tolower" | "toupper" => {
                argc(2, 2)?;
                let s = self.str(&args[1])?;
                let t = if name == "tolower" {
                    s.to_ascii_lowercase()
                } else {
                    s.to_ascii_uppercase()
                };
                self.set_out(&args[0], Value::Str(t))?;
            }
            "code2str" => {
                argc(2, 2)?;
                let c = self.int(&args[1])? as u32;
                let bytes: Vec<u8> = c
                    .to_be_bytes()
                    .into_iter()
                    .skip_while(|&b| b == 0)
                    .collect();
                self.set_out(
                    &args[0],
                    Value::Str(String::from_utf8_lossy(&bytes).into_owned()),
                )?;
            }
            "str2code" => {
                argc(2, 2)?;
                let s = self.str(&args[1])?;
                let v = s.bytes().take(4).fold(0u32, |acc, b| (acc << 8) | b as u32);
                self.set_out(&args[0], Value::Int(v as i32))?;
            }

            // ---- 変数
            "intdim" | "strdim" => {
                argc(2, 2)?;
                let Expr::Var(arr) = &args[0] else {
                    return Err(format!("{name}: array name expected").into());
                };
                let size = self.int(&args[1])?;
                if !(1..=65536).contains(&size) {
                    return Err(format!("{name}: bad size {size}").into());
                }
                if self.vars.contains_key(arr) {
                    return Err(format!("'{arr}' is already a variable").into());
                }
                let init = if name == "intdim" {
                    Value::Int(0)
                } else {
                    Value::Str(String::new())
                };
                self.arrays.insert(arr.clone(), vec![init; size as usize]);
            }

            // ---- ファイル
            "fileopen" | "filecreate" => {
                let (min, max) = if name == "fileopen" { (3, 4) } else { (2, 2) };
                argc(min, max)?;
                let file = self.str(&args[1])?;
                let path = self.path(&file);
                let opened = if name == "filecreate" {
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&path)
                } else {
                    let append = self.int(&args[2])? != 0;
                    let readonly = match args.get(3) {
                        Some(a) => self.int(a)? != 0,
                        None => false,
                    };
                    let f = if readonly {
                        File::open(&path)
                    } else {
                        OpenOptions::new()
                            .read(true)
                            .write(true)
                            .create(true)
                            .truncate(false)
                            .open(&path)
                    };
                    f.and_then(|mut f| {
                        if append {
                            f.seek(SeekFrom::End(0))?;
                        }
                        Ok(f)
                    })
                };
                let handle = match opened {
                    Ok(file) => {
                        let h = self.next_handle;
                        self.next_handle += 1;
                        self.files.insert(h, Handle { file });
                        self.set_result(0);
                        h
                    }
                    Err(_) => {
                        self.set_result(-1);
                        -1
                    }
                };
                self.set_out(&args[0], Value::Int(handle))?;
            }
            "fileclose" => {
                argc(1, 1)?;
                let h = self.int(&args[0])?;
                self.files.remove(&h);
            }
            "filereadln" => {
                argc(2, 2)?;
                let h = self.int(&args[0])?;
                let f = self
                    .files
                    .get_mut(&h)
                    .ok_or("filereadln: bad file handle")?;
                let mut line = Vec::new();
                let mut byte = [0u8];
                let mut eof = true;
                while let Ok(1) = f.file.read(&mut byte) {
                    eof = false;
                    if byte[0] == b'\n' {
                        break;
                    }
                    line.push(byte[0]);
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.set_out(
                    &args[1],
                    Value::Str(String::from_utf8_lossy(&line).into_owned()),
                )?;
                self.set_result(eof as i32);
            }
            "filewrite" | "filewriteln" => {
                argc(2, 2)?;
                let h = self.int(&args[0])?;
                let mut data = match self.eval(&args[1])? {
                    Value::Str(s) => s.into_bytes(),
                    Value::Int(c) => vec![c as u8],
                };
                if name == "filewriteln" {
                    data.extend_from_slice(b"\r\n");
                }
                let f = self.files.get_mut(&h).ok_or("filewrite: bad file handle")?;
                f.file
                    .write_all(&data)
                    .map_err(|e| format!("{name}: {e}"))?;
            }
            "filesearch" => {
                argc(1, 1)?;
                let file = self.str(&args[0])?;
                let found = self.path(&file).exists();
                self.set_result(found as i32);
            }
            "filedelete" => {
                argc(1, 1)?;
                let file = self.str(&args[0])?;
                let ok = std::fs::remove_file(self.path(&file)).is_ok();
                self.set_result(if ok { 0 } else { -1 });
            }
            "getdir" => {
                argc(1, 1)?;
                let d = self.base_dir.display().to_string();
                self.set_out(&args[0], Value::Str(d))?;
            }
            "setdir" => {
                argc(1, 1)?;
                let d = self.str(&args[0])?;
                self.base_dir = self.path(&d);
            }

            // ---- ダイアログと画面
            "messagebox" => {
                argc(1, 2)?;
                let text = self.str(&args[0])?;
                let title = self.opt_str(args.get(1))?;
                self.host.message_box(&title, &text);
            }
            "inputbox" | "passwordbox" => {
                argc(1, 3)?;
                let text = self.str(&args[0])?;
                let title = self.opt_str(args.get(1))?;
                let default = self.opt_str(args.get(2))?;
                let answer = self
                    .host
                    .input_box(&title, &text, &default, name == "passwordbox");
                self.set_result(answer.is_some() as i32);
                self.set_str("inputstr", answer.unwrap_or_default());
            }
            "yesnobox" => {
                argc(1, 2)?;
                let text = self.str(&args[0])?;
                let title = self.opt_str(args.get(1))?;
                let yes = self.host.yes_no_box(&title, &text);
                self.set_result(yes as i32);
            }
            "statusbox" => {
                argc(1, 2)?;
                let text = self.str(&args[0])?;
                let title = self.opt_str(args.get(1))?;
                self.host.status_box(&title, Some(&text));
            }
            "closesbox" => self.host.status_box("", None),
            "dispstr" => {
                let mut s = String::new();
                for a in args {
                    match self.eval(a)? {
                        Value::Str(t) => s.push_str(&t),
                        Value::Int(c) => s.extend(char::from_u32(c as u32)),
                    }
                }
                self.host.display(&s);
            }
            "settitle" => {
                argc(1, 1)?;
                self.title = self.str(&args[0])?;
                let t = self.title.clone();
                self.host.set_title(&t);
            }
            "gettitle" => {
                argc(1, 1)?;
                let t = self.title.clone();
                self.set_out(&args[0], Value::Str(t))?;
            }
            "beep" => self.host.beep(),
            "show" | "showtt" => {}

            // ---- その他
            "pause" => {
                argc(1, 1)?;
                let s = self.int(&args[0])?.max(0) as u64;
                self.sleep(Duration::from_secs(s))?;
            }
            "mpause" => {
                argc(1, 1)?;
                let ms = self.int(&args[0])?.max(0) as u64;
                self.sleep(Duration::from_millis(ms))?;
            }
            "getdate" | "gettime" => {
                argc(1, 2)?;
                let default = if name == "getdate" {
                    "%Y-%m-%d"
                } else {
                    "%H:%M:%S"
                };
                let fmt = match args.get(1) {
                    Some(a) => self.str(a)?,
                    None => default.to_owned(),
                };
                let mut out = String::new();
                use std::fmt::Write as _;
                write!(out, "{}", chrono::Local::now().format(&fmt))
                    .map_err(|_| format!("{name}: bad format"))?;
                self.set_out(&args[0], Value::Str(out))?;
            }
            "getenv" => {
                argc(2, 2)?;
                let k = self.str(&args[0])?;
                let v = std::env::var(&k).unwrap_or_default();
                self.set_out(&args[1], Value::Str(v))?;
            }
            "random" => {
                argc(2, 2)?;
                let max = self.int(&args[1])?.max(0) as u64;
                // xorshift64
                self.rng ^= self.rng << 13;
                self.rng ^= self.rng >> 7;
                self.rng ^= self.rng << 17;
                let v = (self.rng % (max + 1)) as i32;
                self.set_out(&args[0], Value::Int(v))?;
            }
            _ => return Err(format!("unsupported command '{name}'").into()),
        }
        Ok(Flow::Next)
    }

    fn opt_str(&mut self, e: Option<&Expr>) -> R<String> {
        match e {
            Some(e) => self.str(e),
            None => Ok(String::new()),
        }
    }

    fn set_groups(&mut self, groups: &[String]) {
        self.set_str("matchstr", groups.first().cloned().unwrap_or_default());
        for i in 1..=9 {
            self.set_str(
                &format!("groupmatchstr{i}"),
                groups.get(i).cloned().unwrap_or_default(),
            );
        }
    }

    /// 転送を始めて終わるまで待つ。result は成功で 1、失敗で 0。
    fn transfer(&mut self, protocol: Protocol, direction: Direction) -> R<()> {
        let session = self.session_ref()?;
        let send = matches!(direction, Direction::Send(_));
        let label = match &direction {
            Direction::Send(f) => f
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            Direction::Receive(p) => p.display().to_string(),
        };
        let transfer = mt_xfer::new_transfer(protocol, direction)
            .map_err(|e| format!("transfer: {label}: {e}"))?;
        let shared = Arc::new(Mutex::new(transfer));
        session
            .start_transfer(Box::new(Divert(Arc::clone(&shared))))
            .map_err(|e| format!("transfer: {e}"))?;
        self.host.transfer_started(&label, send);
        let ok = loop {
            let status = shared.lock().unwrap_or_else(|e| e.into_inner()).status();
            if status.is_finished() {
                break status == mt_xfer::Status::Done;
            }
            if self.stop.load(Ordering::SeqCst) {
                session.cancel_transfer();
                self.host.transfer_finished(false);
                return Err(Fail::Stopped);
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        self.host.transfer_finished(ok);
        self.set_result(ok as i32);
        Ok(())
    }
}

/// いちばん手前で見つかった文字列（同じ位置なら先に書いたもの）。(番号, 位置, 長さ)
fn earliest(buf: &str, pats: &[String]) -> Option<(usize, usize, usize)> {
    pats.iter()
        .enumerate()
        .filter(|(_, p)| !p.is_empty())
        .filter_map(|(i, p)| buf.find(p.as_str()).map(|pos| (i, pos, p.len())))
        .min_by_key(|&(i, pos, _)| (pos, i))
}

fn int_op(op: BinOp, l: i32, r: i32) -> Result<i32, Fail> {
    Ok(match op {
        BinOp::Mul => l.wrapping_mul(r),
        BinOp::Div => {
            if r == 0 {
                return Err("division by zero".into());
            }
            l.wrapping_div(r)
        }
        BinOp::Mod => {
            if r == 0 {
                return Err("division by zero".into());
            }
            l.wrapping_rem(r)
        }
        BinOp::Add => l.wrapping_add(r),
        BinOp::Sub => l.wrapping_sub(r),
        BinOp::Shr => l.wrapping_shr(r as u32),
        BinOp::Shl => l.wrapping_shl(r as u32),
        BinOp::UShr => ((l as u32).wrapping_shr(r as u32)) as i32,
        BinOp::BitAnd => l & r,
        BinOp::BitXor => l ^ r,
        BinOp::BitOr => l | r,
        BinOp::Lt => (l < r) as i32,
        BinOp::Gt => (l > r) as i32,
        BinOp::Le => (l <= r) as i32,
        BinOp::Ge => (l >= r) as i32,
        BinOp::Eq => (l == r) as i32,
        BinOp::Ne => (l != r) as i32,
        BinOp::And | BinOp::Or => unreachable!(),
    })
}

/// mt-xfer の転送を Session の受信の行き先にする。
struct Divert(Arc<Mutex<Box<dyn Transfer>>>);

impl Divert {
    fn lock(&self) -> std::sync::MutexGuard<'_, Box<dyn Transfer>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl ByteDivert for Divert {
    fn start(&mut self) -> Vec<u8> {
        self.lock().start(Instant::now())
    }
    fn receive(&mut self, data: &[u8]) -> Vec<u8> {
        self.lock().receive(data, Instant::now())
    }
    fn tick(&mut self) -> Vec<u8> {
        self.lock().tick(Instant::now())
    }
    fn cancel(&mut self) -> Vec<u8> {
        self.lock().cancel()
    }
    fn finished(&self) -> bool {
        self.lock().status().is_finished()
    }
}
