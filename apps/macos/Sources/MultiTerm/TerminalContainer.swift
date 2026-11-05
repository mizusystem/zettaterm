import AppKit
import MTCore
import SwiftUI

/// SwiftUI から AppKit の端末ビューを使うための橋渡し。
struct TerminalContainer: NSViewRepresentable {
    @Binding var title: String

    func makeNSView(context: Context) -> TerminalView {
        let view = TerminalView(profile: .localShell(program: nil, args: [], cwd: nil))
        view.onTitle = { newTitle in
            title = newTitle.isEmpty ? "Multi-Term" : newTitle
        }
        view.onClosed = { [weak view] in
            view?.window?.performClose(nil)
        }
        return view
    }

    func updateNSView(_ nsView: TerminalView, context: Context) {}

    static func dismantleNSView(_ nsView: TerminalView, coordinator: ()) {
        nsView.closeSession()
    }
}
