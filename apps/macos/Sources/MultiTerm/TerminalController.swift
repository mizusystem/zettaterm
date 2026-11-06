import AppKit
import MTCore
import SwiftUI

/// 1 ウィンドウ（タブ）分の接続とダイアログの状態。メニューからは FocusedObject として届く。
@MainActor
final class TerminalController: ObservableObject {
    enum Status { case disconnected, connecting, connected, reconnecting }

    enum Sheet: String, Identifiable {
        case newConnection, serialSettings, terminalSettings, log, broadcast, auth, transfer, scp, drop
        var id: String { rawValue }
    }

    let view = TerminalView()
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

    @Published private(set) var status: Status = .disconnected
    @Published private(set) var isLogging = false
    @Published private(set) var remoteTitle = ""
    @Published private(set) var target: ConnectTarget?
    @Published var sheet: Sheet?
    @Published var errorMessage: String?

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
        view.prompter = prompter
        prompter.controller = self
        SessionRegistry.shared.register(self)
    }

    var connectionLabel: String { target?.label ?? "" }

    var windowTitle: String {
        var base = macroTitle ?? (remoteTitle.isEmpty ? connectionLabel : remoteTitle)
        if base.isEmpty { base = "Multi-Term" }
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
    var isConnected: Bool { status == .connected || status == .reconnecting }

    // MARK: 接続

    func open(_ request: ConnectRequest?) {
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
        Task {
            do {
                try await view.connect(target.profile)
                prompter.setOneTimePassword(nil)
                guard current == attempt else { return }
                status = .connected
                if remember { HostStore.shared.add(target) }
            } catch is CancellationError {
                prompter.setOneTimePassword(nil)
                return
            } catch {
                prompter.setOneTimePassword(nil)
                guard current == attempt else { return }
                status = .disconnected
                report(error)
            }
            focusTerminal()
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
    }

    func disconnect() {
        authSheetDismissed()
        attempt += 1
        view.disconnect()
        status = .disconnected
        isLogging = false
    }

    func sendBreak() {
        guard let session = view.session else { return }
        // ブレーク中は待つので、メインスレッドを止めない
        Task {
            let message: String? = await Task.detached {
                do {
                    try session.sendBreak(ms: 300)
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
        } catch {
            report(error)
        }
    }

    func stopLog() {
        view.session?.logStop()
        isLogging = false
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
            // シェルが終わったらウィンドウを閉じる（ターミナル.app と同じ）。
            // SSH / Telnet / シリアルは、エラーや切断の理由が読めるよう窓を残す
            if target?.isLocalShell == true {
                view.window?.performClose(nil)
            }
        }
    }

    func report(_ error: Error) {
        errorMessage = Self.describe(error)
    }

    nonisolated static func describe(_ error: Error) -> String {
        localized(rawMessage(error))
    }

    /// コアのエラーは MtError.Io(message:)。メッセージだけを取り出す
    nonisolated private static func rawMessage(_ error: Error) -> String {
        if let child = Mirror(reflecting: error).children.first {
            if let s = child.value as? String { return s }
            if let s = Mirror(reflecting: child.value).children.first?.value as? String { return s }
        }
        return error.localizedDescription
    }

    /// コアの英語のメッセージのうち、よく出るものを日本語にする。
    nonisolated static func localized(_ message: String) -> String {
        let table: [(String, String)] = [
            ("authentication canceled", "認証を中止しました。"),
            ("Unknown server key", "サーバーのホスト鍵を受け入れなかったため、接続を中止しました。"),
            ("connection timed out", "接続がタイムアウトしました。"),
            ("Connection refused", "接続を拒否されました（ホストまたはポートを確かめてください）。"),
            ("host is empty", "ホストを入力してください。"),
            // LAN 内のアドレスで出るときは、たいてい macOS のローカルネットワークの許可がない
            ("No route to host", "ホストに到達できません。同じ LAN 内のホストなら、「システム設定 > プライバシーとセキュリティ > ローカルネットワーク」で Multi-Term を許可してください。"),
        ]
        for (en, ja) in table where message.contains(en) {
            return message == en ? L(ja) : "\(L(ja))\n（\(message)）"
        }
        return message
    }
}
