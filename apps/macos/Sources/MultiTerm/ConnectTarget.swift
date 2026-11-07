import Foundation
import MTCore

/// 接続先。ウィンドウ間で受け渡せるよう Codable にしてある（コアの Profile は Codable ではないため）。
enum ConnectTarget: Codable, Hashable, Sendable {
    enum IP: String, Codable, CaseIterable, Sendable { case auto, v4, v6 }

    case localShell
    case serial(port: String, config: SerialConfig)
    /// `systemSSH` なら内蔵の SSH ではなくシステムの ssh を使う（認証は端末の中で行う）
    case ssh(host: String, port: UInt16?, user: String?, systemSSH: Bool = false)
    case telnet(host: String, port: UInt16?, ip: IP)
    case tcp(host: String, port: UInt16, ip: IP)
    /// ログの再生（「ファイル > ログを再生」）。履歴には残さない
    case replay(path: String)

    var profile: Profile {
        switch self {
        case .localShell:
            return .localShell(program: nil, args: [], cwd: nil)
        case let .serial(port, config):
            return .serial(port: port, settings: config.settings)
        case let .ssh(host, port, user, system):
            return .ssh(host: host, port: port, user: user, systemSsh: system)
        case let .telnet(host, port, ip):
            return .telnet(host: host, port: port, ip: ip.core)
        case let .tcp(host, port, ip):
            return .tcp(host: host, port: port, ip: ip.core)
        case let .replay(path):
            return .replay(path: path, speed: 1.0)
        }
    }

    /// 履歴（コアの Profile）から戻す。
    init(_ profile: Profile) {
        switch profile {
        case .localShell:
            self = .localShell
        case let .serial(port, s):
            var c = SerialConfig()
            c.baud = s.baud
            c.dataBits = s.dataBits
            c.parity = {
                switch s.parity {
                case .none: return .none
                case .odd: return .odd
                case .even: return .even
                case .mark: return .mark
                case .space: return .space
                }
            }()
            c.stopBits = s.stopBits == .one ? .one : .two
            c.flow = {
                switch s.flow {
                case .none: return .none
                case .xonXoff: return .xonXoff
                case .rtsCts: return .rtsCts
                case .dsrDtr: return .dsrDtr
                }
            }()
            c.delayPerCharMs = s.delayPerCharMs
            c.delayPerLineMs = s.delayPerLineMs
            self = .serial(port: port, config: c)
        case let .ssh(host, port, user, system):
            self = .ssh(host: host, port: port, user: user, systemSSH: system)
        case let .telnet(host, port, ip):
            self = .telnet(host: host, port: port, ip: IP(ip))
        case let .tcp(host, port, ip):
            self = .tcp(host: host, port: port, ip: IP(ip))
        case let .replay(path, _):
            self = .replay(path: path)
        }
    }

    /// タイトルに出す名前。
    var label: String {
        switch self {
        case .localShell:
            return L("ローカルシェル")
        case let .serial(port, config):
            return "\(port) - \(config.summary)"
        case let .ssh(host, port, user, _):
            var s = user.map { "\($0)@" } ?? ""
            s += host
            if let port { s += ":\(port)" }
            return "\(s) (SSH)"
        case let .telnet(host, port, _):
            return port.map { "\(host):\($0) (Telnet)" } ?? "\(host) (Telnet)"
        case let .tcp(host, port, _):
            return "\(host):\(port)"
        case let .replay(path):
            return L("%@（再生）", URL(fileURLWithPath: path).lastPathComponent)
        }
    }

    /// ホスト名（TCP/IP のときだけ）。
    var host: String? {
        switch self {
        case let .ssh(host, _, _, _), let .telnet(host, _, _), let .tcp(host, _, _): return host
        default: return nil
        }
    }

    var isLocalShell: Bool { self == .localShell }

    var serialPort: String? {
        if case let .serial(port, _) = self { return port }
        return nil
    }
}

extension ConnectTarget.IP {
    var core: IpVersion {
        switch self {
        case .auto: return .auto
        case .v4: return .v4
        case .v6: return .v6
        }
    }

    init(_ v: IpVersion) {
        switch v {
        case .auto: self = .auto
        case .v4: self = .v4
        case .v6: self = .v6
        }
    }

    var label: String {
        switch self {
        case .auto: return "AUTO"
        case .v4: return "IPv4"
        case .v6: return "IPv6"
        }
    }
}

/// 新しいウィンドウ（またはタブ）で何を開くか。`openWindow(value:)` で渡す。
struct ConnectRequest: Codable, Hashable {
    enum Target: Codable, Hashable {
        /// 「新しい接続」ダイアログを出す
        case dialog
        case connect(ConnectTarget)
    }

    var id = UUID()
    var target: Target
    /// 今のウィンドウのタブとして開く
    var asTab = false
}
