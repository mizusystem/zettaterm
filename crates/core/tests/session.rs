//! モック Transport を使った Session の結合テスト。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mt_core::mock::MockTransport;
use mt_core::{
    Key, LogOptions, Modifiers, SendDelay, SerialSettings, Session, SessionListener,
    SessionOptions, SessionState, WindowSize,
};

#[derive(Default)]
struct Recorder {
    damage: AtomicUsize,
    states: Mutex<Vec<SessionState>>,
    titles: Mutex<Vec<String>>,
}

impl SessionListener for Recorder {
    fn on_damage(&self) {
        self.damage.fetch_add(1, Ordering::SeqCst);
    }
    fn on_state(&self, state: SessionState) {
        self.states.lock().unwrap().push(state);
    }
    fn on_bell(&self) {}
    fn on_title(&self, title: String) {
        self.titles.lock().unwrap().push(title);
    }
}

const WAIT: Duration = Duration::from_secs(2);

fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < WAIT {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn screen_text(session: &Session) -> String {
    session
        .snapshot(0)
        .lines
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn open(transport: MockTransport, rec: &Arc<Recorder>) -> Arc<Session> {
    let options = SessionOptions {
        size: WindowSize::new(20, 4),
        scrollback: 100,
        ..Default::default()
    };
    Session::open(Box::new(transport), options, rec.clone()).unwrap()
}

#[test]
fn received_bytes_reach_the_screen() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);

    remote.send(b"hello\r\nworld");
    assert!(wait_until(
        || screen_text(&session).starts_with("hello\nworld")
    ));
    assert!(rec.damage.load(Ordering::SeqCst) >= 1);
}

#[test]
fn damage_is_coalesced_until_snapshot() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.snapshot(0);
    let before = rec.damage.load(Ordering::SeqCst);

    for _ in 0..50 {
        remote.send(b"x");
    }
    assert!(wait_until(|| received_x(&session) >= 50));
    // snapshot を呼ぶまでは何度受信しても通知は 1 回
    assert_eq!(rec.damage.load(Ordering::SeqCst), before + 1);

    session.snapshot(0);
    remote.send(b"y");
    assert!(wait_until(
        || rec.damage.load(Ordering::SeqCst) == before + 2
    ));
}

/// 通知フラグに触れずに（snapshot を呼ばずに）受信済みの "x" を数える
fn received_x(session: &Session) -> usize {
    let rows = session.scrollback(0, 4);
    rows.iter()
        .map(|r| r.cells.iter().filter(|c| c.text == "x").count())
        .sum()
}

#[test]
fn keys_and_replies_are_written_back() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);

    session.key(
        Key::Text("c".into()),
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    );
    assert_eq!(remote.recv(WAIT).unwrap(), [0x03]);

    // DSR（カーソル位置要求）への応答
    remote.send(b"\x1b[6n");
    assert_eq!(remote.recv(WAIT).unwrap(), b"\x1b[1;1R");

    session.paste("a\nb".into());
    assert_eq!(remote.recv(WAIT).unwrap(), b"a\rb");
}

#[test]
fn resize_is_forwarded_and_reflected() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);

    session.resize(WindowSize::new(40, 10));
    assert_eq!(remote.last_size(), Some(WindowSize::new(40, 10)));
    let s = session.snapshot(0);
    assert_eq!((s.cols, s.rows), (40, 10));
}

#[test]
fn title_and_close_are_notified() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);

    remote.send(b"\x1b]2;my title\x07");
    assert!(wait_until(|| rec
        .titles
        .lock()
        .unwrap()
        .contains(&"my title".to_owned())));

    session.close();
    assert!(wait_until(|| session.state() == SessionState::Closed));
    assert_eq!(
        *rec.states.lock().unwrap(),
        [SessionState::Connected, SessionState::Closed]
    );
}

#[test]
fn replay_transport_feeds_recorded_data() {
    let rec = Arc::new(Recorder::default());
    let session = open(
        MockTransport::replay(b"\x1b[1mrecorded\x1b[0m".to_vec()),
        &rec,
    );
    assert!(wait_until(|| screen_text(&session).starts_with("recorded")));
    session.close();
}

#[test]
fn unplug_and_plug_in_reconnects() {
    let (transport, remote) = MockTransport::reconnectable();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);

    remote.send(b"before ");
    assert!(wait_until(|| screen_text(&session).contains("before")));

    remote.unplug();
    assert!(wait_until(|| session.state() == SessionState::Reconnecting));
    // 抜けている間はつなぎ直せない
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(remote.reconnects(), 0);

    remote.plug_in();
    assert!(wait_until(|| session.state() == SessionState::Connected));
    assert_eq!(remote.reconnects(), 1);

    // 再接続後も受信・送信できる
    remote.send(b"after");
    assert!(wait_until(|| screen_text(&session).contains("before after")));
    session.write(b"x".to_vec());
    assert_eq!(remote.recv(WAIT).unwrap(), b"x");

    assert_eq!(
        *rec.states.lock().unwrap(),
        [
            SessionState::Connected,
            SessionState::Reconnecting,
            SessionState::Connected
        ]
    );
}

#[test]
fn close_while_reconnecting_ends_the_session() {
    let (transport, remote) = MockTransport::reconnectable();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    remote.unplug();
    assert!(wait_until(|| session.state() == SessionState::Reconnecting));
    session.close();
    assert!(wait_until(|| session.state() == SessionState::Closed));
}

#[test]
fn hang_up_without_reconnect_support_closes() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    remote.hang_up();
    assert!(wait_until(|| session.state() == SessionState::Closed));
}

#[test]
fn send_delay_spaces_out_characters() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_send_delay(SendDelay {
        per_char_ms: 30,
        per_line_ms: 0,
    });

    let start = Instant::now();
    session.write(b"abcd".to_vec());
    let mut got = Vec::new();
    while got.len() < 4 {
        got.extend(remote.recv(WAIT).unwrap());
    }
    assert_eq!(got, b"abcd");
    // 1 文字ずつ送り、各文字の後に 30ms 待つ
    assert!(start.elapsed() >= Duration::from_millis(90));
}

#[test]
fn serial_settings_reach_the_transport() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    let settings = SerialSettings {
        baud: 115_200,
        ..Default::default()
    };
    session.set_serial_settings(&settings).unwrap();
    assert_eq!(remote.serial_settings(), [settings]);
}

#[test]
fn log_records_received_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.log");
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);

    session.log_start(&LogOptions::new(&path)).unwrap();
    assert!(session.is_logging());
    remote.send(b"\x1b[32mhello\x1b[0m\r\n");
    assert!(wait_until(|| std::fs::read_to_string(&path)
        .unwrap_or_default()
        == "hello\n"));

    session.log_stop();
    assert!(!session.is_logging());
    remote.send(b"ignored\r\n");
    assert!(wait_until(|| screen_text(&session).contains("ignored")));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
}
