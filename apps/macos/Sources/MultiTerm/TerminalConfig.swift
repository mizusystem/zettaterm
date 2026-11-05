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

    /// 相手との間の文字コード（Tera Term の「エンコーディング-受信 / 送信」）
    enum CharsetChoice: String, Codable, CaseIterable, Sendable {
        case utf8, shiftJis, eucJp, jis

        var label: String {
            switch self {
            case .utf8: return "UTF-8"
            case .shiftJis: return "Shift_JIS"
            case .eucJp: return "EUC-JP"
            case .jis: return "JIS"
            }
        }

        var core: Charset {
            switch self {
            case .utf8: return .utf8
            case .shiftJis: return .shiftJis
            case .eucJp: return .eucJp
            case .jis: return .jis
            }
        }
    }

    var receive: Receive = .cr
    var send: Send = .cr
    var localEcho = false
    var charsetReceive: CharsetChoice = .utf8
    var charsetSend: CharsetChoice = .utf8

    init(receive: Receive = .cr, send: Send = .cr, localEcho: Bool = false,
         charsetReceive: CharsetChoice = .utf8, charsetSend: CharsetChoice = .utf8) {
        self.receive = receive
        self.send = send
        self.localEcho = localEcho
        self.charsetReceive = charsetReceive
        self.charsetSend = charsetSend
    }

    /// 前の版で保存した設定（文字コードの項目が無い）も読めるようにする
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        receive = try c.decodeIfPresent(Receive.self, forKey: .receive) ?? .cr
        send = try c.decodeIfPresent(Send.self, forKey: .send) ?? .cr
        localEcho = try c.decodeIfPresent(Bool.self, forKey: .localEcho) ?? false
        charsetReceive = try c.decodeIfPresent(CharsetChoice.self, forKey: .charsetReceive) ?? .utf8
        charsetSend = try c.decodeIfPresent(CharsetChoice.self, forKey: .charsetSend) ?? .utf8
    }

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
            localEcho: localEcho,
            charsetReceive: charsetReceive.core,
            charsetSend: charsetSend.core
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
