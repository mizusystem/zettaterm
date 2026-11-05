import AppKit
import MTCore
import SwiftUI

/// 新しいウィンドウで何を開くか。`openWindow(value:)` で渡す。
struct ConnectRequest: Codable, Hashable {
    enum Target: Codable, Hashable {
        /// 「新しい接続」ダイアログを出す
        case dialog
        case localShell
        case serial(port: String, config: SerialConfig)
    }

    var id = UUID()
    var target: Target
}

/// 1 ウィンドウ分の接続とダイアログの状態。メニューからは FocusedObject として届く。
@MainActor
final class TerminalController: ObservableObject {
    enum Status { case disconnected, connected, reconnecting }

    enum Sheet: String, Identifiable {
        case newConnection, serialSettings, log
        var id: String { rawValue }
    }

    let view = TerminalView()

    @Published private(set) var status: Status = .disconnected
    @Published private(set) var isLogging = false
    @Published private(set) var remoteTitle = ""
    @Published private(set) var connectionLabel = ""
    @Published var sheet: Sheet?
    @Published var errorMessage: String?

    /// シリアルで接続中ならそのポート
    @Published private(set) var serialPort: String?
    @Published private(set) var serialConfig = SerialConfig.saved
    private var isLocalShell = false
    private var closingByUser = false

    init() {
        view.onState = { [weak self] state in self?.handle(state) }
        view.onTitle = { [weak self] title in self?.remoteTitle = title }
        view.onError = { [weak self] error in self?.report(error) }
    }

    var windowTitle: String {
        var base = remoteTitle.isEmpty ? connectionLabel : remoteTitle
        if base.isEmpty { base = "Multi-Term" }
        switch status {
        case .connected: return base
        case .reconnecting: return "\(base) [再接続待ち]"
        case .disconnected: return connectionLabel.isEmpty ? "Multi-Term" : "\(base) [未接続]"
        }
    }

    var isSerialConnected: Bool { serialPort != nil && status != .disconnected }

    // MARK: 接続

    func open(_ request: ConnectRequest?) {
        switch request?.target ?? .dialog {
        case .dialog: sheet = .newConnection
        case .localShell: connectLocalShell()
        case let .serial(port, config): connectSerial(port: port, config: config)
        }
    }

    func connectLocalShell() {
        connect(.localShell(program: nil, args: [], cwd: nil), label: "ローカルシェル") {
            self.isLocalShell = true
            self.serialPort = nil
        }
    }

    func connectSerial(port: String, config: SerialConfig) {
        config.save()
        SerialConfig.savedPort = port
        connect(.serial(port: port, settings: config.settings), label: "\(port) - \(config.summary)") {
            self.isLocalShell = false
            self.serialPort = port
            self.serialConfig = config
        }
    }

    private func connect(_ profile: Profile, label: String, onSuccess: () -> Void) {
        closingByUser = true
        view.disconnect()
        closingByUser = false
        isLogging = false
        remoteTitle = ""
        do {
            try view.connect(profile)
            connectionLabel = label
            status = .connected
            onSuccess()
        } catch {
            status = .disconnected
            report(error)
        }
        focusTerminal()
    }

    /// 「現在の接続を再設定」。つないだまま通信設定と送信遅延を変える。
    func reconfigureSerial(_ config: SerialConfig) {
        guard let session = view.session, let port = serialPort else { return }
        do {
            try session.setSerialSettings(settings: config.settings)
            serialConfig = config
            config.save()
            connectionLabel = "\(port) - \(config.summary)"
        } catch {
            report(error)
        }
    }

    func disconnect() {
        closingByUser = true
        view.disconnect()
        closingByUser = false
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
        case .connecting, .connected:
            status = .connected
        case .reconnecting:
            status = .reconnecting
        case .closed:
            status = .disconnected
            isLogging = false
            // シェルが終わったらウィンドウを閉じる（ターミナル.app と同じ）。シリアルは窓を残す
            if isLocalShell && !closingByUser {
                view.window?.performClose(nil)
            }
        }
    }

    private func report(_ error: Error) {
        errorMessage = Self.describe(error)
    }

    nonisolated static func describe(_ error: Error) -> String {
        // コアのエラーは MtError.Io(message:)。メッセージだけを取り出す
        if let child = Mirror(reflecting: error).children.first {
            if let s = child.value as? String { return s }
            if let s = Mirror(reflecting: child.value).children.first?.value as? String { return s }
        }
        return error.localizedDescription
    }
}
