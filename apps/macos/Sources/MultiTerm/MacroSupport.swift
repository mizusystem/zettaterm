import AppKit
import MTCore
import SwiftUI
import UniformTypeIdentifiers

/// 実行中のマクロ（Tera Term の「コントロール > マクロ」）。
struct ActiveMacro {
    let name: String
    let run: MacroRun
    /// `statusbox` の内容
    var status: String?
    /// 転送中のファイル
    var transferring: String?
}

extension TerminalController {
    /// 「コントロール > マクロ...」。マクロファイルを選んで、このウィンドウの接続で実行する。
    func chooseMacro() {
        guard macro == nil else { return }
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        if let ttl = UTType(filenameExtension: "ttl") {
            panel.allowedContentTypes = [ttl, .plainText]
        }
        panel.message = TT.text("FILEDLG_OPEN_MACRO_TITLE", "マクロを開く")
        if let last = UserDefaults.standard.string(forKey: "macro.lastDir") {
            panel.directoryURL = URL(fileURLWithPath: last)
        }
        guard panel.runModal() == .OK, let url = panel.url else { return }
        UserDefaults.standard.set(url.deletingLastPathComponent().path, forKey: "macro.lastDir")
        runMacro(url)
    }

    func runMacro(_ url: URL) {
        guard macro == nil else { return }
        let bridge = MacroBridge()
        bridge.controller = self
        let run = MTCore.runMacro(path: url.path, params: [], session: isConnected ? view.session : nil, host: bridge)
        macro = ActiveMacro(name: url.lastPathComponent, run: run)
    }

    /// 「マクロの停止」。Tera Term と同じく確かめてから止める。
    func stopMacro() {
        guard let m = macro else { return }
        let alert = NSAlert()
        alert.messageText = TT.text("MSG_MACRO_CONF", "確認")
        alert.informativeText = TT.text("MSG_MACRO_HALT_SCRIPT", "本当にこのマクロスクリプトを停止しますか？")
        alert.addButton(withTitle: TT.text("BTN_YES", "はい"))
        alert.addButton(withTitle: TT.text("BTN_NO", "いいえ"))
        if alert.runModal() == .alertFirstButtonReturn {
            m.run.stop()
        }
    }

    func macroFinished(stopped: Bool, error: String?) {
        macro = nil
        if let error {
            errorMessage = "マクロを中止しました。\n\(error)"
        }
    }

    /// マクロの `connect`。接続して、つながるか失敗するまで待つ（呼び出しはマクロのスレッド）。
    func macroConnect(_ target: MacroConnect) -> ConnectTarget? {
        switch target {
        case let .serial(port, baud):
            var config = SerialConfig.saved
            if let baud { config.baud = baud }
            return .serial(port: port, config: config)
        case let .telnet(host, port):
            return .telnet(host: host, port: port, ip: .auto)
        case let .tcp(host, port):
            return .tcp(host: host, port: port, ip: .auto)
        case let .ssh(host, port, user, password):
            if let password {
                // 最初の認証ダイアログを出さずに使う（「パスワードをメモリ上に記憶する」と同じ仕組み）
                let key = PasswordMemory.key(user: user ?? NSUserName(), host: host, port: port ?? 22)
                PasswordMemory.set(key, password)
            }
            return .ssh(host: host, port: port, user: user)
        }
    }
}

/// マクロからの問い合わせを、このウィンドウのダイアログで尋ねる。マクロのスレッドから呼ばれる。
final class MacroBridge: MacroHost, @unchecked Sendable {
    @MainActor weak var controller: TerminalController?

    /// メインスレッドで `body` を実行して結果を返す。
    private func onMain<T: Sendable>(_ body: @escaping @MainActor @Sendable () -> T) -> T {
        DispatchQueue.main.sync { MainActor.assumeIsolated { body() } }
    }

    func connect(target: MacroConnect) -> Session? {
        let started: Bool = onMain {
            guard let c = self.controller, let t = c.macroConnect(target) else { return false }
            c.connect(t)
            return true
        }
        guard started else { return nil }
        // 認証ダイアログなどはメインスレッドで進む。こちらは結果を待つだけ
        let deadline = Date().addingTimeInterval(120)
        while Date() < deadline {
            Thread.sleep(forTimeInterval: 0.1)
            let state: (TerminalController.Status, Session?)? = onMain {
                guard let c = self.controller else { return nil }
                return (c.status, c.view.session)
            }
            guard let (status, session) = state else { return nil }
            switch status {
            case .connected, .reconnecting: return session
            case .disconnected: return nil
            case .connecting: continue
            }
        }
        return nil
    }

    func disconnect() {
        onMain { self.controller?.disconnect() }
    }

    private final class Box<T>: @unchecked Sendable {
        var value: T
        init(_ v: T) { value = v }
    }

    /// ウィンドウのシートとしてアラートを出し、閉じられるまで待つ（ウィンドウが無ければ単独で出す）。
    /// `make` はアラートと、閉じたときの答えの取り出し方を返す。
    private func alert<T: Sendable>(
        _ fallback: T,
        _ make: @escaping @MainActor @Sendable () -> (NSAlert, @MainActor (NSApplication.ModalResponse) -> T)
    ) -> T {
        let done = DispatchSemaphore(value: 0)
        let result = Box(fallback)
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                let (alert, answer) = make()
                if let window = self.controller?.view.window, window.attachedSheet == nil {
                    alert.beginSheetModal(for: window) { response in
                        MainActor.assumeIsolated { result.value = answer(response) }
                        done.signal()
                    }
                } else {
                    result.value = answer(alert.runModal())
                    done.signal()
                }
            }
        }
        done.wait()
        return result.value
    }

    private static func make(_ title: String, _ text: String) -> NSAlert {
        let a = NSAlert()
        a.messageText = title.isEmpty ? "MACRO" : title
        a.informativeText = text
        return a
    }

    func messageBox(title: String, text: String) {
        alert(()) { (Self.make(title, text), { _ in () }) }
    }

    func inputBox(title: String, text: String, defaultValue: String, password: Bool) -> String? {
        alert(nil) {
            let a = Self.make(title, text)
            let field: NSTextField = password ? NSSecureTextField() : NSTextField()
            field.frame = NSRect(x: 0, y: 0, width: 280, height: 24)
            field.stringValue = defaultValue
            a.accessoryView = field
            a.addButton(withTitle: TT.text("BTN_OK", "OK"))
            a.addButton(withTitle: TT.text("BTN_CANCEL", "キャンセル"))
            a.window.initialFirstResponder = field
            return (a, { $0 == .alertFirstButtonReturn ? field.stringValue : nil })
        }
    }

    func yesNoBox(title: String, text: String) -> Bool {
        alert(false) {
            let a = Self.make(title, text)
            a.addButton(withTitle: TT.text("BTN_YES", "はい"))
            a.addButton(withTitle: TT.text("BTN_NO", "いいえ"))
            return (a, { $0 == .alertFirstButtonReturn })
        }
    }

    func statusBox(title: String, text: String?) {
        onMain {
            guard let c = self.controller, c.macro != nil else { return }
            c.macro?.status = text.map { title.isEmpty ? $0 : "\(title): \($0)" }
        }
    }

    func setTitle(title: String) {
        onMain { self.controller?.macroTitle = title }
    }

    func beep() {
        onMain { NSSound.beep() }
    }

    func downloadDir() -> String {
        onMain { TerminalController.downloads.path }
    }

    func transferStarted(name: String, send: Bool) {
        let file = URL(fileURLWithPath: name).lastPathComponent
        onMain { self.controller?.macro?.transferring = "\(send ? "送信中" : "受信中"): \(file)" }
    }

    func transferFinished(ok: Bool) {
        onMain { self.controller?.macro?.transferring = nil }
    }

    func finished(stopped: Bool, error: String?) {
        DispatchQueue.main.async {
            MainActor.assumeIsolated { self.controller?.macroFinished(stopped: stopped, error: error) }
        }
    }
}

/// マクロ実行中の表示（ウィンドウの右上）。
struct MacroBadge: View {
    let macro: ActiveMacro
    let stop: () -> Void

    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("マクロ: \(macro.name)").font(.caption).lineLimit(1)
                Button {
                    stop()
                } label: {
                    Image(systemName: "stop.fill")
                }
                .buttonStyle(.borderless)
                .help("マクロの停止")
            }
            ForEach([macro.transferring, macro.status].compactMap { $0 }, id: \.self) { line in
                Text(line).font(.caption).lineLimit(3).frame(maxWidth: 260, alignment: .trailing)
            }
        }
        .padding(8)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .padding(8)
    }
}
