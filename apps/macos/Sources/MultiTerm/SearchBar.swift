import MTCore
import SwiftUI

/// 端末の検索の状態（ウィンドウごと）。一致はスクロールバックを含む全体から探す。
@MainActor
final class SearchState: ObservableObject {
    @Published var visible = false
    @Published var text = ""
    @Published var caseSensitive = false
    @Published var regex = false
    @Published private(set) var matches: [TermRange] = []
    @Published private(set) var current: Int?
    @Published private(set) var error: String?
    /// 検索欄にフォーカスを移す合図（開くたびに増やす）
    @Published var focusToken = 0

    /// 探し直す。`view` に一致を渡し、最後の一致（いちばん新しい出力）へ移る。
    func run(in view: TerminalView) {
        error = nil
        guard let session = view.textSession, !text.isEmpty else {
            matches = []
            current = nil
            view.setMatches([], current: nil)
            return
        }
        do {
            matches = try session.search(text: text, caseSensitive: caseSensitive, regex: regex)
        } catch {
            matches = []
            self.error = TerminalController.describe(error)
        }
        current = matches.isEmpty ? nil : matches.count - 1
        view.setMatches(matches, current: current)
    }

    /// 次（`forward`）または前の一致へ。
    func step(_ forward: Bool, in view: TerminalView) {
        if matches.isEmpty {
            run(in: view)
            return
        }
        let i = current ?? (forward ? -1 : matches.count)
        current = (i + (forward ? 1 : -1) + matches.count) % matches.count
        view.setMatches(matches, current: current)
    }

    func close(in view: TerminalView) {
        visible = false
        matches = []
        current = nil
        view.setMatches([], current: nil)
    }

    /// 出力が増えて行がスクロールバックへ押し出されたぶん、一致の行番号をずらす。
    func shift(by delta: Int32) {
        guard delta != 0, !matches.isEmpty else { return }
        matches = matches.map { $0.shifted(by: delta) }
    }

    var status: String {
        if let error { return error }
        guard !text.isEmpty else { return "" }
        guard let current else { return L("見つかりません") }
        return "\(current + 1) / \(matches.count)"
    }
}

extension TermRange {
    func shifted(by delta: Int32) -> TermRange {
        TermRange(
            start: TermPoint(line: start.line - delta, col: start.col),
            end: TermPoint(line: end.line - delta, col: end.col),
            block: block)
    }

    /// 読む順に並べた両端。
    var ordered: (TermPoint, TermPoint) {
        let s = (start.line, start.col), e = (end.line, end.col)
        return s <= e ? (start, end) : (end, start)
    }

    var isEmpty: Bool {
        (start.line == end.line && start.col == end.col) || (block && start.col == end.col)
    }

    /// セル `(line, col)` を含むか（mt_vt::Selection::contains と同じ）。
    func contains(line: Int32, col: UInt16) -> Bool {
        let (a, b) = ordered
        if line < a.line || line > b.line { return false }
        if block {
            let l = min(start.col, end.col), r = max(start.col, end.col)
            return col >= l && col < r
        }
        let afterStart = line > a.line || col >= a.col
        let beforeEnd = line < b.line || col < b.col
        return afterStart && beforeEnd
    }
}

/// 端末の右上に出す検索欄（Cmd+F）。
struct SearchBar: View {
    @ObservedObject var search: SearchState
    let view: TerminalView
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField(L("検索"), text: $search.text)
                .textFieldStyle(.plain)
                .frame(width: 180)
                .focused($focused)
                .onSubmit { search.step(!NSEvent.modifierFlags.contains(.shift), in: view) }
                .onChange(of: search.text) { _, _ in search.run(in: view) }
            Text(search.status)
                .font(.caption)
                .foregroundStyle(search.error == nil ? Color.secondary : Color.red)
                .lineLimit(1)
                .frame(minWidth: 60)
            Toggle("Aa", isOn: $search.caseSensitive)
                .toggleStyle(.button)
                .help(L("大文字と小文字を区別する"))
                .onChange(of: search.caseSensitive) { _, _ in search.run(in: view) }
            Toggle(".*", isOn: $search.regex)
                .toggleStyle(.button)
                .help(L("正規表現"))
                .onChange(of: search.regex) { _, _ in search.run(in: view) }
            Button { search.step(false, in: view) } label: { Image(systemName: "chevron.up") }
                .help(L("前を検索"))
            Button { search.step(true, in: view) } label: { Image(systemName: "chevron.down") }
                .help(L("次を検索"))
            Button { search.close(in: view); view.window?.makeFirstResponder(view) } label: {
                Image(systemName: "xmark")
            }
            .keyboardShortcut(.cancelAction)
            .help(L("閉じる"))
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
        .shadow(radius: 2)
        .padding(8)
        .onAppear { DispatchQueue.main.async { focused = true } }
        .onChange(of: search.focusToken) { _, _ in DispatchQueue.main.async { focused = true } }
    }
}
