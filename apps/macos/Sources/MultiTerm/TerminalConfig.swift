import Foundation
import MTCore

/// 「設定 > 端末」のうち、接続に効く項目（改行コード、ローカルエコー）。全ウィンドウ共通で保存する。
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

    private static let key = "terminalConfig"

    static var saved: TerminalConfig {
        guard let data = UserDefaults.standard.data(forKey: key),
              let config = try? JSONDecoder().decode(TerminalConfig.self, from: data)
        else { return TerminalConfig() }
        return config
    }

    func save() {
        if let data = try? JSONEncoder().encode(self) {
            UserDefaults.standard.set(data, forKey: Self.key)
        }
    }
}
