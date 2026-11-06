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
        .commands { TerminalCommands() }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // swift run で起動したとき（.app バンドルなし）も通常のアプリとして前面に出す
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
        // デバイス監視を早めに始めて、最初のダイアログで一覧が出ているようにする
        Task { @MainActor in _ = DeviceStore.shared }
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
                }
                // スクロールバーにかからないように
                .padding(.trailing, NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy))
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

            Button(TT.text("MENU_FILE_SENDFILE", "ファイル送信...")) { controller?.beginSendFile() }
                .disabled(controller?.isConnected != true || controller?.transfer != nil)

            // TTSSH が「ファイル」メニューに足す項目
            Button("SSH SCP...") { controller?.sheet = .scp }
                .disabled(controller?.canScp != true || controller?.transfer != nil)

            Divider()

            Button(TT.text("MENU_FILE_DISCONNECT", "接続断")) { controller?.disconnect() }
                .keyboardShortcut("i")
                .disabled(controller == nil || controller?.status == .disconnected)
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
            Button(TT.text("MENU_SETUP_SERIALPORT", "シリアルポート...")) { controller?.sheet = .serialSettings }
                .disabled(controller == nil)
            Button(TT.text("DLG_TABSHEET_TITLE_COPYPASTE", "コピーと貼り付け") + "...") {
                controller?.sheet = .copyPasteSettings
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

            Divider()

            Button(TT.text("MENU_CONTROL_SENDBREAK", "ブレーク送信")) { controller?.sendBreak() }
                .keyboardShortcut("b")
                .disabled(controller?.isSerialConnected != true)

            Divider()

            Button(TT.text("MENU_CONTROL_MACRO", "マクロ") + "...") { controller?.chooseMacro() }
                .disabled(controller == nil || controller?.macro != nil)
            Button(L("マクロの停止")) { controller?.stopMacro() }
                .disabled(controller?.macro == nil)

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
