import AppKit
import SwiftUI

@main
struct MultiTermApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        WindowGroup {
            TerminalWindow()
        }
        .defaultSize(width: 740, height: 460)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // swift run で起動したとき（.app バンドルなし）も通常のアプリとして前面に出す
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

/// 1 ウィンドウ = 1 セッション。
struct TerminalWindow: View {
    @State private var title = "Multi-Term"

    var body: some View {
        TerminalContainer(title: $title)
            .navigationTitle(title)
            .frame(minWidth: 200, minHeight: 100)
    }
}
