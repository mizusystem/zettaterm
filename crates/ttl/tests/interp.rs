//! TTL インタプリタの結合テスト。相手はモック Transport（同じプロセスの中で応答を返す）。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use zetta_core::mock::{MockRemote, MockTransport};
use zetta_core::{Session, SessionListener, SessionOptions, SessionState};
use zetta_ttl::{ConnectSpec, Host, Interp, Outcome};

struct Quiet;
impl SessionListener for Quiet {
    fn on_damage(&self) {}
    fn on_state(&self, _: SessionState) {}
    fn on_bell(&self) {}
    fn on_title(&self, _: String) {}
}

/// messagebox / dispstr の中身を記録し、connect でモックの接続を渡す。
#[derive(Default)]
struct TestHost {
    out: Arc<Mutex<Vec<String>>>,
    pending: Option<Arc<Session>>,
    connects: Vec<ConnectSpec>,
    answers: Vec<String>,
    download: PathBuf,
    clip: Option<String>,
}

impl Host for TestHost {
    fn connect(&mut self, spec: &ConnectSpec) -> Result<Arc<Session>, String> {
        self.connects.push(spec.clone());
        self.pending.take().ok_or_else(|| "no device".into())
    }
    fn message_box(&mut self, _title: &str, text: &str) {
        self.out.lock().unwrap().push(text.to_owned());
    }
    fn display(&mut self, text: &str) {
        self.out.lock().unwrap().push(format!("disp:{text}"));
    }
    fn input_box(&mut self, _: &str, _: &str, _: &str, _: bool) -> Option<String> {
        if self.answers.is_empty() {
            None
        } else {
            Some(self.answers.remove(0))
        }
    }
    fn download_dir(&self) -> PathBuf {
        self.download.clone()
    }
    fn list_box(
        &mut self,
        _: &str,
        text: &str,
        items: &[String],
        selected: usize,
    ) -> Option<usize> {
        self.out
            .lock()
            .unwrap()
            .push(format!("list:{text}:{}:{selected}", items.join(",")));
        Some(items.len() - 1)
    }
    fn file_name_box(&mut self, _: &str, save: bool, dir: &Path) -> Option<String> {
        Some(
            dir.join(if save { "out.txt" } else { "in.txt" })
                .display()
                .to_string(),
        )
    }
    fn clipboard(&mut self) -> Option<String> {
        self.clip.clone()
    }
    fn set_clipboard(&mut self, text: &str) -> bool {
        self.clip = Some(text.to_owned());
        true
    }
    fn local_addresses(&mut self) -> Option<Vec<std::net::IpAddr>> {
        Some(
            ["127.0.0.1", "::1", "192.0.2.5"]
                .iter()
                .map(|a| a.parse().unwrap())
                .collect(),
        )
    }
}

fn open_mock() -> (Arc<Session>, MockRemote) {
    let (t, remote) = MockTransport::pair();
    let s = Session::open(Box::new(t), SessionOptions::default(), Arc::new(Quiet)).unwrap();
    (s, remote)
}

fn run(src: &str, host: &mut TestHost, session: Option<Arc<Session>>) -> Vec<String> {
    let stop = Arc::new(AtomicBool::new(false));
    let mut it = Interp::new(host, session, stop);
    it.run_source(src, Path::new(".")).unwrap();
    drop(it);
    host.out.lock().unwrap().clone()
}

/// 相手が受け取ったものを、`want` を含むまで集める。
fn expect(remote: &MockRemote, want: &[u8]) -> Vec<u8> {
    let start = Instant::now();
    let mut got = Vec::new();
    while !got.windows(want.len()).any(|w| w == want) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "never got {want:?}: {got:?}"
        );
        if let Some(b) = remote.recv(Duration::from_millis(50)) {
            got.extend(b);
        }
    }
    got
}

#[test]
fn control_flow_and_strings() {
    let src = r#"
; 制御構文
sum = 0
for i 1 10
  if i % 2 = 0 continue
  sum = sum + i
next
int2str s sum
messagebox s 'sum'

n = 0
while 1
  n = n + 1
  if n >= 5 break
endwhile
do
  n = n - 1
loop while n > 2
until n = 0
  n = n - 1
enduntil
sprintf2 s '%d|%03d|%-3s|%x' n 7 'ab' 255
messagebox s

for j 3 1
  call show
next
goto done
messagebox 'skipped'
:show
  int2str t j
  messagebox t
  return
:done

if 0 then
  messagebox 'no'
elseif 1 then
  messagebox 'elseif'
else
  messagebox 'else'
endif

s = 'abc'
strconcat s 'def'
strlen s
int2str t result
strconcat s t
strcopy s 2 3 u
strscan s 'de'
sprintf '%s %d %s' s result u
messagebox inputstr

strsplit 'a,b,c' ','
sprintf2 t '%d %s %s %s' result groupmatchstr1 groupmatchstr2 groupmatchstr3
messagebox t
strmatch 'ver 15.2(4)M' '(\d+)\.(\d+)'
sprintf2 t '%d %s %s %s' result matchstr groupmatchstr1 groupmatchstr2
messagebox t
str2int x '$1F'
int2str t x
messagebox t
strdim names 3
names[1] = 'one'
intdim nums 2
nums[1] = 40 + 2
sprintf2 t '%s %d %d' names[1] nums[1] nums[0]
messagebox t
s = 'Hello'
toupper s s
strreplace s 1 'L+' 'l'
messagebox s
dispstr 'x' 65 #10
"#;
    let mut host = TestHost::default();
    let out = run(src, &mut host, None);
    assert_eq!(
        out,
        [
            "25",
            "0|007|ab |ff",
            "3",
            "2",
            "1",
            "elseif",
            "abcdef6 4 bcd",
            "3 a b c",
            "5 15.2 15 2",
            "31",
            "one 42 0",
            "HElO",
            "disp:xA\n",
        ]
    );
}

#[test]
fn errors_name_the_line() {
    let mut host = TestHost::default();
    let stop = Arc::new(AtomicBool::new(false));
    let mut it = Interp::new(&mut host, None, stop);
    let e = it
        .run_source("x = 1\n\ny = x + 'a'\n", Path::new("."))
        .unwrap_err();
    assert_eq!((e.line, e.message.as_str()), (3, "type mismatch"));
    let e = it.run_source("frobnicate 1", Path::new(".")).unwrap_err();
    assert!(e.message.contains("unsupported command"), "{e}");
    let e = it.run_source("if 1 then\n", Path::new(".")).unwrap_err();
    assert_eq!(e.line, 1);
}

#[test]
fn send_and_wait_talk_to_the_device() {
    let (session, remote) = open_mock();
    let r = remote;
    let device = thread::spawn(move || {
        expect(&r, b"show version\r");
        r.send(b"Cisco IOS Software, Version 15.2(4)M\r\nuptime 3 days\r\nRouter>");
        expect(&r, b"enable\r");
        r.send(b"Password: ");
        expect(&r, b"\x03");
    });
    let src = r#"
timeout = 5
sendln 'show version'
waitln 'Version'
messagebox inputstr
recvln
messagebox inputstr
wait 'Router#' 'Router>'
int2str s result
messagebox s
sendln 'enable'
waitregex 'Pass(\w+):'
messagebox groupmatchstr1
send 3
mtimeout = 200
timeout = 0
wait 'never'
int2str s result
messagebox s
testlink
int2str s result
messagebox s
"#;
    let mut host = TestHost::default();
    let out = run(src, &mut host, Some(Arc::clone(&session)));
    device.join().unwrap();
    assert_eq!(
        out,
        [
            "Cisco IOS Software, Version 15.2(4)M",
            "uptime 3 days",
            "2",
            "word",
            "0",
            "2",
        ]
    );
    session.close();
}

#[test]
fn connect_goes_through_the_host() {
    let (session, remote) = open_mock();
    let mut host = TestHost {
        pending: Some(Arc::clone(&session)),
        ..Default::default()
    };
    // 接続した直後（マクロが受信を見始める前）に届いたプロンプトも wait で見える
    remote.send(b"\r\nUsername: ");
    thread::sleep(Duration::from_millis(100));
    let src = "connect '/C=/dev/cu.test /BAUD=9600'\ntimeout = 2\nwait 'Username:'\nint2str s result\nmessagebox s\nsendln 'hi'\ndisconnect\n";
    assert_eq!(run(src, &mut host, None), ["1"]);
    assert_eq!(
        host.connects,
        [ConnectSpec::Serial {
            port: "/dev/cu.test".into(),
            baud: Some(9600)
        }]
    );
    expect(&remote, b"hi\r");
    let mut host = TestHost::default();
    let stop = Arc::new(AtomicBool::new(false));
    let e = Interp::new(&mut host, None, stop)
        .run_source("connect 'router /telnet'", Path::new("."))
        .unwrap_err();
    assert!(e.message.contains("no device"), "{e}");
}

#[test]
fn xmodem_send_from_a_macro() {
    let dir = tempfile::tempdir().unwrap();
    let firmware: Vec<u8> = (0..5000u32).map(|i| (i * 31 % 256) as u8).collect();
    std::fs::write(dir.path().join("fw.bin"), &firmware).unwrap();
    let (session, remote) = open_mock();
    let out = dir.path().join("got.bin");
    let out2 = out.clone();
    // 機器側：コマンドを受けたら XMODEM で受け取る
    let device = thread::spawn(move || {
        expect(&remote, b"xmodem -c flash:fw.bin\r");
        remote.send(b"Ready to receive\r\n");
        let mut rx = zetta_xfer::new_transfer(
            zetta_xfer::Protocol::XModem {
                crc: true,
                one_k: false,
            },
            zetta_xfer::Direction::Receive(out2),
        )
        .unwrap();
        let first = rx.start(Instant::now());
        remote.send(&first);
        let start = Instant::now();
        while !rx.status().is_finished() {
            assert!(start.elapsed() < Duration::from_secs(20), "xmodem stalled");
            let reply = match remote.recv(Duration::from_millis(20)) {
                Some(b) => rx.receive(&b, Instant::now()),
                None => rx.tick(Instant::now()),
            };
            remote.send(&reply);
        }
        remote.send(b"\r\nCopy complete\r\nrommon 2 > ");
        rx.status()
    });
    let src = r#"
timeout = 10
sendln 'xmodem -c flash:fw.bin'
wait 'Ready'
xmodemsend 'fw.bin' 2
int2str s result
messagebox s
wait 'rommon'
messagebox 'done'
"#;
    let mut host = TestHost::default();
    let stop = Arc::new(AtomicBool::new(false));
    let mut it = Interp::new(&mut host, Some(Arc::clone(&session)), stop);
    assert_eq!(it.run_source(src, dir.path()).unwrap(), Outcome::Finished);
    drop(it);
    assert_eq!(device.join().unwrap(), zetta_xfer::Status::Done);
    assert_eq!(*host.out.lock().unwrap(), ["1", "done"]);
    // XMODEM は 128 バイト単位で埋めるので、先頭が一致すればよい
    let got = std::fs::read(&out).unwrap();
    assert_eq!(&got[..firmware.len()], &firmware[..]);
    session.close();
}

#[test]
fn files_include_and_params() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.ttl"),
        "messagebox 'in lib'\nexit\nmessagebox 'no'\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("config.txt"),
        "hostname R1\r\ninterface g0/0\n",
    )
    .unwrap();
    let main = dir.path().join("main.ttl");
    std::fs::write(
        &main,
        r#"
messagebox param2
include 'lib.ttl'
fileopen fh 'config.txt' 0 1
:read
filereadln fh line
if result goto eof
messagebox line
goto read
:eof
fileclose fh
filecreate fh 'out.txt'
filewriteln fh 'written'
fileclose fh
filesearch 'out.txt'
int2str s result
messagebox s
inputbox 'name?' 'title'
messagebox inputstr
inputbox 'again?' 'title'
int2str s result
messagebox s
end
messagebox 'not reached'
"#,
    )
    .unwrap();
    let mut host = TestHost {
        answers: vec!["kt".into()],
        ..Default::default()
    };
    let stop = Arc::new(AtomicBool::new(false));
    let mut it = Interp::new(&mut host, None, stop);
    it.run_file(&main, &["arg1".into()]).unwrap();
    drop(it);
    assert_eq!(
        *host.out.lock().unwrap(),
        [
            "arg1",
            "in lib",
            "hostname R1",
            "interface g0/0",
            "1",
            "kt",
            "0"
        ]
    );
    assert_eq!(
        std::fs::read(dir.path().join("out.txt")).unwrap(),
        b"written\r\n"
    );
}

#[test]
fn stop_interrupts_a_wait() {
    let (session, _remote) = open_mock();
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&stop);
    let t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        s2.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let mut host = TestHost::default();
    let mut it = Interp::new(&mut host, Some(Arc::clone(&session)), stop);
    let started = Instant::now();
    assert_eq!(
        it.run_source("wait 'forever'", Path::new(".")).unwrap(),
        Outcome::Stopped
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    t.join().unwrap();
    session.close();
}

/// 外部レビュー B-1：1 行の if を重ねた break、i32 の端の for、切断後の無期限 wait。
#[test]
fn nested_one_line_if_break_and_for_at_the_edge() {
    let src = r#"
n = 0
while 1
  n = n + 1
  if n > 2 if n < 100 break
endwhile
int2str s n
messagebox s
c = 0
for i 2147483645 2147483647
  c = c + 1
next
int2str s c
messagebox s
c = 0
lo = -2147483646
hi = -2147483647 - 1
for i lo hi
  c = c + 1
next
int2str s c
messagebox s
"#;
    let mut host = TestHost::default();
    assert_eq!(run(src, &mut host, None), ["3", "3", "3"]);
}

#[test]
fn wait_ends_when_the_peer_goes_away() {
    let (session, remote) = open_mock();
    let t = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        remote.hang_up();
    });
    let mut host = TestHost::default();
    let stop = Arc::new(AtomicBool::new(false));
    let mut it = Interp::new(&mut host, Some(Arc::clone(&session)), stop);
    let started = Instant::now();
    // timeout = 0（無期限）でも、切れたら result = 0 で次へ進む
    it.run_source(
        "wait 'never'\nint2str s result\nmessagebox s",
        Path::new("."),
    )
    .unwrap();
    drop(it);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(*host.out.lock().unwrap(), ["0"]);
    t.join().unwrap();
}

#[test]
fn paths_checksums_logs_and_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let fw = dir.path().join("fw.bin");
    std::fs::write(&fw, b"123456789").unwrap();
    let log = dir.path().join("m.log");
    let src = format!(
        r#"
basename b '/var/log/messages'
dirname d '/var/log/messages'
makepath p 'conf' 'r1.txt'
messagebox b
messagebox d
messagebox p
crc32 c '123456789'
sprintf2 h '%08X' c
messagebox h
crc32file c '{fw}'
sprintf2 h '%08X %d' c result
messagebox h
crc32file c 'missing.bin'
int2str r result
messagebox r
loginfo f
int2str r result
messagebox r
logopen '{log}' 0 0
logwrite 'first'
logpause
loginfo f
int2str r result
messagebox r
logstart
logwrite 'second'
logclose
setexitcode 3
"#,
        fw = fw.display(),
        log = log.display(),
    );
    let (session, _remote) = open_mock();
    let mut host = TestHost::default();
    let stop = Arc::new(AtomicBool::new(false));
    let mut it = Interp::new(&mut host, Some(session), stop);
    it.run_source(&src, dir.path()).unwrap();
    assert_eq!(it.exit_code(), 3);
    drop(it);
    let out = host.out.lock().unwrap().clone();
    assert_eq!(
        out,
        [
            "messages",
            "/var/log",
            &format!("conf{}r1.txt", std::path::MAIN_SEPARATOR),
            "CBF43926",
            "CBF43926 0",
            "-1",
            "-1",
            "1",
        ]
    );
    // logpause の間も logwrite は書く（コメントは中断中でも残す）
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "first\nsecond\n");
}

#[test]
fn serial_lines_baud_echo_and_more_waits() {
    let (session, remote) = open_mock();
    remote.set_modem_lines(true, false, false, true);
    let r = remote;
    let device = thread::spawn(move || {
        expect(&r, b"go\r");
        r.send(b"boot: OK\r\n");
        r.send(b"ready> ");
        r.send(b"ABCDEFG");
        r
    });
    let src = r#"
timeout = 5
setbaud 115200
int2str s result
messagebox s
setflowctrl 2
getmodemstatus m
sprintf2 s '%02X %d' m result
messagebox s
setdtr 0
setecho 1
sendln 'go'
wait4all 'ready>' 'boot:'
int2str s result
messagebox s
waitn 3
messagebox inputstr
mtimeout = 200
timeout = 0
wait4all 'X' 'Y'
int2str s result
messagebox s
clearscreen 1
"#;
    let mut host = TestHost::default();
    let out = run(src, &mut host, Some(Arc::clone(&session)));
    let remote = device.join().unwrap();
    assert_eq!(out, ["0", "90 0", "1", " AB", "0"]);
    let last = remote.serial_settings().last().copied().unwrap();
    assert_eq!(last.baud, 115_200);
    assert_eq!(last.flow, zetta_core::FlowControl::RtsCts);
    assert_eq!(remote.output_lines(), (false, true));
    assert!(session.terminal_settings().local_echo);
    session.close();
}

#[test]
fn file_folder_checksum_and_password_commands() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();
    let src = r#"
foldercreate 'sub/deep'
foldersearch 'sub/deep'
int2str s result
messagebox s
filecopy 'a.txt' 'sub/b.txt'
fileconcat 'sub/b.txt' 'a.txt'
filestat 'sub/b.txt' size
int2str s size
messagebox s
filerename 'sub/b.txt' 'sub/c.txt'
filesearch 'sub/c.txt'
int2str s result
messagebox s
fileopen h 'sub/c.txt' 0 1
fileread h 3 t
messagebox t
filemarkptr h
filereadln h t
messagebox t
fileseekback h
fileread h 2 t
messagebox t
off = -6
fileseek h off 2
filereadln h t
messagebox t
filelock h 100
int2str s result
messagebox s
fileunlock h
fileclose h
filetruncate 'sub/c.txt' 2
filestat 'sub/c.txt' size
int2str s size
messagebox s
crc16 c '123456789'
checksum8 k '123456789'
sprintf2 s '%04X %02X' c k
messagebox s
folderdelete 'sub/deep'
foldersearch 'sub/deep'
int2str s result
messagebox s
ispassword 'pw.dat' 'r1'
int2str s result
messagebox s
setpassword 'pw.dat' 'r1' 'p@ss'
getpassword 'pw.dat' 'r1' p
messagebox p
getpassword 'pw.dat' 'r2' p
messagebox p
delpassword 'pw.dat' 'r1'
ispassword 'pw.dat' 'r1'
int2str s result
messagebox s
basename n 'x/y.ttl'
getver v 'CURRENT_VERSION'
int2str s result
messagebox s
"#;
    // getver は今の版と比べる（同じなら 0）
    let src = &src.replace("CURRENT_VERSION", env!("CARGO_PKG_VERSION"));
    let mut host = TestHost {
        answers: vec!["asked".into()],
        ..Default::default()
    };
    let stop = Arc::new(AtomicBool::new(false));
    let mut it = Interp::new(&mut host, None, stop);
    it.run_source(src, dir.path()).unwrap();
    drop(it);
    let out = host.out.lock().unwrap().clone();
    assert_eq!(
        out,
        [
            "1", "24", "1", "hel", "lo", "lo", "world", "0", "2", "906E DD", "0", "0", "p@ss",
            "asked", "0", "0",
        ]
    );
    // 尋ねた r2 は保存されている
    assert!(
        std::fs::read_to_string(dir.path().join("pw.dat"))
            .unwrap()
            .contains("r2=")
    );
}

#[test]
fn dialogs_clipboard_and_broadcast_go_through_the_host() {
    let (session, remote) = open_mock();
    let src = r#"
strdim items 3
items[0] = 'r1'
items[1] = 'r2'
items[2] = 'r3'
listbox 'pick' 'routers' items 1
int2str s result
messagebox s
filenamebox 'save' 1
messagebox inputstr
dirnamebox 'dir'
int2str s result
messagebox s
var2clipb 'copied text'
clipb2var c
messagebox c
sendlnbroadcast 'show ' 'clock'
scpsend 'a' 'b'
int2str s result
messagebox s
"#;
    let mut host = TestHost::default();
    let out = run(src, &mut host, Some(Arc::clone(&session)));
    assert_eq!(out[0], "list:pick:r1,r2,r3:1");
    assert_eq!(out[1], "2");
    assert!(out[2].ends_with("out.txt"), "{out:?}");
    // dirnamebox は Host が答えない（取り消し）
    assert_eq!(&out[3..], ["0", "copied text", "1"]);
    // 一斉送信は Host が引き受けないので、マクロの接続に送る
    expect(&remote, b"show clock\r");
    session.close();
}

#[test]
fn clipb2var_does_not_cut_characters_in_half() {
    let (session, _remote) = open_mock();
    // 3 バイトの文字が 200 個（600 バイト）。511 バイトで切ると文字の途中になる
    let text = "あ".repeat(200);
    let src = r#"
clipb2var a 0
int2str r result
messagebox r
messagebox a
clipb2var b 1
int2str r result
messagebox r
messagebox b
"#;
    let mut host = TestHost {
        clip: Some(text.clone()),
        ..Default::default()
    };
    let out = run(src, &mut host, Some(Arc::clone(&session)));
    assert_eq!(out[0], "2");
    assert_eq!(out[2], "1");
    assert!(!out[1].contains('\u{FFFD}') && !out[3].contains('\u{FFFD}'));
    assert_eq!(format!("{}{}", out[1], out[3]), text);
    session.close();
}

#[test]
fn local_addresses_keys_and_window_queries() {
    let (session, remote) = open_mock();
    let src = r#"
strdim a 1
getipv4addr a n
sprintf2 s '%d %d %s' result n a[0]
messagebox s
strdim b 4
getipv6addr b n
sprintf2 s '%d %d %s' result n b[0]
messagebox s
sendkcode 336 2
sendkcode 47 1
getttpos f wx wy ww wh cx cy cw ch
int2str s result
messagebox s
setdlgpos 10 20 6 1 2
restoresetup 'nothing.ini'
int2str s result
messagebox s
"#;
    let mut host = TestHost::default();
    let out = run(src, &mut host, Some(Arc::clone(&session)));
    assert_eq!(out[0], "0 2 127.0.0.1");
    assert_eq!(out[1], "1 1 ::1");
    expect(&remote, b"\x1b[B\x1b[Bv");
    // 答えない Host では -1 と 0
    assert_eq!(&out[2..], ["-1", "0"]);
    session.close();
}

#[test]
fn logrotate_and_logautoclosemode() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("m.log");
    let (session, _remote) = open_mock();
    let src = format!(
        r#"
logopen '{}' 0 0
logrotate 'size' '32K'
logrotate 'rotate' 3
logrotate 'halt'
logautoclosemode 1
"#,
        log.display()
    );
    let mut host = TestHost::default();
    run(&src, &mut host, Some(Arc::clone(&session)));
    // 最後まで行ったので閉じている
    assert!(!session.is_logging());
    // 小さすぎる大きさは誤り
    let stop = Arc::new(AtomicBool::new(false));
    let mut it = Interp::new(&mut host, Some(Arc::clone(&session)), stop);
    let src = format!("logopen '{}' 0 0\nlogrotate 'size' 100\n", log.display());
    assert!(it.run_source(&src, Path::new(".")).is_err());
    drop(it);
    session.close();
}

#[test]
fn waitevent_times_out_or_sees_a_disconnect() {
    let (session, remote) = open_mock();
    let mut host = TestHost::default();
    let out = run(
        "timeout = 0\nmtimeout = 100\nwaitevent 1 or 8\nint2str s result\nmessagebox s\n",
        &mut host,
        Some(Arc::clone(&session)),
    );
    assert_eq!(out, ["1"]);
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        remote.hang_up();
    });
    let mut host = TestHost::default();
    let out = run(
        "timeout = 5\nwaitevent 1 or 4\nint2str s result\nmessagebox s\n",
        &mut host,
        Some(Arc::clone(&session)),
    );
    t.join().unwrap();
    assert_eq!(out, ["4"]);
}

#[test]
fn pause_holds_the_macro_until_resumed() {
    use zetta_ttl::MacroControl;
    let control = Arc::new(MacroControl::default());
    control
        .paused
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let c = Arc::clone(&control);
    let worker = std::thread::spawn(move || {
        let mut host = TestHost::default();
        let stop = Arc::new(AtomicBool::new(false));
        let mut it = Interp::new(&mut host, None, stop);
        it.set_control(c);
        it.run_source("x = 1\nmessagebox 'done' 't'\n", Path::new("."))
            .unwrap();
        drop(it);
        host.out.lock().unwrap().clone()
    });
    std::thread::sleep(Duration::from_millis(200));
    // 中断中は 1 行目で止まっている
    assert_eq!(control.line.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!worker.is_finished());
    control
        .paused
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let out = worker.join().unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(control.line.load(std::sync::atomic::Ordering::SeqCst), 2);
}
