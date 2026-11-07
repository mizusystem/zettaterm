import MTCore
import SwiftUI

/// TTProxy の「プロキシの設定」。Telnet・TCP・内蔵 SSH の接続に使う。
/// パスワードは保存しない（アプリを終了すると消える）。
enum ProxyConfig {
    enum Kind: String, CaseIterable { case none, http, socks4, socks5 }

    private static var d: UserDefaults { .standard }
    /// この実行の間だけ持つパスワード
    @MainActor static var password = ""

    static var kind: Kind {
        get { d.string(forKey: "proxy.kind").flatMap(Kind.init) ?? .none }
        set { d.set(newValue.rawValue, forKey: "proxy.kind") }
    }
    static var host: String {
        get { d.string(forKey: "proxy.host") ?? "" }
        set { d.set(newValue, forKey: "proxy.host") }
    }
    static var port: Int {
        get { d.object(forKey: "proxy.port") == nil ? 8080 : d.integer(forKey: "proxy.port") }
        set { d.set(newValue, forKey: "proxy.port") }
    }
    static var user: String {
        get { d.string(forKey: "proxy.user") ?? "" }
        set { d.set(newValue, forKey: "proxy.user") }
    }

    /// 今の設定をコアに渡す（起動時と、設定を変えたとき）。
    @MainActor
    static func apply() {
        let k: ProxyKind?
        switch kind {
        case .none: k = nil
        case .http: k = .http
        case .socks4: k = .socks4
        case .socks5: k = .socks5
        }
        guard let k, !host.isEmpty else {
            setProxy(proxy: nil)
            return
        }
        setProxy(proxy: ProxySetting(kind: k, host: host, port: UInt16(clamping: port),
                                     user: user.isEmpty ? nil : user,
                                     password: password.isEmpty ? nil : password))
    }
}

/// 「設定 > プロキシ...」。
struct ProxySettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var kind = ProxyConfig.kind
    @State private var host = ProxyConfig.host
    @State private var port = ProxyConfig.port
    @State private var user = ProxyConfig.user
    @State private var password = ProxyConfig.password

    private func t(_ key: String, _ fallback: String) -> String {
        TT.text(key, fallback, section: "TTProxy")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(t("DLG_SETUP_TITLE", "プロキシの設定")).font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                GridRow {
                    Text(t("DLG_SETUP_TYPE", "種類")).gridColumnAlignment(.trailing)
                    Picker("", selection: $kind) {
                        Text(L("使わない")).tag(ProxyConfig.Kind.none)
                        Text("HTTP").tag(ProxyConfig.Kind.http)
                        Text("SOCKS4").tag(ProxyConfig.Kind.socks4)
                        Text("SOCKS5").tag(ProxyConfig.Kind.socks5)
                    }
                    .labelsHidden().fixedSize()
                }
                GridRow {
                    Text(t("DLG_SETUP_HOST", "ホスト名"))
                    TextField("", text: $host)
                }
                GridRow {
                    Text(t("DLG_SETUP_PORT", "ポート番号"))
                    TextField("", value: $port, format: .number.grouping(.never)).frame(width: 80)
                }
                GridRow {
                    Text(t("DLG_SETUP_USER", "ユーザ名"))
                    TextField("", text: $user)
                }
                GridRow {
                    Text(t("DLG_SETUP_PASS", "パスワード"))
                    SecureField("", text: $password)
                }
            }
            .disabled(kind == .none)
            Text(L("Telnet・TCP・内蔵 SSH の接続に使います（ssh_config の ProxyCommand / ProxyJump があればそちら）。パスワードは保存せず、アプリを終了すると消えます。"))
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    ProxyConfig.kind = kind
                    ProxyConfig.host = host.trimmingCharacters(in: .whitespaces)
                    ProxyConfig.port = max(1, min(port, 65535))
                    ProxyConfig.user = user
                    ProxyConfig.password = password
                    ProxyConfig.apply()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 460)
    }
}
