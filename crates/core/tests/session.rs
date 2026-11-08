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

/// `want` の長さになるまで受け取ってつなげる。
fn recv_until(remote: &mt_core::mock::MockRemote, want: &[u8]) -> Vec<u8> {
    let mut got = Vec::new();
    let start = Instant::now();
    while got.len() < want.len() && start.elapsed() < WAIT {
        if let Some(b) = remote.recv(Duration::from_millis(20)) {
            got.extend(b);
        }
    }
    got
}

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
    // 貼り付けは行ごとに分かれて届く（PasteDelayPerLine）
    assert_eq!(recv_until(&remote, b"a\rb"), b"a\rb");
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
    // 貼り付けは行ごとに分かれて届く（PasteDelayPerLine）
    assert_eq!(recv_until(&remote, b"a\r\nb"), b"a\r\nb");
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

/// 第 2 回レビュー N-8：返事が無ければ、その後の受信が無くても時間切れで画面に出す。
/// 返事を待つ間に相手が開始を送り直しても、もう一度は知らせない。
#[test]
fn held_zmodem_bytes_flush_on_their_own_and_resends_are_not_asked_again() {
    #[derive(Default)]
    struct Requests(Mutex<usize>);
    impl SessionListener for Requests {
        fn on_damage(&self) {}
        fn on_state(&self, _: SessionState) {}
        fn on_bell(&self) {}
        fn on_title(&self, _: String) {}
        fn on_transfer_request(&self, _: mt_core::TransferRequest) {
            *self.0.lock().unwrap() += 1;
        }
    }
    let (transport, remote) = MockTransport::pair();
    let listener = Arc::new(Requests::default());
    let options = SessionOptions {
        size: WindowSize::new(40, 4),
        ..Default::default()
    };
    let session = Session::open(Box::new(transport), options, listener.clone()).unwrap();
    remote.send(b"x**\x18B00000000000000\r\x8a\x11 tail");
    assert!(wait_until(|| *listener.0.lock().unwrap() == 1));
    let start = Instant::now();
    while !screen_text(&session).contains("tail") && start.elapsed() < Duration::from_secs(8) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(screen_text(&session).contains("tail"));
    remote.send(b"**\x18B00000000000000\r\x8a\x11");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(*listener.0.lock().unwrap(), 1);
    // 返事をしたあとの開始は、また知らせる
    session.decline_transfer();
    remote.send(b"**\x18B00000000000000\r\x8a\x11");
    assert!(wait_until(|| *listener.0.lock().unwrap() == 2));
    session.close();
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

/// LogIncludeScreenBuffer：始める前に画面に出ていた内容を先に書き、中断中の受信は書かない。
#[test]
fn log_can_start_with_the_screen_and_be_paused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("screen.log");
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    remote.send(b"before\r\n");
    assert!(wait_until(|| screen_text(&session).contains("before")));
    let mut options = LogOptions::new(&path);
    options.include_screen = true;
    session.log_start(&options).unwrap();
    session.log_write("# note").unwrap();
    session.log_pause(true);
    remote.send(b"skipped\r\n");
    assert!(wait_until(|| screen_text(&session).contains("skipped")));
    session.log_pause(false);
    remote.send(b"after\r\n");
    assert!(wait_until(|| screen_text(&session).contains("after")));
    std::thread::sleep(Duration::from_millis(50));
    let status = session.log_status().unwrap();
    assert!(!status.paused);
    session.log_stop();
    assert!(session.log_status().is_none());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "before\n# note\nafter\n");
    assert_eq!(status.bytes, text.len() as u64);
    session.close();
}

/// 接続の直後に始めたログ（起動引数の /L= や自動開始）：それまでに届いた分をカーソルまで書き、
/// 続きの受信がプロンプトの行につながる（Tera Term は最初から記録するので同じ内容になる）。
#[test]
fn log_started_right_after_connecting_continues_the_prompt_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connect.log");
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    remote.send(b"Welcome\r\n$ ");
    assert!(wait_until(|| screen_text(&session).contains("$")));
    let mut options = LogOptions::new(&path);
    options.include_screen = true;
    session.log_start(&options).unwrap();
    remote.send(b"ls\r\nfile\r\n$ ");
    assert!(wait_until(|| screen_text(&session).contains("file")));
    std::thread::sleep(Duration::from_millis(50));
    session.log_stop();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "Welcome\n$ ls\nfile\n$ "
    );
    session.close();
}

/// TTXViewMode：閲覧専用の間はキー入力と貼り付けを送らない。TTXRecurringCommand：一定間隔で送る。
#[test]
fn view_only_and_recurring_command() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_view_only(true);
    assert!(session.is_view_only());
    session.key(Key::Text("x".into()), Modifiers::default());
    session.paste("y".into());
    assert!(remote.recv(Duration::from_millis(200)).is_none());
    session.set_view_only(false);
    session.key(Key::Text("z".into()), Modifiers::default());
    assert_eq!(remote.recv(WAIT).unwrap(), b"z");

    session.set_recurring(Some((Duration::from_secs(1), "keep".into(), true)));
    let start = std::time::Instant::now();
    let mut got = Vec::new();
    while start.elapsed() < Duration::from_millis(2600) {
        if let Some(b) = remote.recv(Duration::from_millis(100)) {
            got.extend(b);
        }
    }
    assert_eq!(String::from_utf8_lossy(&got), "keep\rkeep\r");
    session.set_recurring(None);
    assert!(remote.recv(Duration::from_millis(1300)).is_none());
    session.close();
}

/// ENQ にアンサーバックを返し、キーボードの設定で BS キーの送るものが変わる。
#[test]
fn answerback_and_keyboard_options() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    remote.send(b"\x05");
    assert!(remote.recv(Duration::from_millis(200)).is_none());
    session.set_answerback(Some("RTR-01".into()));
    remote.send(b"x\x05");
    assert_eq!(remote.recv(WAIT).unwrap(), b"RTR-01");
    session.key(Key::Backspace, Modifiers::default());
    assert_eq!(remote.recv(WAIT).unwrap(), [0x7f]);
    session.set_keyboard(mt_core::KeyboardOptions {
        backspace_sends_bs: true,
        ..Default::default()
    });
    session.key(Key::Backspace, Modifiers::default());
    assert_eq!(remote.recv(WAIT).unwrap(), [0x08]);
    session.close();
}

/// 貼り付けは行ごとに待つ（Tera Term の PasteDelayPerLine）。中身は変わらない。
#[test]
fn paste_waits_between_lines() {
    let (transport, remote) = MockTransport::pair();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_paste_delay(80);
    let start = Instant::now();
    session.paste("a\nb\nc".into());
    let mut got = Vec::new();
    assert!(wait_until(|| {
        while let Some(b) = remote.recv(Duration::from_millis(10)) {
            got.extend(b);
        }
        got.ends_with(b"c")
    }));
    assert_eq!(got, b"a\rb\rc");
    // 行の終わりは 2 つなので、少なくとも 2 回待つ
    assert!(start.elapsed() >= Duration::from_millis(160));
    session.close();
}

/// 自動再接続を切っているとき、抜かれたら送信口も手放す（シリアルの名前を空ける。Tera Term も閉じる）。
#[test]
fn unplugged_without_auto_reconnect_releases_the_port() {
    let (transport, remote) = MockTransport::exclusive();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_auto_reconnect(false);
    assert_eq!(remote.open_writers(), 1);
    remote.unplug();
    assert!(wait_until(|| session.state() == SessionState::Closed));
    assert!(wait_until(|| remote.open_writers() == 0));
    session.close();
}

/// 「ポートのリセット」はシリアルのように排他で開くポートでも開き直せる（前の送信口を手放す）。
#[test]
fn reset_port_reopens_an_exclusive_port() {
    let (transport, remote) = MockTransport::exclusive();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.reset_port().unwrap();
    assert!(wait_until(|| remote.reconnects() == 1));
    assert!(wait_until(|| session.state() == SessionState::Connected));
    session.write(b"after".to_vec());
    assert_eq!(recv_until(&remote, b"after"), b"after");
    session.close();
}

/// 「ポートのリセット」：自動再接続を切っていても開き直し、そのまま使える。
#[test]
fn reset_port_reopens_the_port() {
    let (transport, remote) = MockTransport::reconnectable();
    let rec = Arc::new(Recorder::default());
    let session = open(transport, &rec);
    session.set_auto_reconnect(false);
    session.reset_port().unwrap();
    assert!(wait_until(|| remote.reconnects() == 1));
    assert!(wait_until(|| session.state() == SessionState::Connected));
    remote.send(b"after reset");
    assert!(wait_until(|| screen_text(&session).contains("after reset")));
    // つなぎ直せない接続はリセットできない
    let (transport, _remote) = MockTransport::pair();
    let other = open(transport, &rec);
    assert!(other.reset_port().is_err());
    session.close();
    other.close();
}
