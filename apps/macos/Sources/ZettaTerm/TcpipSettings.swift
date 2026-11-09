import ZettaCore
import SwiftUI

/// 「設定 > TCP/IP」：ホストリスト（履歴）の並べ替えと削除、端末タイプ、Telnet のキープアライブ。
struct TcpipSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @ObservedObject private var hosts = HostStore.shared
    @State private var selected: Int?
    @State private var termType = UserDefaults.standard.string(forKey: "tcpip.termType") ?? ""
    @State private var keepalive = UserDefaults.standard.integer(forKey: "telnet.keepalive")

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_TCPIP_TITLE", "TCP/IP")).font(.headline)
            Text(TT.text("DLG_TCPIP_HOSTLIST", "ホストリスト"))
            HStack(alignment: .top) {
                List(selection: $selected) {
                    ForEach(Array(hosts.recent.enumerated()), id: \.offset) { i, t in
                        Text(t.label).tag(i)
                    }
                }
                .frame(minHeight: 160)
                VStack {
                    Button(TT.text("DLG_TCPIP_UP", "上へ")) { move(-1) }
                        .disabled((selected ?? 0) == 0)
                    Button(TT.text("DLG_TCPIP_DOWN", "下へ")) { move(1) }
                        .disabled(selected.map { $0 >= hosts.recent.count - 1 } ?? true)
                    Button(TT.text("DLG_TCPIP_REMOVE", "削除")) {
                        if let i = selected {
                            hosts.remove(at: i)
                            selected = hosts.recent.isEmpty ? nil : min(i, hosts.recent.count - 1)
                        }
                    }
                    .disabled(selected == nil)
                }
            }
            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                GridRow {
                    Text(TT.text("DLG_TCPIP_TERMTYPE", "端末タイプ:"))
                    TextField(L("既定（Telnet は xterm、SSH は xterm-256color）"), text: $termType)
                }
                GridRow {
                    Text(TT.text("DLG_TCPIP_KEEPALIVE", "Telnet Keep-alive"))
                    HStack {
                        TextField("", value: $keepalive, format: .number.grouping(.never)).frame(width: 60)
                        Text(TT.text("DLG_TCPIP_KEEPALIVE_SEC", "秒(0指定で無効)"))
                    }
                }
            }
            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    TcpipSettings.save(termType: termType, keepalive: max(0, keepalive))
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 500)
    }

    private func move(_ delta: Int) {
        guard let i = selected, i + delta >= 0, i + delta < hosts.recent.count else { return }
        hosts.move(from: i, to: i + delta)
        selected = i + delta
    }
}

enum TcpipSettings {
    /// 起動時に、保存した端末タイプをコアに渡す。
    static func apply() {
        setTermType(term: UserDefaults.standard.string(forKey: "tcpip.termType") ?? "")
    }

    static func save(termType: String, keepalive: Int) {
        let d = UserDefaults.standard
        d.set(termType.trimmingCharacters(in: .whitespaces), forKey: "tcpip.termType")
        d.set(keepalive, forKey: "telnet.keepalive")
        apply()
        setTelnetKeepalive(secs: UInt32(clamping: keepalive))
    }
}
