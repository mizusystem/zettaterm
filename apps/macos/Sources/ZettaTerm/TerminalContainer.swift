import AppKit
import SwiftUI

/// SwiftUI から AppKit の端末ビューを使うための橋渡し。ビューは TerminalController が持つ。
struct TerminalContainer: NSViewRepresentable {
    let controller: TerminalController

    func makeNSView(context: Context) -> TerminalView {
        controller.view
    }

    func updateNSView(_ nsView: TerminalView, context: Context) {}

    static func dismantleNSView(_ nsView: TerminalView, coordinator: ()) {
        nsView.closeSession()
    }
}
