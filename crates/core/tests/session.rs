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

#[test]
fn lf_only_device_lines_start_at_column_zero_in_lf_mode() {
    use mt_core::{NewlineReceive, TerminalSettings};
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_terminal_settings(TerminalSettings {
        newline_receive: NewlineReceive::Lf,
        ..Default::default()
    });
    remote.send(b"first\nsecond\n");
    assert!(wait_until(
        || screen_text(&session).starts_with("first\nsecond")
    ));
}

#[test]
fn enter_uses_send_newline_and_local_echo_shows_input() {
    use mt_core::{NewlineSend, TerminalSettings};
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_terminal_settings(TerminalSettings {
        newline_send: NewlineSend::CrLf,
        local_echo: true,
        ..Default::default()
    });
    session.key(Key::Text("ok".into()), Modifiers::default());
    session.key(Key::Enter, Modifiers::default());
    assert_eq!(remote.recv(WAIT).unwrap(), b"ok");
    assert_eq!(remote.recv(WAIT).unwrap(), b"\r\n");
    assert!(wait_until(|| screen_text(&session).starts_with("ok\n")));
    let s = session.snapshot(0);
    assert_eq!((s.cursor.row, s.cursor.col), (1, 0));

    session.paste("a\nb".into());
    assert_eq!(remote.recv(WAIT).unwrap(), b"a\r\nb");
}

#[test]
fn shift_jis_is_converted_both_ways() {
    use mt_core::{Charset, TerminalSettings};
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_terminal_settings(TerminalSettings {
        charset_receive: Charset::ShiftJis,
        charset_send: Charset::ShiftJis,
        ..Default::default()
    });
    // 「日本」を Shift_JIS で、1 文字目の途中で割って受信する
    remote.send(b"\x93");
    remote.send(b"\xfa\x96\x7b");
    assert!(wait_until(|| screen_text(&session).starts_with("日本")));
    session.key(Key::Text("あ".into()), Modifiers::default());
    assert_eq!(remote.recv(WAIT).unwrap(), b"\x82\xa0");
}

/// 決まった応答をする転送の代役。受信の行き先が切り替わることを確かめる。
struct EchoDivert {
    got: Arc<Mutex<Vec<u8>>>,
    done_after: usize,
    ticks: Arc<AtomicUsize>,
}

impl mt_core::ByteDivert for EchoDivert {
    fn start(&mut self) -> Vec<u8> {
        b"START".to_vec()
    }
    fn receive(&mut self, data: &[u8]) -> Vec<u8> {
        self.got.lock().unwrap().extend_from_slice(data);
        b"ACK".to_vec()
    }
    fn tick(&mut self) -> Vec<u8> {
        self.ticks.fetch_add(1, Ordering::SeqCst);
        Vec::new()
    }
    fn cancel(&mut self) -> Vec<u8> {
        b"CANCEL".to_vec()
    }
    fn finished(&self) -> bool {
        self.got.lock().unwrap().len() >= self.done_after
    }
}

#[test]
fn transfer_diverts_received_bytes_until_finished() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    let got = Arc::new(Mutex::new(Vec::new()));
    let ticks = Arc::new(AtomicUsize::new(0));
    session
        .start_transfer(Box::new(EchoDivert {
            got: got.clone(),
            done_after: 6,
            ticks: ticks.clone(),
        }))
        .unwrap();
    assert_eq!(remote.recv(WAIT).unwrap(), b"START");
    assert!(session.is_transferring());
    // 2 つ目は始められない
    assert!(
        session
            .start_transfer(Box::new(EchoDivert {
                got: got.clone(),
                done_after: 1,
                ticks: ticks.clone()
            }))
            .is_err()
    );

    // 見回り（tick）が回ることも確かめる
    std::thread::sleep(Duration::from_millis(100));
    remote.send(b"abc");
    assert_eq!(remote.recv(WAIT).unwrap(), b"ACK");
    remote.send(b"def");
    assert_eq!(remote.recv(WAIT).unwrap(), b"ACK");
    assert!(wait_until(|| !session.is_transferring()));
    assert!(ticks.load(Ordering::SeqCst) > 0);
    // 転送中の受信は画面に出ていない。終わったら端末に戻る
    assert!(!screen_text(&session).contains("abc"));
    remote.send(b"back");
    assert!(wait_until(|| screen_text(&session).contains("back")));
}

#[test]
fn cancel_transfer_sends_the_cancel_sequence() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    let got = Arc::new(Mutex::new(Vec::new()));
    session
        .start_transfer(Box::new(EchoDivert {
            got,
            done_after: 100,
            ticks: Arc::default(),
        }))
        .unwrap();
    assert_eq!(remote.recv(WAIT).unwrap(), b"START");
    session.cancel_transfer();
    assert_eq!(remote.recv(WAIT).unwrap(), b"CANCEL");
    assert!(!session.is_transferring());
}

#[test]
fn zmodem_start_is_detected() {
    #[derive(Default)]
    struct Requests(Mutex<Vec<mt_core::TransferRequest>>);
    impl SessionListener for Requests {
        fn on_damage(&self) {}
        fn on_state(&self, _: SessionState) {}
        fn on_bell(&self) {}
        fn on_title(&self, _: String) {}
        fn on_transfer_request(&self, r: mt_core::TransferRequest) {
            self.0.lock().unwrap().push(r);
        }
    }
    let (transport, remote) = MockTransport::pair();
    let listener = Arc::new(Requests::default());
    let options = SessionOptions {
        size: WindowSize::new(40, 4),
        ..Default::default()
    };
    let session = Session::open(Box::new(transport), options, listener.clone()).unwrap();
    remote.send(b"$ sz file\r\nrz\r**\x18B00000000000000\r\x8a\x11");
    assert!(wait_until(|| !listener.0.lock().unwrap().is_empty()));
    assert_eq!(
        *listener.0.lock().unwrap(),
        [mt_core::TransferRequest::ZmodemReceive]
    );
    // ヘッダより手前は画面に出て、ヘッダは出ない
    assert!(wait_until(|| screen_text(&session).contains("$ sz file")));
    assert!(!screen_text(&session).contains("B0000"));
}

/// ファイル送信：バイナリは 1 バイトも変えずに送り、テキストは改行を「送信」の改行コードに揃える。
#[test]
fn send_file_binary_is_untouched_and_text_follows_newline_setting() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let options = SessionOptions {
        size: WindowSize::new(20, 4),
        terminal: mt_core::TerminalSettings {
            newline_send: mt_core::NewlineSend::CrLf,
            charset_send: mt_core::Charset::ShiftJis,
            ..Default::default()
        },
        ..Default::default()
    };
    let session = Session::open(Box::new(transport), options, rec.clone()).unwrap();

    let firmware: Vec<u8> = (0..=255u8)
        .chain(b"\r\n\n\x11\x13".iter().copied())
        .collect();
    session.send_file(firmware.clone(), true);
    let mut got = Vec::new();
    assert!(wait_until(|| {
        while let Some(b) = remote.recv(Duration::from_millis(10)) {
            got.extend(b);
        }
        got.len() >= firmware.len()
    }));
    assert_eq!(got, firmware);

    session.send_file("a\nb\r\n日本".as_bytes().to_vec(), false);
    let mut got = Vec::new();
    let want = [&b"a\r\nb\r\n"[..], &[0x93, 0xfa, 0x96, 0x7b]].concat();
    assert!(wait_until(|| {
        while let Some(b) = remote.recv(Duration::from_millis(10)) {
            got.extend(b);
        }
        got.len() >= want.len()
    }));
    assert_eq!(got, want);
    session.close();
}

/// 外部レビュー S-3：ZMODEM のヘッダから後ろ（同じ受信の塊と、返事までに届いた分）を取りこぼさない。
#[test]
fn zmodem_bytes_after_the_header_go_to_the_transfer_or_back_to_the_screen() {
    struct Recording(Arc<Mutex<Vec<u8>>>);
    impl mt_core::ByteDivert for Recording {
        fn start(&mut self) -> Vec<u8> {
            Vec::new()
        }
        fn receive(&mut self, data: &[u8]) -> Vec<u8> {
            self.0.lock().unwrap().extend_from_slice(data);
            Vec::new()
        }
        fn tick(&mut self) -> Vec<u8> {
            Vec::new()
        }
        fn cancel(&mut self) -> Vec<u8> {
            Vec::new()
        }
        fn finished(&self) -> bool {
            false
        }
    }
    let rec = Arc::new(Recorder::default());
    let (transport, remote) = MockTransport::pair();
    let session = open(transport, &rec);
    remote.send(b"before**\x18B00000000000000\r\x8a\x11");
    remote.send(b"MORE");
    assert!(wait_until(|| screen_text(&session).contains("before")));
    std::thread::sleep(Duration::from_millis(100));
    assert!(!screen_text(&session).contains("MORE"));
    let got = Arc::new(Mutex::new(Vec::new()));
    session
        .start_transfer(Box::new(Recording(Arc::clone(&got))))
        .unwrap();
    remote.send(b"AFTER");
    assert!(wait_until(|| got.lock().unwrap().ends_with(b"AFTER")));
    assert_eq!(
        *got.lock().unwrap(),
        [&b"**\x18B00000000000000\r\x8a\x11"[..], b"MORE", b"AFTER"].concat()
    );
    session.cancel_transfer();

    // 応じない（decline）ときは、とっておいた分を画面に出す
    let rec = Arc::new(Recorder::default());
    let (transport, remote) = MockTransport::pair();
    let session = open(transport, &rec);
    remote.send(b"x**\x18B00000000000000\r\x8a\x11 tail");
    assert!(wait_until(|| screen_text(&session).starts_with('x')));
    session.decline_transfer();
    assert!(wait_until(|| screen_text(&session).contains("tail")));
}

/// 外部レビュー B-2：JIS（ISO-2022-JP）でも、テキストのログは UTF-8 で読める形になる。
#[test]
fn text_log_is_utf8_even_with_jis() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jis.log");
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let options = SessionOptions {
        size: WindowSize::new(20, 4),
        terminal: mt_core::TerminalSettings {
            charset_receive: mt_core::Charset::Jis,
            ..Default::default()
        },
        ..Default::default()
    };
    let session = Session::open(Box::new(transport), options, rec.clone()).unwrap();
    session.log_start(&LogOptions::new(&path)).unwrap();
    // 「日本」を JIS で
    remote.send(b"A\x1b$BF|K\\\x1b(B\r\n");
    assert!(wait_until(|| screen_text(&session).contains("日本")));
    session.log_stop();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "A日本\n");
    session.close();
}

/// 外部レビュー B-5：ローカルエコーとブラケットペーストが両方有効でも、画面には貼り付けた文字だけが出る。
#[test]
fn paste_echo_does_not_show_bracketed_paste_markers() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_terminal_settings(mt_core::TerminalSettings {
        local_echo: true,
        ..Default::default()
    });
    remote.send(b"\x1b[?2004h");
    std::thread::sleep(Duration::from_millis(100));
    session.paste("hi".into());
    assert_eq!(remote.recv(WAIT).unwrap(), b"\x1b[200~hi\x1b[201~");
    assert!(wait_until(|| screen_text(&session).starts_with("hi")));
    assert!(!screen_text(&session).contains("200~"));
}
