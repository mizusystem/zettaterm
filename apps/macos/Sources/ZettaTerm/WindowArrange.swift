import AppKit
import ZettaCore
import SwiftUI

/// Tera Term の「ウィンドウ」メニュー：重ねて表示・左右に並べて表示・上下に並べて表示・すべて最小化・すべて復元。
/// タブにまとめたウィンドウは 1 つとして並べる。ウィンドウの一覧は macOS の「ウィンドウ」メニューにある。
@MainActor
enum WindowArrange {
    enum Layout { case cascade, sideBySide, stacked }

    /// 並べる前の位置（「元に戻す」で戻す）
    private static var saved: [(NSWindow, NSRect)] = []
    private(set) static var lastLayout: Layout?

    /// 端末のウィンドウ（タブのまとまりは、表に出ている 1 枚）。
    static var windows: [NSWindow] {
        var seen = Set<ObjectIdentifier>()
        var out: [NSWindow] = []
        for c in SessionRegistry.shared.controllers {
            guard let w = c.view.window else { continue }
            let front = w.tabGroup?.selectedWindow ?? w
            if seen.insert(ObjectIdentifier(front)).inserted { out.append(front) }
        }
        return out
    }

    static func arrange(_ layout: Layout) {
        let list = windows.filter { !$0.isMiniaturized }
        guard !list.isEmpty, let screen = (NSApp.keyWindow?.screen ?? NSScreen.main)?.visibleFrame else { return }
        saved = list.map { ($0, $0.frame) }
        lastLayout = layout
        let n = CGFloat(list.count)
        for (i, w) in list.enumerated() {
            let k = CGFloat(i)
            let frame: NSRect
            switch layout {
            case .cascade:
                let step: CGFloat = 28
                var f = w.frame
                // ずらした分だけはみ出さないよう、入りきらなければ縮める（Windows の重ねて表示も画面に収める）
                f.size.width = min(f.width, screen.width - step * (n - 1))
                f.size.height = min(f.height, screen.height - step * (n - 1))
                f.origin.x = screen.minX + step * k
                f.origin.y = screen.maxY - f.height - step * k
                frame = f
            case .sideBySide:
                let width = screen.width / n
                frame = NSRect(x: screen.minX + width * k, y: screen.minY, width: width, height: screen.height)
            case .stacked:
                let height = screen.height / n
                frame = NSRect(x: screen.minX, y: screen.maxY - height * (k + 1), width: screen.width, height: height)
            }
            w.setFrame(frame, display: true, animate: false)
        }
    }

    /// 並べる前に戻す。
    static func undo() {
        for (w, f) in saved { w.setFrame(f, display: true, animate: false) }
        saved = []
        lastLayout = nil
    }

    static func minimizeAll() {
        for w in windows where !w.isMiniaturized { w.miniaturize(nil) }
    }

    static func restoreAll() {
        for w in windows where w.isMiniaturized { w.deminiaturize(nil) }
    }

    /// 「ヘルプ > バージョン情報」。参照したソフトウェアの表示（Tera Term の BSD-3-Clause）も出す。
    static func showAbout() {
        let credits = NSAttributedString(
            string: thirdPartyNotices(),
            attributes: [.font: NSFont.monospacedSystemFont(ofSize: 10, weight: .regular),
                         .foregroundColor: NSColor.labelColor])
        NSApp.orderFrontStandardAboutPanel(options: [
            .applicationName: "ZettaTerm",
            .applicationVersion: versionLine(),
            .version: "",
            .credits: credits,
        ])
        NSApp.activate(ignoringOtherApps: true)
    }
}

/// メニューの「ウィンドウ」に足す項目。
struct WindowArrangeCommands: Commands {
    var body: some Commands {
        CommandGroup(replacing: .appInfo) {
            Button(TT.text("MENU_HELP_ABOUT", "ZettaTerm について...").replacingOccurrences(of: "Tera Term", with: "ZettaTerm")) {
                WindowArrange.showAbout()
            }
        }
        CommandGroup(after: .windowArrangement) {
            Divider()
            Button(TT.text("MENU_WINDOW_CASCADE", "重ねて表示")) { WindowArrange.arrange(.cascade) }
            Button(TT.text("MENU_WINDOW_SIDEBYSIDE", "左右に並べて表示")) { WindowArrange.arrange(.sideBySide) }
            Button(TT.text("MENU_WINDOW_STACKED", "上下に並べて表示")) { WindowArrange.arrange(.stacked) }
            Button(L("元に戻す - 並べて表示")) { WindowArrange.undo() }
            Divider()
            Button(TT.text("MENU_WINDOW_MINIMIZEALL", "すべて最小化")) { WindowArrange.minimizeAll() }
            Button(TT.text("MENU_WINDOW_RESTOREALL", "すべて復元")) { WindowArrange.restoreAll() }
        }
    }
}
