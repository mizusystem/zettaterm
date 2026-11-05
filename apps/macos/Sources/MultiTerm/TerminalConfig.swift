import Foundation
import MTCore

/// 「設定 > 端末」のうち、接続に効く項目（改行コード、ローカルエコー）。
/// シリアル機器は LF だけで改行するものが多い一方、シェルや SSH で受信 AUTO にすると
/// 行頭に戻る CR まで改行になってしまうので、シリアルとそれ以外で別々に保存する。
struct TerminalConfig: Codable, Hashable, Sendable {
    /// 並びは Tera Term の NLListRcv（CR, CR+LF, LF, AUTO）と同じ
    enum Receive: String, Codable, CaseIterable, Sendable { case cr, crLf, lf, auto }
    /// 並びは Tera Term の NLList（CR, CR+LF, LF）と同じ
    enum Send: String, Codable, CaseIterable, Sendable { case cr, crLf, lf }

    var receive: Receive = .cr
    var send: Send = .cr
    var localEcho = false

    var settings: TerminalSettings {
        TerminalSettings(
            newlineReceive: {
                switch receive {
                case .cr: return NewlineReceive.cr
                case .crLf: return .crLf
                case .lf: return .lf
                case .auto: return .auto
                }
            }(),
            newlineSend: {
                switch send {
                case .cr: return NewlineSend.cr
                case .crLf: return .crLf
                case .lf: return .lf
                }
            }(),
            localEcho: localEcho
        )
    }

    static func label(_ r: Receive) -> String {
        let items = TT.options("nlRcv")
        let i = Receive.allCases.firstIndex(of: r)!
        return i < items.count ? items[i] : r.rawValue.uppercased()
    }

    static func label(_ s: Send) -> String {
        let items = TT.options("nl")
        let i = Send.allCases.firstIndex(of: s)!
        return i < items.count ? items[i] : s.rawValue.uppercased()
    }

    enum Scope: String { case serial, other }

    static func scope(for target: ConnectTarget?) -> Scope {
        target?.serialPort != nil ? .serial : .other
    }

    private static func key(_ scope: Scope) -> String { "terminalConfig.\(scope.rawValue)" }

    static func saved(_ scope: Scope) -> TerminalConfig {
        let defaults = UserDefaults.standard
        // 以前は 1 つだけ保存していた。シリアル向けに設定されたものとして引き継ぐ
        if scope == .serial, defaults.data(forKey: key(.serial)) == nil,
           let old = defaults.data(forKey: "terminalConfig") {
            defaults.set(old, forKey: key(.serial))
            defaults.removeObject(forKey: "terminalConfig")
        }
        guard let data = defaults.data(forKey: key(scope)),
              let config = try? JSONDecoder().decode(TerminalConfig.self, from: data)
        else { return TerminalConfig() }
        return config
    }

    func save(_ scope: Scope) {
        if let data = try? JSONEncoder().encode(self) {
            UserDefaults.standard.set(data, forKey: Self.key(scope))
        }
    }
}
