import Foundation
import ZettaCore

/// シリアル設定。ウィンドウ間で受け渡しでき（Codable）、前回値として保存する。
struct SerialConfig: Codable, Hashable, Sendable {
    enum ParityChoice: String, Codable, CaseIterable, Sendable { case none, odd, even, mark, space }
    enum StopChoice: String, Codable, CaseIterable, Sendable { case one, two }
    /// 並びは Tera Term の FlowList（XON/XOFF, RTS/CTS, DSR/DTR, NONE）と同じ
    enum FlowChoice: String, Codable, CaseIterable, Sendable { case xonXoff, rtsCts, dsrDtr, none }

    var baud: UInt32 = 9600
    var dataBits: UInt8 = 8
    var parity: ParityChoice = .none
    var stopBits: StopChoice = .one
    var flow: FlowChoice = .none
    var delayPerCharMs: UInt32 = 0
    var delayPerLineMs: UInt32 = 0

    var settings: SerialSettings {
        SerialSettings(
            baud: baud,
            dataBits: dataBits,
            parity: {
                switch parity {
                case .none: return Parity.none
                case .odd: return .odd
                case .even: return .even
                case .mark: return .mark
                case .space: return .space
                }
            }(),
            stopBits: stopBits == .one ? .one : .two,
            flow: {
                switch flow {
                case .none: return FlowControl.none
                case .xonXoff: return .xonXoff
                case .rtsCts: return .rtsCts
                case .dsrDtr: return .dsrDtr
                }
            }(),
            delayPerCharMs: delayPerCharMs,
            delayPerLineMs: delayPerLineMs
        )
    }

    /// ブレーク信号の長さ（ミリ秒。Tera Term の SendBreakTime。既定 1000）
    static var breakMs: UInt32 {
        get {
            let v = UserDefaults.standard.integer(forKey: "serial.breakMs")
            return v > 0 ? UInt32(clamping: v) : 1000
        }
        set { UserDefaults.standard.set(Int(max(1, min(newValue, 10_000))), forKey: "serial.breakMs") }
    }

    /// タイトルなどに出す短い表記（例：115200bps 8N1）。
    var summary: String {
        let p: String
        switch parity {
        case .none: p = "N"
        case .odd: p = "O"
        case .even: p = "E"
        case .mark: p = "M"
        case .space: p = "S"
        }
        return "\(baud)bps \(dataBits)\(p)\(stopBits == .one ? 1 : 2)"
    }

    // MARK: 選択肢の表示名（Tera Term の serial_pp.cpp の並びと同じ順）

    static func label(_ c: ParityChoice) -> String {
        label("parity", ParityChoice.allCases.firstIndex(of: c)!, c.rawValue)
    }

    static func label(_ c: StopChoice) -> String {
        label("stop", StopChoice.allCases.firstIndex(of: c)!, c == .one ? "1 bit" : "2 bit")
    }

    static func label(_ c: FlowChoice) -> String {
        label("flow", FlowChoice.allCases.firstIndex(of: c)!, c.rawValue)
    }

    static func dataLabel(_ bits: UInt8) -> String {
        label("data", bits == 7 ? 0 : 1, "\(bits) bit")
    }

    static var baudPresets: [UInt32] {
        let list = TT.options("baud").compactMap(UInt32.init)
        return list.isEmpty ? [9600, 19200, 38400, 57600, 115200] : list
    }

    private static func label(_ list: String, _ index: Int, _ fallback: String) -> String {
        let items = TT.options(list)
        return index < items.count ? items[index] : fallback
    }

    // MARK: 前回値

    private static let key = "serialConfig"
    private static let portKey = "serialPort"

    static var saved: SerialConfig {
        guard let data = UserDefaults.standard.data(forKey: key),
              let config = try? JSONDecoder().decode(SerialConfig.self, from: data)
        else { return SerialConfig() }
        return config
    }

    func save() {
        if let data = try? JSONEncoder().encode(self) {
            UserDefaults.standard.set(data, forKey: Self.key)
        }
    }

    static var savedPort: String? {
        get { UserDefaults.standard.string(forKey: portKey) }
        set { UserDefaults.standard.set(newValue, forKey: portKey) }
    }
}

extension SerialDevice: Identifiable {
    public var id: String { path }

    /// 一覧に出す名前。製品名があれば製品名とパス、なければパスだけ。
    var displayName: String {
        if let product, !product.isEmpty { return "\(product) — \(path)" }
        return path
    }
}
