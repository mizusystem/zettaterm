import AppKit
import MTCore
import SwiftUI

@main
struct MultiTermApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        // 起動直後のウィンドウは値なし（= 「新しい接続」ダイアログを出す）
        WindowGroup(for: ConnectRequest.self) { $request in
            TerminalWindow(request: request)
        }
        .defaultSize(width: 740, height: 460)
        .commands {
            TerminalCommands()
            WindowArrangeCommands()
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // swift run で起動したとき（.app バンドルなし）も通常のアプリとして前面に出す
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        // デバイス監視を早めに始めて、最初のダイアログで一覧が出ているようにする
        Task { @MainActor in _ = DeviceStore.shared }
        // 内蔵 SSH のキープアライブとポート転送
        SshSettings.apply()
        ProxyConfig.apply()
        setTelnetKeepalive(secs: UInt32(clamping: UserDefaults.standard.integer(forKey: "telnet.keepalive")))
        // ttermpro と同じ起動引数（open MultiTerm.app --args host /ssh /M=x.ttl など）
        SettingsImport.readLaunchArguments()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

/// 1 ウィンドウ = 1 接続。
struct TerminalWindow: View {
    let request: ConnectRequest?
    @StateObject private var controller = TerminalController()

    var body: some View {
        TerminalContainer(controller: controller)
            .overlay(alignment: .topTrailing) {
                VStack(alignment: .trailing, spacing: 0) {
                    SearchOverlay(search: controller.search, view: controller.view)
                    if let m = controller.macro {
                        MacroBadge(macro: m) { controller.stopMacro() }
                    }
                    if let notice = controller.notice {
                        Text(notice)
                            .font(.caption)
                            .padding(.horizontal, 10).padding(.vertical, 6)
                            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
                            .padding(8)
                            .transition(.opacity)
                    }
                }
                // スクロールバーにかからないように
                .padding(.trailing, NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy))
            }
            .overlay(alignment: .bottomTrailing) {
                if let lines = controller.modemLines {
                    SerialLinesBar(lines: lines, controller: controller)
                        .padding(.trailing, NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy) + 6)
                        .padding(.bottom, 6)
                }
            }
            .background(WindowAccessor { window in
                WindowTabs.attach(window, asTab: request?.asTab ?? false)
                if let sheet = controller.sheet {
                    DispatchQueue.main.async { ensureRoom(for: sheet) }
                }
            })
            .navigationTitle(controller.windowTitle)
            .frame(minWidth: 200, minHeight: 100)
            .focusedSceneObject(controller)
            .onAppear { controller.open(request) }
            .onChange(of: controller.sheet) { _, sheet in
                if let sheet { ensureRoom(for: sheet) }
            }
            .sheet(item: $controller.sheet, onDismiss: {
                controller.authSheetDismissed()
                controller.focusTerminal()
            }) { sheet in
                switch sheet {
                case .newConnection: NewConnectionView(controller: controller)
                case .serialSettings: SerialSettingsView(controller: controller)
                case .terminalSettings: TerminalSettingsView(controller: controller)
                case .log: LogOptionsView(controller: controller)
                case .broadcast: BroadcastView(controller: controller)
                case .auth: AuthPromptView(controller: controller)
                case .transfer: TransferView(controller: controller)
                case .scp: ScpView(controller: controller)
                case .drop: DropView(controller: controller)
                case .copyPasteSettings: CopyPasteSettingsView()
                case .pasteConfirm: PasteConfirmView(controller: controller)
                case .logSettings: LogSettingsView(controller: controller)
                case .logComment: LogCommentView(controller: controller)
                case .windowSettings: WindowSettingsView()
                case .fontSettings: FontSettingsView()
                case .sshSettings: SshSettingsView()
                case .sshKeyGen: SshKeyGenView()
                case .recurring: RecurringView(controller: controller)
                case .otherSettings: OtherSettingsView()
                case .proxySettings: ProxySettingsView()
                case .keyboardSettings: KeyboardSettingsView()
                }
            }
            .alert(
                "Multi-Term",
                isPresented: Binding(
                    get: { controller.errorMessage != nil },
                    set: { if !$0 { controller.errorMessage = nil } }
                ),
                actions: { Button(TT.text("BTN_OK", "OK")) {} },
                message: { Text(controller.errorMessage ?? "") }
            )
    }
}

extension TerminalWindow {
    /// シートがウィンドウからはみ出さないよう、小さいウィンドウは広げる。
    fileprivate func ensureRoom(for sheet: TerminalController.Sheet) {
        guard let window = controller.view.window, !window.styleMask.contains(.fullScreen) else { return }
        let need: CGSize
        switch sheet {
        case .newConnection: need = CGSize(width: 620, height: 600)
        case .serialSettings: need = CGSize(width: 580, height: 520)
        case .auth: need = CGSize(width: 600, height: 520)
        default: need = CGSize(width: 580, height: 420)
        }
        var frame = window.frame
        let w = max(frame.width, need.width), h = max(frame.height, need.height)
        guard w != frame.width || h != frame.height else { return }
        // 上端をそろえたまま下へ広げる
        frame.origin.y -= h - frame.height
        frame.size = CGSize(width: w, height: h)
        if let screen = window.screen?.visibleFrame { frame = frame.intersection(screen) }
        window.setFrame(frame, display: true, animate: true)
    }
}

private func sendKey(_ k: TransferKind) -> String {
    switch k {
    case .xmodem: return "MENU_TRANS_X_SEND"
    case .ymodem: return "MENU_TRANS_Y_SEND"
    case .zmodem: return "MENU_TRANS_Z_SEND"
    case .kermit: return "MENU_TRANS_KERMIT_SEND"
    }
}

private func receiveKey(_ k: TransferKind) -> String {
    switch k {
    case .xmodem: return "MENU_TRANS_X_RCV"
    case .ymodem: return "MENU_TRANS_Y_RCV"
    case .zmodem: return "MENU_TRANS_Z_RCV"
    case .kermit: return "MENU_TRANS_KERMIT_RCV"
    }
}

/// メニュー。項目名は Tera Term の文言、ショートカットは Alt を Cmd に読み替えたもの。
struct TerminalCommands: Commands {
    @FocusedObject private var controller: TerminalController?
    @AppStorage("zmodem.autoReceive") private var autoZmodem = true
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button(TT.text("MENU_FILE_NEW", "新しい接続...")) {
                if let controller, controller.status == .disconnected {
                    controller.sheet = .newConnection
                } else {
                    openWindow(value: ConnectRequest(target: .dialog))
                }
            }
            .keyboardShortcut("n")

            Button(L("新しいタブ")) {
                WindowTabs.prepareTab()
                openWindow(value: ConnectRequest(target: .dialog, asTab: true))
            }
            .keyboardShortcut("t")

            Button(TT.text("MENU_FILE_DUPLICATE", "セッションの複製")) {
                if let request = controller?.duplicateRequest {
                    WindowTabs.prepareTab()
                    openWindow(value: request)
                }
            }
            .keyboardShortcut("d")
            .disabled(controller?.target == nil)

            Divider()

            Button(TT.text("MENU_FILE_LOG", "ログ...")) { controller?.sheet = .log }
                .disabled(controller?.isConnected != true)
            Button(TT.text("MENU_FILE_STOPLOG", "ログを終了")) { controller?.stopLog() }
                .disabled(controller?.isLogging != true)
            Toggle(TT.text("MENU_FILE_PAUSELOG", "ログを中断"), isOn: Binding(
                get: { controller?.isLogPaused == true },
                set: { _ in controller?.toggleLogPause() }
            ))
            .disabled(controller?.isLogging != true)
            Button(TT.text("MENU_FILE_COMMENTLOG", "ログにコメントを付加...")) { controller?.sheet = .logComment }
                .disabled(controller?.isLogging != true)
            Button(TT.text("MENU_FILE_VIEWLOG", "ログを表示")) { controller?.viewLog() }
                .disabled(controller?.isLogging != true)
            Button(TT.text("MENU_FILE_REPLAYLOG", "ログを再生...")) {
                if let path = TerminalController.chooseReplayFile() {
                    WindowTabs.prepareTab()
                    openWindow(value: ConnectRequest(target: .connect(.replay(path: path)), asTab: true))
                }
            }

            Button(TT.text("MENU_FILE_SENDFILE", "ファイル送信...")) { controller?.beginSendFile() }
                .disabled(controller?.isConnected != true || controller?.transfer != nil)
            Button(TT.text("MENU_FILE_RECVFILE", "ファイル受信...")) { controller?.beginRawReceive() }
                .disabled(controller?.isConnected != true || controller?.transfer != nil)

            // TTSSH が「ファイル」メニューに足す項目
            Button("SSH SCP...") { controller?.sheet = .scp }
                .disabled(controller?.canScp != true || controller?.transfer != nil)

            Divider()

            Button(TT.text("MENU_FILE_DISCONNECT", "接続断")) { controller?.requestDisconnect() }
                .keyboardShortcut("i")
                .disabled(controller == nil || controller?.status == .disconnected)
            // すべてのウィンドウを閉じて終わる（「Multi-Term を終了」と同じ）
            Button(TT.text("MENU_FILE_EXITALL", "全終了").replacingOccurrences(of: "Tera Term", with: "Multi-Term")) {
                NSApp.terminate(nil)
            }
        }

        // 「編集」。コピー・貼り付け・全て選択は応答チェーンに送るので、ダイアログの入力欄でも効く
        CommandGroup(replacing: .undoRedo) {}
        CommandGroup(replacing: .pasteboard) {
            Button(TT.text("MENU_EDIT_COPY", "コピー")) { NSApp.sendAction(#selector(TerminalView.copy(_:)), to: nil, from: nil) }
                .keyboardShortcut("c")
            Button(TT.text("MENU_EDIT_COPYTABLE", "テーブルをコピー")) { controller?.view.copyTable(nil) }
                .disabled(controller == nil)
            Button(TT.text("MENU_EDIT_PASTE", "貼り付け")) { NSApp.sendAction(#selector(TerminalView.paste(_:)), to: nil, from: nil) }
                .keyboardShortcut("v")
            Button(TT.text("MENU_EDIT_PASTECR", "貼り付け<CR>")) { controller?.view.pasteClipboard(appendCR: true) }
                .keyboardShortcut("r")
                .disabled(controller?.isConnected != true)
            Divider()
            Button(TT.text("MENU_EDIT_CLSCREEN", "画面クリア")) { controller?.view.clearScreen(nil) }
                .disabled(controller?.view.textSession == nil)
            Button(TT.text("MENU_EDIT_CLBUFFER", "バッファのクリア")) { controller?.view.clearBuffer(nil) }
                .disabled(controller?.view.textSession == nil)
            Divider()
            Button(TT.text("MENU_EDIT_CANCELSELECT", "選択を解除")) { controller?.view.cancelSelection(nil) }
                .disabled(controller == nil)
            Button(TT.text("MENU_EDIT_SELECTSCREEN", "表示画面を選択")) { controller?.view.selectScreen(nil) }
                .disabled(controller == nil)
            Button(TT.text("MENU_EDIT_SELECTALL", "全て選択")) { NSApp.sendAction(#selector(NSResponder.selectAll(_:)), to: nil, from: nil) }
                .keyboardShortcut("a")
        }
        CommandGroup(replacing: .textEditing) {
            Button(L("検索...")) { controller?.showSearch() }
                .keyboardShortcut("f")
                .disabled(controller == nil)
            Button(L("次を検索")) { controller?.findNext(true) }
                .keyboardShortcut("g")
                .disabled(controller == nil)
            Button(L("前を検索")) { controller?.findNext(false) }
                .keyboardShortcut("g", modifiers: [.command, .shift])
                .disabled(controller == nil)
        }

        CommandMenu(TT.text("MENU_SETUP", "設定")) {
            Button(TT.text("MENU_SETUP_TERMINAL", "端末...")) { controller?.sheet = .terminalSettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SETUP_WINDOW", "ウィンドウ...")) { controller?.sheet = .windowSettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SETUP_FONT", "フォント...")) { controller?.sheet = .fontSettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SETUP_KEYBOARD", "キーボード...")) { controller?.sheet = .keyboardSettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SETUP_SERIALPORT", "シリアルポート...")) { controller?.sheet = .serialSettings }
                .disabled(controller == nil)
            Button(TT.text("DLG_TABSHEET_TITLE_COPYPASTE", "コピーと貼り付け") + "...") {
                controller?.sheet = .copyPasteSettings
            }
            .disabled(controller == nil)
            Button(TT.text("DLG_FOPT_TITLE", "ログ") + "...") { controller?.sheet = .logSettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SETUP_ADDITION", "その他の設定...")) { controller?.sheet = .otherSettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SETUP", "プロキシ...", section: "TTProxy")) { controller?.sheet = .proxySettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SETUP_RESTORE", "設定の読み込み...")) {
                SettingsImport.chooseAndImport(controller)
            }
            Divider()
            // TTSSH の「SSH...」と「SSH 転送...」をまとめたもの
            Button(TT.text("MENU_SSH", "SSH...", section: "TTSSH")) { controller?.sheet = .sshSettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SSH_KEYGEN", "SSH鍵生成...", section: "TTSSH")) { controller?.sheet = .sshKeyGen }
                .disabled(controller == nil)

            Divider()

            // 文字コードのクイック切り替え（Tera Term の TTXKanjiMenu）。受信と送信をまとめて変える
            Menu(L("エンコーディング")) {
                ForEach(TerminalConfig.CharsetChoice.allCases, id: \.self) { choice in
                    Toggle(choice.label, isOn: Binding(
                        get: { controller?.charset == choice },
                        set: { _ in controller?.setCharset(choice) }
                    ))
                }
            }
            .disabled(controller == nil)
        }

        CommandMenu(TT.text("MENU_TRANS", "転送")) {
            ForEach(TransferKind.allCases) { kind in
                Menu(kind.rawValue) {
                    Button(TT.text(sendKey(kind), "送信...")) { controller?.beginSend(kind) }
                    // Tera Term の ZMODEM / Kermit の受信は保存先を尋ねないが、ここでは保存先フォルダを選ぶ
                    Button(TT.text(receiveKey(kind), "受信...") + (kind == .zmodem || kind == .kermit ? "..." : "")) {
                        controller?.beginReceive(kind)
                    }
                    if kind == .kermit {
                        // 相手がサーバー（C-Kermit の server など）のとき
                        Button(TT.text("MENU_TRANS_KERMIT_GET", "取得...")) { controller?.beginKermitGet() }
                        Button(TT.text("MENU_TRANS_KERMIT_FINISH", "終了")) { controller?.kermitFinish() }
                    }
                }
                .disabled(controller?.isConnected != true || controller?.transfer != nil)
            }
            Divider()
            // 相手の sz で自動的に受け取るか（切ると、受け取る前に確かめる）
            Toggle(L("ZMODEM の自動受信"), isOn: $autoZmodem)
        }

        CommandMenu(TT.text("MENU_CONTROL", "コントロール")) {
            Button(TT.text("MENU_CONTROL_RESET", "端末リセット")) { controller?.resetTerminal() }
                .disabled(controller?.view.textSession == nil)
            Button(TT.text("MENU_CONTROL_RESETTITLE", "リモートタイトルリセット")) { controller?.resetRemoteTitle() }
                .disabled(controller == nil)
            Button(TT.text("MENU_CONTROL_RESETPORT", "ポートのリセット")) { controller?.resetPort() }
                .disabled(controller?.isSerialConnected != true)
            Button(TT.text("MENU_CONTROL_AREYOUTHERE", "AYT 送信")) { try? controller?.view.session?.areYouThere() }
                .disabled(controller?.isTelnet != true)
            Toggle(TT.text("MENU_ALWAYSONTOP", "常に最前面に表示", section: "TTXAlwaysOnTop"), isOn: Binding(
                get: { controller?.alwaysOnTop == true },
                set: { controller?.alwaysOnTop = $0 }
            ))
            .disabled(controller == nil)

            Divider()

            Button(TT.text("MENU_CONTROL_SENDBREAK", "ブレーク送信")) { controller?.sendBreak() }
                .keyboardShortcut("b")
                .disabled(controller?.isSerialConnected != true)
            // 出す信号線。ESP32 などは DTR / RTS でリセットや書き込みモードに入るので、押したときだけ変える
            Toggle("DTR", isOn: Binding(
                get: { controller?.modemLines?.dtr == true },
                set: { controller?.setLine(dtr: $0) }
            ))
            .disabled(controller?.modemLines == nil)
            Toggle("RTS", isOn: Binding(
                get: { controller?.modemLines?.rts == true },
                set: { controller?.setLine(rts: $0) }
            ))
            .disabled(controller?.modemLines == nil)

            Divider()

            Button(TT.text("MENU_CONTROL_MACRO", "マクロ") + "...") { controller?.chooseMacro() }
                .disabled(controller == nil || controller?.macro != nil)
            Button(L("マクロの停止")) { controller?.stopMacro() }
                .disabled(controller?.macro == nil)

            Divider()

            Toggle(TT.text("MENU_VIEWMODE", "View modeに入る", section: "TTXViewMode"), isOn: Binding(
                get: { controller?.viewOnly == true },
                set: { _ in controller?.toggleViewOnly() }
            ))
            .disabled(controller == nil)
            Button(TT.text("MENU_CONTROL_RECURRING", "繰り返しコマンド", section: "TTXRecurringCommand") + "...") {
                controller?.sheet = .recurring
            }
            .disabled(controller?.isConnected != true)

            Divider()

            Button(TT.text("MENU_CONTROL_BROADCAST", "ブロードキャストコマンド") + "...") {
                controller?.sheet = .broadcast
            }
            .keyboardShortcut("b", modifiers: [.command, .shift])
            .disabled(controller == nil)
        }
    }
}

/// 検索欄（開いているときだけ出す）。
private struct SearchOverlay: View {
    @ObservedObject var search: SearchState
    let view: TerminalView

    var body: some View {
        if search.visible {
            SearchBar(search: search, view: view)
        }
    }
}

/// シリアルの信号線の表示（右下）。CTS・DSR・CD・RI は相手の状態、DTR・RTS は押すと切り替わる。
private struct SerialLinesBar: View {
    let lines: ModemLines
    @ObservedObject var controller: TerminalController

    var body: some View {
        HStack(spacing: 10) {
            lamp("CTS", lines.cts)
            lamp("DSR", lines.dsr)
            lamp("CD", lines.cd)
            lamp("RI", lines.ri)
            Divider().frame(height: 12)
            Button { controller.setLine(dtr: !lines.dtr) } label: { lamp("DTR", lines.dtr) }
                .help(L("DTR を切り替える"))
            Button { controller.setLine(rts: !lines.rts) } label: { lamp("RTS", lines.rts) }
                .help(L("RTS を切り替える"))
        }
        .buttonStyle(.plain)
        .font(.system(size: 10, design: .monospaced))
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(.regularMaterial, in: Capsule())
        .opacity(0.9)
    }

    private func lamp(_ name: String, _ on: Bool) -> some View {
        HStack(spacing: 3) {
            Circle().fill(on ? Color.green : Color.gray.opacity(0.5)).frame(width: 7, height: 7)
            Text(name)
        }
    }
}
