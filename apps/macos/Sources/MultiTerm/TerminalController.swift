import AppKit
import MTCore
import SwiftUI

/// 1 ウィンドウ（タブ）分の接続とダイアログの状態。メニューからは FocusedObject として届く。
@MainActor
final class TerminalController: ObservableObject {
    enum Status { case disconnected, connecting, connected, reconnecting }

    enum Sheet: String, Identifiable {
        case newConnection, serialSettings, terminalSettings, log, broadcast, auth, transfer, scp, drop
        case recurring
        case copyPasteSettings, pasteConfirm, logSettings, logComment, windowSettings, fontSettings, sshSettings, sshKeyGen, keyboardSettings
        case otherSettings, proxySettings, tcpipSettings
        var id: String { rawValue }
    }

    let view = TerminalView()
    /// 端末内の検索（Cmd+F）
    let search = SearchState()
    /// 確認を待っている貼り付け（sheet == .pasteConfirm のときに使う）
    @Published var pendingPaste: String?
    /// 確認を待っている貼り付けの末尾に改行を足すか（「貼り付け<CR>」）
    private var pendingPasteCR = false
    let id = UUID()
    /// SSH の認証・ホスト鍵確認をこのウィンドウのダイアログで尋ねる
    let prompter = SshPrompterBridge()
    /// 答え待ちの問い合わせ（sheet == .auth のときに出す）
    @Published private(set) var authPrompt: AuthPrompt?
    /// 進行中（か、終わったばかり）のファイル転送
    @Published var transfer: ActiveTransfer?
    /// 実行中のマクロ
    @Published var macro: ActiveMacro?
    /// ドロップされたファイル（sheet == .drop のときに使う）
    @Published var dropped: [URL] = []
    /// マクロの `settitle`
    @Published var macroTitle: String?
    /// マクロの `setmulticastname`（`sendmulticast` の宛先の名前）
    var multicastName: String?

    @Published private(set) var status: Status = .disconnected {
        didSet {
            guard oldValue != status else { return }
            let other = OtherConfig.saved
            let wasUp = oldValue == .connected || oldValue == .reconnecting
            if status == .connected && oldValue != .reconnecting && other.beepOnConnect { NSSound.beep() }
            if status == .disconnected && wasUp {
                if other.beepOnConnect { NSSound.beep() }
                if other.clearOnClose { view.clearClosedScreen() }
            }
        }
    }
    @Published private(set) var isLogging = false
    @Published private(set) var isLogPaused = false
    /// 閲覧専用（TTXViewMode）
    @Published private(set) var viewOnly = false
    /// 繰り返し送信（TTXRecurringCommand）。秒、文字列、改行
    @Published private(set) var recurring: RecurringCommand?
    /// シリアルの信号線（シリアル接続中だけ、0.5 秒ごとに読む）
    @Published private(set) var modemLines: ModemLines?
    private var modemTimer: Timer?
    /// 起動引数の /L= と /M=（接続できてから始める）
    private var launchLog: (String, Bool)?
    private var launchMacro: URL?
    @Published private(set) var remoteTitle = ""
    @Published private(set) var target: ConnectTarget?
    @Published var sheet: Sheet?
    /// シートのために広げる前の端末の桁数と行数（閉じたら戻す）
    var gridBeforeSheet: (cols: Int, rows: Int)?
    @Published var errorMessage: String?
    /// しばらく右上に出す短い知らせ（相手がクリップボードに書いたときなど）
    @Published var notice: String?
    /// 「ログ記録中ダイアログを表示」
    @Published var showLogPanel = false
    private var noticeGen = 0

    func showNotice(_ text: String) {
        noticeGen += 1
        let gen = noticeGen
        withAnimation { notice = text }
        DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
            MainActor.assumeIsolated {
                guard let self, self.noticeGen == gen else { return }
                withAnimation { self.notice = nil }
            }
        }
    }

    @Published private(set) var serialConfig = SerialConfig.saved
    /// 今の接続先（シリアルか、それ以外か）の端末設定
    var terminalConfig: TerminalConfig { TerminalConfig.saved(TerminalConfig.scope(for: target)) }
    /// 何回目の接続か。古い接続の完了通知を捨てるのに使う
    private var attempt = 0

    init() {
        view.onState = { [weak self] state in self?.handle(state) }
        view.onTitle = { [weak self] title in self?.remoteTitle = title }
        view.onError = { [weak self] error in self?.report(error) }
        view.onTransferRequest = { [weak self] request in self?.handleTransferRequest(request) }
        view.onDropFiles = { [weak self] urls in self?.handleDrop(urls) }
        view.onPaste = { [weak self] text, cr in self?.handlePaste(text, appendCR: cr) }
        view.onHistoryShift = { [weak self] delta in self?.search.shift(by: delta) }
        view.prompter = prompter
        prompter.controller = self
        SessionRegistry.shared.register(self)
    }

    var connectionLabel: String { target?.label ?? "" }

    var windowTitle: String {
        let custom = AppearanceConfig.saved.title
        var base = macroTitle ?? OtherConfig.saved.title(remote: remoteTitle, base: connectionLabel)
        // 「設定 > ウィンドウ」のタイトルは前に付ける（空なら Multi-Term）
        if base.isEmpty { base = custom.isEmpty ? "Multi-Term" : custom } else if !custom.isEmpty { base = "\(custom) - \(base)" }
        switch status {
        case .connected: return base
        case .connecting: return L("%@ [接続中]", connectionLabel)
        case .reconnecting: return L("%@ [再接続待ち]", base)
        case .disconnected: return target == nil ? "Multi-Term" : L("%@ [未接続]", base)
        }
    }

    /// 接続中（再接続待ちを含む）のシリアルポート
    var serialPort: String? { status == .disconnected ? nil : target?.serialPort }
    var isSerialConnected: Bool { serialPort != nil }
    var isTelnet: Bool {
        if case .telnet = target, isConnected { return true }
        return false
    }
    var isConnected: Bool { status == .connected || status == .reconnecting }

    // MARK: 接続

    func open(_ request: ConnectRequest?) {
        // 起動引数（ttermpro と同じ書き方）があれば、最初のウィンドウで 1 回だけ使う
        if request == nil, let launch = SettingsImport.pending {
            SettingsImport.pending = nil
            applyLaunch(launch)
            return
        }
        switch request?.target ?? .dialog {
        case .dialog: sheet = .newConnection
        case let .connect(target): connect(target)
        }
    }

    /// 接続する。Telnet / TCP は接続に時間がかかることがあるので、コアの呼び出しは別スレッドで行う。
    /// `remember` なら成功したときにホスト履歴へ足す。
    func connect(_ target: ConnectTarget, remember: Bool = false) {
        view.disconnect()
        attempt += 1
        let current = attempt
        isLogging = false
        remoteTitle = ""
        self.target = target
        status = .connecting
        if case let .serial(port, config) = target {
            config.save()
            SerialConfig.savedPort = port
            serialConfig = config
        }
        view.terminalSettings = TerminalConfig.saved(TerminalConfig.scope(for: target)).settings
        view.answerback = TerminalConfig.saved(TerminalConfig.scope(for: target)).answerback
        view.terminalId = TerminalConfig.saved(TerminalConfig.scope(for: target)).terminalId
        let saved = TerminalConfig.saved(TerminalConfig.scope(for: target))
        view.jis = (saved.kanjiIn, saved.kanjiOut, saved.kana7bit)
        view.onClipboardWritten = { [weak self] text in
            self?.showNotice(L("相手がクリップボードに書き込みました（%@ 文字）", String(text.count)))
        }
        Task {
            do {
                try await view.connect(target.profile)
                prompter.setOneTimeLogin(nil)
                guard current == attempt else { return }
                status = .connected
                // 始められなかったポート転送など（接続はできている）
                if let warnings = view.session?.warnings(), !warnings.isEmpty {
                    errorMessage = L("接続しましたが、次はうまくいきませんでした。\n%@", warnings.joined(separator: "\n"))
                }
                if remember { HostStore.shared.add(target) }
                autoStartLog()
                startModemPolling()
                // つなぎ直しても、閲覧専用と繰り返し送信はそのまま
                if viewOnly { view.session?.setViewOnly(on: true) }
                if let r = recurring { setRecurring(r) }
                startLaunchExtras()
            } catch is CancellationError {
                prompter.setOneTimeLogin(nil)
                return
            } catch {
                prompter.setOneTimeLogin(nil)
                guard current == attempt else { return }
                status = .disconnected
                report(error)
            }
            focusTerminal()
        }
    }

    /// 起動引数の /W=（タイトル）、接続先、/L=（ログ）、/M=（マクロ）。
    private func applyLaunch(_ l: LaunchOptions) {
        if let t = l.title { macroTitle = t }
        launchLog = l.logFile.map { ($0, l.logAppend) }
        launchMacro = l.macroFile.map { URL(fileURLWithPath: $0) }
        if let c = l.connect, let target = SettingsImport.target(c) {
            if case let .ssh(_, _, _, password, auth, keyFile) = c {
                prompter.setOneTimeLogin(LoginPreset(auth: auth, keyFile: keyFile, password: password))
            }
            connect(target)
        } else {
            if l.connect != nil { errorMessage = L("起動引数の接続先はこの OS では開けません。") }
            if let m = launchMacro {
                launchMacro = nil
                runMacro(m)
            } else {
                sheet = .newConnection
            }
        }
    }

    /// 起動引数の接続が終わったら始めるログとマクロ。
    private func startLaunchExtras() {
        if let (path, append) = launchLog {
            launchLog = nil
            // 接続の直後に始める。それまでに届いた分（歓迎の表示など）は画面から書く（Tera Term は最初から記録する）
            var options = LogOptionsView.savedOptions(path: path, includeScreen: true)
            options.append = append
            startLog(options)
        }
        if let m = launchMacro {
            launchMacro = nil
            runMacro(m)
        }
    }

    /// 今の接続先を、新しいタブでもう 1 つ開く（Tera Term の「セッションの複製」）。
    var duplicateRequest: ConnectRequest? {
        target.map { ConnectRequest(target: .connect($0), asTab: true) }
    }

    /// 「現在の接続を再設定」。つないだまま通信設定と送信遅延を変える。
    func reconfigureSerial(_ config: SerialConfig) {
        guard let session = view.session, let port = serialPort else { return }
        do {
            try session.setSerialSettings(settings: config.settings)
            serialConfig = config
            config.save()
            target = .serial(port: port, config: config)
        } catch {
            report(error)
        }
    }

    /// 「設定 > 端末」。今の接続先の種類（シリアル / それ以外）に保存して、接続中ならその場で効かせる。
    func applyTerminal(_ config: TerminalConfig) {
        config.save(TerminalConfig.scope(for: target))
        objectWillChange.send()
        view.terminalSettings = config.settings
        view.session?.setTerminalSettings(settings: config.settings)
        view.answerback = config.answerback
        view.session?.setAnswerback(text: config.answerback)
        view.terminalId = config.terminalId
        view.session?.setTerminalId(id: config.terminalId)
        view.jis = (config.kanjiIn, config.kanjiOut, config.kana7bit)
        view.session?.setJisOptions(kanjiIn: config.kanjiIn, kanjiOut: config.kanjiOut, kana7bit: config.kana7bit)
    }

    /// 「ファイル > 接続断」。「その他の設定」で確かめる設定なら先に尋ねる。
    func requestDisconnect() {
        if OtherConfig.saved.confirmDisconnect && isConnected {
            let alert = NSAlert()
            alert.messageText = TT.text("MSG_DISCONNECT_CONF", "切断しますか？")
            alert.addButton(withTitle: TT.text("BTN_OK", "OK"))
            alert.addButton(withTitle: TT.text("BTN_CANCEL", "キャンセル"))
            guard alert.runModal() == .alertFirstButtonReturn else { return }
        }
        disconnect()
    }

    /// 「設定 > キーマップ読み込み」：KEYBOARD.CNF の [User keys] を、開いている接続とこれからの接続に使う。
    func loadKeymap() {
        let panel = NSOpenPanel()
        panel.message = L("Tera Term のキーボード設定ファイル（KEYBOARD.CNF）を選んでください。")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        UserDefaults.standard.set(url.path, forKey: "keymap.path")
        var count: UInt32 = 0
        for c in SessionRegistry.shared.controllers {
            if let n = try? c.view.session?.setKeymap(path: url.path) { count = n }
        }
        if let probe = try? Data(contentsOf: url), !probe.isEmpty {
            showNotice(L("キーマップを読み込みました（%@ 個のキー）", String(count)))
        }
    }

    /// 「コントロール > ポートのリセット」：シリアルポートを閉じて開き直す。
    func resetPort() {
        guard isSerialConnected, let session = view.session else { return }
        do {
            try session.resetPort()
        } catch {
            errorMessage = Self.describe(error)
        }
    }

    func disconnect() {
        authSheetDismissed()
        attempt += 1
        view.disconnect()
        status = .disconnected
        isLogging = false
        isLogPaused = false
        stopModemPolling()
    }

    func sendBreak() {
        guard let session = view.session else { return }
        let ms = SerialConfig.breakMs
        // ブレーク中は待つので、メインスレッドを止めない
        Task {
            let message: String? = await Task.detached {
                do {
                    try session.sendBreak(ms: ms)
                    return nil
                } catch {
                    return TerminalController.describe(error)
                }
            }.value
            if let message { errorMessage = message }
        }
    }

    /// ブロードキャストの 1 件分。文字列をそのまま送り、`enter` なら続けて Enter を送る。
    func broadcast(_ text: String, enter: Bool) {
        guard let session = view.session, isConnected else { return }
        if !text.isEmpty { session.write(bytes: Data(text.utf8)) }
        if enter { session.key(key: .enter, mods: Modifiers(shift: false, ctrl: false, alt: false)) }
    }

    /// ブロードキャストのリアルタイム送信で、消した文字の分の BackSpace を送る。
    func broadcastBackspace(_ count: Int) {
        guard let session = view.session, isConnected, count > 0 else { return }
        for _ in 0..<count { session.key(key: .backspace, mods: Modifiers(shift: false, ctrl: false, alt: false)) }
    }

    // MARK: 貼り付け・検索・画面

    /// 改行や危険な語を含む貼り付けは、送る前に確かめる（Tera Term の ConfirmChangePaste）。
    func handlePaste(_ text: String, appendCR: Bool) {
        let config = CopyPasteConfig.saved
        if config.confirmPaste {
            let check = checkPaste(text: text, words: config.dangerousWords)
            if check.multiline || !check.dangerous.isEmpty {
                pendingPaste = text
                pendingPasteCR = appendCR
                sheet = .pasteConfirm
                return
            }
        }
        view.sendPaste(appendCR ? text + "\r" : text)
    }

    /// 確認ダイアログで OK を押したとき。
    func sendPaste(_ text: String) {
        view.sendPaste(pendingPasteCR ? text + "\r" : text)
        pendingPasteCR = false
    }

    func showSearch() {
        // 端末ビューがキー入力を持ったままだと検索欄に入らないので、先に手放す
        view.window?.makeFirstResponder(nil)
        search.visible = true
        search.focusToken += 1
    }

    func findNext(_ forward: Bool) {
        if !search.visible {
            showSearch()
            return
        }
        search.step(forward, in: view)
    }

    /// 「コントロール > 端末リセット」
    /// 「コントロール > リモートタイトルリセット」：相手が OSC で付けたタイトルを外す。
    func resetRemoteTitle() {
        remoteTitle = ""
    }

    /// 「常に最前面に表示」（TTXAlwaysOnTop）。
    var alwaysOnTop: Bool {
        get { view.window?.level == .floating }
        set {
            objectWillChange.send()
            view.window?.level = newValue ? .floating : .normal
        }
    }

    func resetTerminal() {
        view.textSession?.resetTerminal()
    }

    // MARK: SSH の問い合わせ

    /// コアからの問い合わせをシートで出す。別のシートが閉じかけていれば、閉じ終わるのを待つ。
    func present(_ prompt: AuthPrompt) {
        if sheet != nil {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
                guard let self else { prompt.cancel(); return }
                self.present(prompt)
            }
            return
        }
        authPrompt = prompt
        sheet = .auth
    }

    /// ダイアログの答え。
    func answer(_ answer: AuthPrompt.Answer) {
        let prompt = authPrompt
        authPrompt = nil
        sheet = nil
        prompt?.answer(answer)
    }

    /// シートが答えずに閉じられたとき（Esc など）。
    func authSheetDismissed() {
        if let prompt = authPrompt, !prompt.isAnswered {
            authPrompt = nil
            prompt.cancel()
        }
    }

    // MARK: ログ

    func startLog(_ options: LogOptions) {
        guard let session = view.session else { return }
        do {
            try session.logStart(options: options)
            isLogging = true
            isLogPaused = false
        } catch {
            report(error)
        }
    }

    func stopLog() {
        view.session?.logStop()
        isLogging = false
        isLogPaused = false
    }

    /// 「設定 > ログ」の自動開始。標準のフォルダとファイル名で始める。
    private func autoStartLog() {
        let config = LogConfig.saved
        guard config.autoStart else { return }
        let path = config.path(for: target)
        try? FileManager.default.createDirectory(
            at: URL(fileURLWithPath: path).deletingLastPathComponent(), withIntermediateDirectories: true)
        // 接続の直後に始める。それまでに届いた分は画面から書く
        startLog(LogOptionsView.savedOptions(path: path, includeScreen: true))
    }

    /// 「ログを中断」と再開。
    func toggleLogPause() {
        guard let session = view.session, isLogging else { return }
        isLogPaused.toggle()
        session.logPause(paused: isLogPaused)
    }

    func writeLogComment(_ text: String) {
        do {
            try view.session?.logWrite(text: text)
        } catch {
            report(error)
        }
    }

    /// 「ログを再生」で流すファイル（バイナリのログか ttyrec）。
    static func chooseReplayFile() -> String? {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = L("再生するログ（バイナリか ttyrec）を選んでください。")
        return panel.runModal() == .OK ? panel.url?.path : nil
    }

    /// 「ログを表示」：記録中のログを既定のアプリで開く。
    func viewLog() {
        guard let path = view.session?.logStatus()?.path else { return }
        let url = URL(fileURLWithPath: path)
        // 「設定 > ログ」のログ表示用エディタ（無ければ、その種類の既定のアプリ）
        let editor = LogConfig.saved.viewer
        if !editor.isEmpty {
            NSWorkspace.shared.open([url], withApplicationAt: URL(fileURLWithPath: editor),
                                    configuration: NSWorkspace.OpenConfiguration())
        } else {
            NSWorkspace.shared.open(url)
        }
    }

    // MARK: 閲覧専用・繰り返し送信

    func toggleViewOnly() {
        viewOnly.toggle()
        view.session?.setViewOnly(on: viewOnly)
    }

    func setRecurring(_ command: RecurringCommand?) {
        recurring = command
        if let c = command {
            view.session?.setRecurring(intervalSecs: UInt32(clamping: c.seconds), text: c.text, newline: c.newline)
            RecurringCommand.last = c
        } else {
            view.session?.setRecurring(intervalSecs: 0, text: "", newline: false)
        }
    }

    // MARK: 文字コード

    /// 今の接続の文字コード（受信と送信が違えば nil）。
    var charset: TerminalConfig.CharsetChoice? {
        let c = terminalConfig
        return c.charsetReceive == c.charsetSend ? c.charsetReceive : nil
    }

    /// 文字コードを受信・送信とも切り替える（TTXKanjiMenu）。
    func setCharset(_ choice: TerminalConfig.CharsetChoice) {
        var c = terminalConfig
        c.charsetReceive = choice
        c.charsetSend = choice
        applyTerminal(c)
    }

    // MARK: シリアルの信号線

    private func startModemPolling() {
        modemTimer?.invalidate()
        modemTimer = nil
        modemLines = nil
        guard isSerialConnected else { return }
        pollModem()
        modemTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.pollModem() }
        }
    }

    private func stopModemPolling() {
        modemTimer?.invalidate()
        modemTimer = nil
        modemLines = nil
    }

    /// ブレーク送信中は Transport が塞がるので、メインスレッドでは待たない。
    private func pollModem() {
        guard let session = view.session, isSerialConnected else { return stopModemPolling() }
        Task {
            let lines = await Task.detached { try? session.modemLines() }.value
            if self.isSerialConnected { self.modemLines = lines }
        }
    }

    /// DTR / RTS の切り替え（Tera Term の TTL setdtr / setrts に当たる）。
    func setLine(dtr: Bool? = nil, rts: Bool? = nil) {
        guard let session = view.session, let now = modemLines else { return }
        do {
            try session.setLines(dtr: dtr ?? now.dtr, rts: rts ?? now.rts)
            pollModem()
        } catch {
            report(error)
        }
    }

    // MARK: その他

    func focusTerminal() {
        DispatchQueue.main.async { [view] in
            view.window?.makeFirstResponder(view)
        }
    }

    private func handle(_ state: SessionState) {
        switch state {
        case .connecting:
            status = .connecting
        case .connected:
            status = .connected
        case .reconnecting:
            status = .reconnecting
        case .closed:
            status = .disconnected
            isLogging = false
            isLogPaused = false
            stopModemPolling()
            // シェルが終わったらウィンドウを閉じる（ターミナル.app と同じ）。
            // SSH / Telnet / シリアルは、エラーや切断の理由が読めるよう窓を残す
            if target?.isLocalShell == true {
                view.window?.performClose(nil)
            }
        }
    }

    func report(_ error: Error) {
        errorMessage = Self.describe(error, subject: target?.errorSubject)
    }

    /// `subject` は文に対象が無いときに入れる名前（ポートやホスト）。
    nonisolated static func describe(_ error: Error, subject: String? = nil) -> String {
        localized(rawMessage(error), subject: subject)
    }

    /// コアのエラーは MtError.Io(message:)。メッセージだけを取り出す
    nonisolated private static func rawMessage(_ error: Error) -> String {
        if let child = Mirror(reflecting: error).children.first {
            if let s = child.value as? String { return s }
            if let s = Mirror(reflecting: child.value).children.first?.value as? String { return s }
        }
        return error.localizedDescription
    }

    /// コアや OS の英語のエラーを、Tera Term の文言（表示言語のもの）か、このアプリの文言にする。
    /// 元の英語の文は詳しい情報として下に添える。見分けられないものはそのまま。
    nonisolated static func localized(_ message: String, subject: String? = nil) -> String {
        let info = classifyError(message: message)
        let what = info.subject ?? subject ?? ""
        func named(_ key: String, _ ja: String) -> String {
            TT.text(key, ja).replacingOccurrences(of: "%s", with: what)
        }
        let text: String
        switch info.class {
        case .refused: text = TT.text("MSG_COMM_REFUSE_ERROR", "接続が拒否されました")
        case .timedOut: text = TT.text("MSG_COMM_TIMEOUT_ERROR", "接続がタイムアウトしました")
        case .unreachable:
            text = TT.text("MSG_COMM_REACH_ERROR", "ネットワークに到達できません")
                // LAN 内のアドレスで出るときは、たいてい macOS のローカルネットワークの許可がない
                + (message.contains("No route to host")
                    ? "\n" + L("同じ LAN 内のホストなら、「システム設定 > プライバシーとセキュリティ > ローカルネットワーク」で Multi-Term を許可してください。")
                    : "")
        case .hostNotFound: text = TT.text("MSG_INVALID_HOST_ERROR", "無効なホスト")
        case .notFound:
            text = what.isEmpty ? TT.text("MSG_CANTOPEN_FILE_ERROR", "ファイルが開けません")
                : named("MSG_CANTOPEN_ERROR_NOTFOUND", "%sが見つかりません。")
        case .denied:
            text = what.isEmpty ? TT.text("MSG_CANTOPEN_FILE_ERROR", "ファイルが開けません")
                : named("MSG_CANTOPEN_ERROR_DENIED", "%sへのアクセスが拒否されました。")
        case .connectionLost: text = L("相手が接続を切りました。")
        case .authCanceled: text = L("認証を中止しました。")
        case .hostKeyRejected: text = L("サーバーのホスト鍵を受け入れなかったため、接続を中止しました。")
        case .hostEmpty: text = L("ホストを入力してください。")
        case .spaceParityUnsupported: text = L("この OS では、8 ビットと space パリティの組み合わせは使えません。")
        case .dsrDtrUnsupported: text = L("この OS では、DSR/DTR フロー制御は使えません。")
        case .transferRunning: text = L("ファイル転送の最中です。")
        case .other: return message
        }
        return text == message ? text : "\(text)\n（\(message)）"
    }
}

/// 繰り返し送信の設定（TTXRecurringCommand）。
struct RecurringCommand: Equatable {
    var seconds: Int
    var text: String
    var newline: Bool

    /// 前回の設定（ダイアログの初期値）
    static var last: RecurringCommand {
        get {
            let d = UserDefaults.standard
            return RecurringCommand(
                seconds: d.object(forKey: "recurring.seconds") == nil ? 60 : d.integer(forKey: "recurring.seconds"),
                text: d.string(forKey: "recurring.text") ?? "",
                newline: d.object(forKey: "recurring.newline") == nil ? true : d.bool(forKey: "recurring.newline"))
        }
        set {
            let d = UserDefaults.standard
            d.set(newValue.seconds, forKey: "recurring.seconds")
            d.set(newValue.text, forKey: "recurring.text")
            d.set(newValue.newline, forKey: "recurring.newline")
        }
    }
}

/// 「コントロール > 繰り返しコマンド」。
struct RecurringView: View {
    @ObservedObject var controller: TerminalController
    @Environment(\.dismiss) private var dismiss
    @State private var command = RecurringCommand.last

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(TT.text("MENU_CONTROL_RECURRING", "繰り返しコマンド", section: "TTXRecurringCommand")).font(.headline)
            Text(L("無通信で切られないよう、決まった間隔で文字列を送ります。"))
                .font(.callout).foregroundStyle(.secondary)
            HStack {
                Text(L("間隔（秒）:"))
                TextField("", value: $command.seconds, format: .number).frame(width: 70)
            }
            HStack {
                Text(L("送る文字列:"))
                TextField("", text: $command.text).frame(minWidth: 240)
            }
            Toggle(L("続けて改行を送る"), isOn: $command.newline)
            HStack {
                if controller.recurring != nil {
                    Button(L("止める")) {
                        controller.setRecurring(nil)
                        dismiss()
                    }
                }
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(L("始める")) {
                    command.seconds = max(1, command.seconds)
                    controller.setRecurring(command)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(command.text.isEmpty && !command.newline)
            }
        }
        .padding(20)
        .frame(width: 440)
        .onAppear { if let r = controller.recurring { command = r } }
    }
}
