import Foundation
import MTCore

/// ホスト履歴（コアの HostHistory、TOML）と ssh_config のホスト名。アプリ全体で 1 つ。
@MainActor
final class HostStore: ObservableObject {
    static let shared = HostStore()

    @Published private(set) var recent: [ConnectTarget] = []
    @Published private(set) var sshConfigHosts: [String] = []
    private var history: HostHistory?

    /// ~/Library/Application Support/Multi-Term/hosts.toml
    static var historyPath: String {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory())
        return base.appendingPathComponent("Multi-Term/hosts.toml").path
    }

    private init() {
        history = try? HostHistory.open(path: Self.historyPath)
        reload()
    }

    func reload() {
        recent = history?.entries().map(ConnectTarget.init) ?? []
        sshConfigHosts = MTCore.sshConfigHosts()
    }

    func add(_ target: ConnectTarget) {
        try? history?.add(profile: target.profile)
        recent = history?.entries().map(ConnectTarget.init) ?? recent
    }

    /// ホスト欄の候補。履歴のホスト（新しい順）のあとに ssh_config の Host 名。
    var hostSuggestions: [String] {
        var seen = Set<String>()
        var out: [String] = []
        for h in recent.compactMap(\.host) + sshConfigHosts where seen.insert(h).inserted {
            out.append(h)
        }
        return out
    }

    /// そのホストを最後に開いたときの接続先（サービスとポートを復元するため）。
    func last(forHost host: String) -> ConnectTarget? {
        recent.first { $0.host == host }
    }
}

/// 開いている端末ウィンドウ。ブロードキャストの送り先を選ぶのに使う。
@MainActor
final class SessionRegistry: ObservableObject {
    static let shared = SessionRegistry()

    private final class Weak {
        weak var value: TerminalController?
        init(_ v: TerminalController) { value = v }
    }

    private var items: [Weak] = []

    var controllers: [TerminalController] {
        items.removeAll { $0.value == nil }
        return items.compactMap(\.value)
    }

    func register(_ c: TerminalController) {
        items.removeAll { $0.value == nil || $0.value === c }
        items.append(Weak(c))
        objectWillChange.send()
    }
}
