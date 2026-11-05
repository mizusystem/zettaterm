import AppKit
import SwiftUI

/// macOS のネイティブタブ。端末ウィンドウは同じ tabbingIdentifier を持ち、
/// 「新しいタブ」「セッションの複製」で開いたウィンドウは、頼んだウィンドウのタブに入る。
@MainActor
enum WindowTabs {
    static let identifier = "MultiTerm.terminal"

    /// 次に開くウィンドウを入れる先（メニューを選んだときのウィンドウ）
    private static weak var pendingParent: NSWindow?

    static func prepareTab() {
        pendingParent = NSApp.keyWindow
    }

    /// ウィンドウができたときに呼ぶ。
    static func attach(_ window: NSWindow, asTab: Bool) {
        window.tabbingIdentifier = identifier
        window.tabbingMode = .automatic
        guard asTab, let parent = pendingParent, parent !== window else { return }
        pendingParent = nil
        parent.addTabbedWindow(window, ordered: .above)
        window.makeKeyAndOrderFront(nil)
    }
}

/// 自分が載っている NSWindow を 1 回だけ知らせる。
struct WindowAccessor: NSViewRepresentable {
    let onWindow: (NSWindow) -> Void

    func makeNSView(context: Context) -> NSView {
        let view = Probe()
        view.onWindow = onWindow
        return view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    private final class Probe: NSView {
        var onWindow: ((NSWindow) -> Void)?
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window, let f = onWindow else { return }
            onWindow = nil
            f(window)
        }
    }
}
