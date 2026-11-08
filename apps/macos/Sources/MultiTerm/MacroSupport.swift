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
            errorMessage = L("マクロを中止しました。\n%@", error)
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
            // この接続の最初の認証ダイアログの代わりに使う。ほかの接続には渡さず、接続が終わったら消す
            prompter.setOneTimePassword(password)
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

    @MainActor
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
        onMain { self.controller?.macro?.transferring = "\(send ? L("送信中") : L("受信中")): \(file)" }
    }

    func transferFinished(ok: Bool) {
        onMain { self.controller?.macro?.transferring = nil }
    }

    func listBox(title: String, text: String, items: [String], selected: UInt32) -> UInt32? {
        alert(nil) {
            let a = Self.make(title, text)
            let popup = NSPopUpButton(frame: NSRect(x: 0, y: 0, width: 280, height: 26), pullsDown: false)
            popup.addItems(withTitles: items)
            if Int(selected) < items.count { popup.selectItem(at: Int(selected)) }
            a.accessoryView = popup
            a.addButton(withTitle: TT.text("BTN_OK", "OK"))
            a.addButton(withTitle: TT.text("BTN_CANCEL", "キャンセル"))
            return (a, { $0 == .alertFirstButtonReturn && popup.indexOfSelectedItem >= 0
                ? UInt32(popup.indexOfSelectedItem) : nil })
        }
    }

    func choosePath(title: String, save: Bool, folder: Bool, startDir: String) -> String? {
        onMain {
            let panel: NSSavePanel
            if save {
                panel = NSSavePanel()
                panel.canCreateDirectories = true
            } else {
                let open = NSOpenPanel()
                open.canChooseFiles = !folder
                open.canChooseDirectories = folder
                open.canCreateDirectories = folder
                open.allowsMultipleSelection = false
                panel = open
            }
            panel.message = title
            panel.directoryURL = URL(fileURLWithPath: startDir)
            return panel.runModal() == .OK ? panel.url?.path : nil
        }
    }

    func clipboard() -> String? {
        onMain { NSPasteboard.general.string(forType: .string) }
    }

    func setClipboard(text: String) -> Bool {
        onMain {
            NSPasteboard.general.clearContents()
            return NSPasteboard.general.setString(text, forType: .string)
        }
    }

    /// `sendbroadcast` / `sendmulticast`。`group` があれば、その名前を付けたウィンドウにだけ送る。
    func broadcast(group: String?, text: String, newline: Bool) {
        onMain {
            for c in SessionRegistry.shared.controllers where c.isConnected {
                if let group, c.multicastName != group { continue }
                c.broadcast(text, enter: newline)
            }
        }
    }

    func setMulticastName(name: String) {
        onMain { self.controller?.multicastName = name.isEmpty ? nil : name }
    }

    /// `getttpos`。座標はポイントで、主ディスプレイの左上が原点（macOS の座標は左下が原点なので直す）。
    func windowPosition() -> MacroWindowPosition? {
        onMain { () -> MacroWindowPosition? in
            guard let view = self.controller?.view, let window = view.window else { return nil }
            let top = NSScreen.screens.first?.frame.maxY ?? 0
            func flip(_ r: NSRect) -> (Int32, Int32, Int32, Int32) {
                (Int32(r.minX.rounded()), Int32((top - r.maxY).rounded()),
                 Int32(r.width.rounded()), Int32(r.height.rounded()))
            }
            let w = flip(window.frame)
            let c = flip(window.convertToScreen(view.convert(view.bounds, to: nil)))
            let show: Int32 = window.isMiniaturized ? 1 : window.isZoomed ? 2 : window.isVisible ? 0 : 3
            return MacroWindowPosition(show: show, x: w.0, y: w.1, width: w.2, height: w.3,
                                       clientX: c.0, clientY: c.1, clientWidth: c.2, clientHeight: c.3)
        }
    }

    /// `restoresetup`。「設定 > 設定の読み込み」と同じく、このアプリの設定に移す。
    func restoreSetup(path: String) -> Bool {
        onMain {
            guard let ini = try? importTeratermIni(path: path) else { return false }
            SettingsImport.apply(ini)
            return true
        }
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
    /// 中断中（Tera Term のマクロウィンドウの「中断」「続行」）
    @State private var paused = false

    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
            HStack(spacing: 6) {
                if paused {
                    Image(systemName: "pause.circle").foregroundStyle(.secondary)
                } else {
                    ProgressView().controlSize(.small)
                }
                Text(L("マクロ: %@", macro.name)).font(.caption).lineLimit(1)
                // 今の行（0.5 秒ごとに取り直す）
                TimelineView(.periodic(from: .now, by: 0.5)) { _ in
                    Text(L("%@ 行目", String(macro.run.currentLine())))
                        .font(.caption).monospacedDigit().foregroundStyle(.secondary)
                }
                Button {
                    paused.toggle()
                    macro.run.setPaused(paused: paused)
                } label: {
                    Image(systemName: paused ? "play.fill" : "pause.fill")
                }
                .buttonStyle(.borderless)
                .help(paused ? TT.text("BTN_CONTINUE", "続行") : TT.text("BTN_PAUSE", "中断"))
                Button {
                    stop()
                } label: {
                    Image(systemName: "stop.fill")
                }
                .buttonStyle(.borderless)
                .help(L("マクロの停止"))
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
