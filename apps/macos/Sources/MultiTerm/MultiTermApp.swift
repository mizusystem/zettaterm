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
            .navigationTitle(controller.windowTitle)
            .frame(minWidth: 200, minHeight: 100)
            .focusedSceneObject(controller)
            .onAppear { controller.open(request) }
            .sheet(item: $controller.sheet, onDismiss: { controller.focusTerminal() }) { sheet in
                switch sheet {
                case .newConnection: NewConnectionView(controller: controller)
                case .serialSettings: SerialSettingsView(controller: controller)
                case .log: LogOptionsView(controller: controller)
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

            Divider()

            Button(TT.text("MENU_FILE_LOG", "ログ...")) { controller?.sheet = .log }
                .disabled(controller?.status != .connected && controller?.status != .reconnecting)
            Button(TT.text("MENU_FILE_STOPLOG", "ログを終了")) { controller?.stopLog() }
                .disabled(controller?.isLogging != true)

            Divider()

            Button(TT.text("MENU_FILE_DISCONNECT", "接続断")) { controller?.disconnect() }
                .keyboardShortcut("i")
                .disabled(controller == nil || controller?.status == .disconnected)
        }

        CommandMenu(TT.text("MENU_SETUP", "設定")) {
            Button(TT.text("MENU_SETUP_SERIALPORT", "シリアルポート...")) { controller?.sheet = .serialSettings }
                .disabled(controller == nil)
        }

        CommandMenu(TT.text("MENU_CONTROL", "コントロール")) {
            Button(TT.text("MENU_CONTROL_SENDBREAK", "ブレーク送信")) { controller?.sendBreak() }
                .keyboardShortcut("b")
                .disabled(controller?.isSerialConnected != true)
        }
    }
}
