//! モック Transport を使った Session の結合テスト。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mt_core::mock::MockTransport;
use mt_core::{Key, Modifiers, Session, SessionListener, SessionOptions, SessionState, WindowSize};

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
