import Foundation
import SwiftUI
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
        case eucKr, gbk, big5, koi8R, windows1251, ibm866, windows1252, iso88592, iso885915

        var label: String { charsetLabel(charset: core) }

        var core: Charset {
            switch self {
            case .utf8: return .utf8
            case .shiftJis: return .shiftJis
            case .eucJp: return .eucJp
            case .jis: return .jis
            case .eucKr: return .eucKr
            case .gbk: return .gbk
            case .big5: return .big5
            case .koi8R: return .koi8R
            case .windows1251: return .windows1251
            case .ibm866: return .ibm866
            case .windows1252: return .windows1252
            case .iso88592: return .iso88592
            case .iso885915: return .iso885915
            }
        }

        init(_ c: Charset) {
            self = Self.allCases.first { $0.core == c } ?? .utf8
        }
    }

    var receive: Receive = .cr
    var send: Send = .cr
    var localEcho = false
    var charsetReceive: CharsetChoice = .utf8
    var charsetSend: CharsetChoice = .utf8
    /// ENQ を受けたときに返す文字列（Tera Term の Answerback）
    var answerback = ""
    /// 端末 ID の名前（Tera Term の TerminalID。`VT100` など）
    var terminalIdName = "VT100"

    /// 選べる端末 ID
    static let terminalIds: [TerminalId] = [.vt100, .vt101, .vt102, .vt220, .vt282, .vt320, .vt382, .vt420, .vt520]

    var terminalId: TerminalId {
        get { Self.terminalIds.first { MTCore.terminalIdName(id: $0) == terminalIdName } ?? .vt100 }
        set { terminalIdName = MTCore.terminalIdName(id: newValue) }
    }

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
        answerback = try c.decodeIfPresent(String.self, forKey: .answerback) ?? ""
        terminalIdName = try c.decodeIfPresent(String.self, forKey: .terminalIdName) ?? "VT100"
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

/// 「設定 > キーボード」（Tera Term の BSKey / DelKey / MetaKey）。全ウィンドウで共通。
enum KeyboardConfig {
    private static var d: UserDefaults { .standard }

    static var saved: KeyboardSettings {
        get {
            KeyboardSettings(
                backspaceSendsBs: d.bool(forKey: "keyboard.bsSendsBs"),
                deleteSendsDel: d.bool(forKey: "keyboard.delSendsDel"),
                // 既定はオフ（Option は Option+¥ で \ を打つなど Mac の文字入力に使うため。ターミナル.app と同じ）
                metaSendsEscape: d.bool(forKey: "keyboard.metaEscape"))
        }
        set {
            d.set(newValue.backspaceSendsBs, forKey: "keyboard.bsSendsBs")
            d.set(newValue.deleteSendsDel, forKey: "keyboard.delSendsDel")
            d.set(newValue.metaSendsEscape, forKey: "keyboard.metaEscape")
        }
    }
}

/// 「設定 > キーボード」。
struct KeyboardSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var k = KeyboardConfig.saved

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(TT.text("DLG_KEYB_TITLE", "キーボード")).font(.headline)
            Text(TT.text("DLG_KEYB_TRANSMIT", "DELを送信するキー:")).foregroundStyle(.secondary)
            Toggle(L("BackSpace キーで DEL（0x7F）を送る（オフなら BS 0x08）"), isOn: Binding(
                get: { !k.backspaceSendsBs }, set: { k.backspaceSendsBs = !$0 }))
            Toggle(L("Delete キーで DEL（0x7F）を送る（オフなら ESC [3~）"), isOn: $k.deleteSendsDel)
            Toggle(L("Option キーをメタキーとして使う（文字の前に ESC を付ける）"), isOn: $k.metaSendsEscape)
            Text(L("Ctrl を押しながら BackSpace を押すと、もう一方を送ります。"))
                .font(.callout).foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    KeyboardConfig.saved = k
                    for c in SessionRegistry.shared.controllers { c.view.session?.setKeyboard(keyboard: k) }
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}
