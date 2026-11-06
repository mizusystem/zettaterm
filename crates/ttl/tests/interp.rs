//! TTL インタプリタの結合テスト。相手はモック Transport（同じプロセスの中で応答を返す）。

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use mt_core::mock::{MockRemote, MockTransport};
use mt_core::{Session, SessionListener, SessionOptions, SessionState};
use mt_ttl::{ConnectSpec, Host, Interp, Outcome};

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
        let mut rx = mt_xfer::new_transfer(
            mt_xfer::Protocol::XModem {
                crc: true,
                one_k: false,
            },
            mt_xfer::Direction::Receive(out2),
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
    assert_eq!(device.join().unwrap(), mt_xfer::Status::Done);
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
