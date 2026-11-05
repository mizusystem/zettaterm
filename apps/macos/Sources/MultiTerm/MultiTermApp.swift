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

            Button("新しいタブ") {
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

            // TTSSH が「ファイル」メニューに足す項目
            Button("SSH SCP...") { controller?.sheet = .scp }
                .disabled(controller?.canScp != true || controller?.transfer != nil)

            Divider()

            Button(TT.text("MENU_FILE_DISCONNECT", "接続断")) { controller?.disconnect() }
                .keyboardShortcut("i")
                .disabled(controller == nil || controller?.status == .disconnected)
        }

        CommandMenu(TT.text("MENU_SETUP", "設定")) {
            Button(TT.text("MENU_SETUP_TERMINAL", "端末...")) { controller?.sheet = .terminalSettings }
                .disabled(controller == nil)
            Button(TT.text("MENU_SETUP_SERIALPORT", "シリアルポート...")) { controller?.sheet = .serialSettings }
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
        }

        CommandMenu(TT.text("MENU_CONTROL", "コントロール")) {
            Button(TT.text("MENU_CONTROL_SENDBREAK", "ブレーク送信")) { controller?.sendBreak() }
                .keyboardShortcut("b")
                .disabled(controller?.isSerialConnected != true)

            Divider()

            Button(TT.text("MENU_CONTROL_BROADCAST", "ブロードキャストコマンド") + "...") {
                controller?.sheet = .broadcast
            }
            .keyboardShortcut("b", modifiers: [.command, .shift])
            .disabled(controller == nil)
        }
    }
}
