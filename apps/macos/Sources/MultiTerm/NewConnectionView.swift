import MTCore
import SwiftUI

/// 「新しい接続」ダイアログ。Tera Term と同じく TCP/IP（ホスト・サービス・ポート）とシリアルを並べ、
/// 当面はローカルシェルも選べるようにしている。
struct NewConnectionView: View {
    enum Kind: Hashable { case tcpip, serial, localShell }
    enum Service: Hashable { case telnet, ssh, other }

    @ObservedObject var controller: TerminalController
    @ObservedObject private var devices = DeviceStore.shared
    @ObservedObject private var hosts = HostStore.shared
    @Environment(\.dismiss) private var dismiss

    @AppStorage("newConnection.kind") private var savedKind = "tcpip"
    @AppStorage("newConnection.addToHistory") private var addToHistory = true
    @AppStorage("ssh.systemSSH") private var systemSSH = false
    @State private var kind: Kind = .tcpip
    @State private var host = ""
    /// SSH のユーザー名。空なら ~/.ssh/config の User か、この Mac のユーザー名
    @State private var user = ""
    @State private var service: Service = .ssh
    @State private var portText = "22"
    @State private var ip: ConnectTarget.IP = .auto
    @State private var serialPort = ""

    private var port: UInt16? { UInt16(portText.trimmingCharacters(in: .whitespaces)) }

    private var canConnect: Bool {
        switch kind {
        case .tcpip: return !host.trimmingCharacters(in: .whitespaces).isEmpty && port != nil
        case .serial: return !serialPort.isEmpty
        case .localShell: return true
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_HOST_TITLE", "新しい接続")).font(.headline)

            // TCP/IP
            radio(.tcpip, "TCP/IP")
            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 10) {
                GridRow {
                    Text(TT.text("DLG_HOST_TCPIPHOST", "ホスト:", section: "TTSSH")).gridColumnAlignment(.trailing)
                    HStack(spacing: 4) {
                        TextField(L("例：web、user@host、192.0.2.10"), text: $host)
                            .frame(minWidth: 260)
                            .onSubmit { if canConnect { connect() } }
                        if !hosts.hostSuggestions.isEmpty {
                            Menu {
                                ForEach(hosts.hostSuggestions, id: \.self) { h in
                                    Button(h) { pickHost(h) }
                                }
                            } label: {
                                Image(systemName: "chevron.down")
                            }
                            .menuStyle(.borderlessButton)
                            .menuIndicator(.hidden)
                            .fixedSize()
                        }
                    }
                }
                GridRow {
                    Color.clear.frame(width: 1, height: 1)
                    Toggle(TT.text("DLG_HOST_TCPIPHISTORY", "ホストリストへ追加", section: "TTSSH"), isOn: $addToHistory)
                }
                if service == .ssh {
                    GridRow {
                        Text(TT.text("DLG_AUTH_USERNAME", "ユーザ名:", section: "TTSSH"))
                        TextField(L("空なら ~/.ssh/config の User か、この Mac のユーザー名"), text: $user)
                            .frame(minWidth: 260)
                            .onSubmit { if canConnect { connect() } }
                    }
                }
                GridRow {
                    Text(TT.text("DLG_HOST_TCPIPSERVICE", "サービス:", section: "TTSSH"))
                    HStack(spacing: 24) {
                        Picker("", selection: $service) {
                            Text("Telnet").tag(Service.telnet)
                            Text("SSH").tag(Service.ssh)
                            Text(TT.text("DLG_HOST_TCPIPOTHER", "その他", section: "TTSSH")).tag(Service.other)
                        }
                        .pickerStyle(.radioGroup)
                        .labelsHidden()
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Text(TT.text("DLG_HOST_TCPIPPORT", "TCPポート#:"))
                                TextField("", text: $portText).frame(width: 64)
                            }
                            HStack {
                                Text(TT.text("DLG_HOST_TCPIPPROTOCOL", "IPバージョン:"))
                                Picker("", selection: $ip) {
                                    ForEach(ConnectTarget.IP.allCases, id: \.self) { Text($0.label).tag($0) }
                                }
                                .labelsHidden()
                                .fixedSize()
                            }
                        }
                    }
                }
            }
            .padding(.leading, 18)
            .disabled(kind != .tcpip)

            if kind == .tcpip && service == .ssh {
                VStack(alignment: .leading, spacing: 6) {
                    Toggle(L("システムの ssh を使う（認証は端末の中で行う）"), isOn: $systemSSH)
                    Text(systemSSH
                         ? L("システムの ssh を起動します。~/.ssh/config のすべての項目が効きます。")
                         : L("~/.ssh/config の Host 名・User・Port・鍵・ProxyJump・ProxyCommand が効きます。パスワードなどはダイアログで尋ねます。"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.leading, 18)
            }

            Divider()

            // シリアル
            radio(.serial, TT.text("DLG_HOST_SERIAL", "シリアル"))
            HStack {
                Text(TT.text("DLG_HOST_SERIALPORT", "ポート:"))
                PortPicker(port: $serialPort, devices: devices.devices)
            }
            .disabled(kind != .serial)
            .padding(.leading, 18)
            if kind == .serial && devices.devices.isEmpty {
                Text(L("シリアルデバイスが見つかりません。USB シリアルを挿すと一覧に出ます。"))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.leading, 18)
            }

            Divider()

            radio(.localShell, L("ローカルシェル"))

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) { connect() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canConnect)
            }
        }
        .padding(20)
        .frame(width: 560)
        .onAppear(perform: chooseDefaults)
        .onChange(of: service) { old, new in
            // 既定のポートのままなら、サービスに合わせて変える
            if portText == defaultPort(old) || portText.isEmpty { portText = defaultPort(new) }
        }
        .onChange(of: devices.devices) { _, _ in
            if serialPort.isEmpty, let first = devices.devices.first { serialPort = first.path }
        }
    }

    private func radio(_ k: Kind, _ label: String) -> some View {
        Button {
            kind = k
        } label: {
            HStack(spacing: 6) {
                Image(systemName: kind == k ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(kind == k ? Color.accentColor : .secondary)
                Text(label).fontWeight(.medium)
            }
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(kind == k ? [.isButton, .isSelected] : .isButton)
    }

    private func defaultPort(_ s: Service) -> String {
        switch s {
        case .telnet: return "23"
        case .ssh: return "22"
        case .other: return portText
        }
    }

    private func pickHost(_ h: String) {
        host = h
        user = ""
        guard let last = hosts.last(forHost: h) else {
            // ssh_config の Host 名は SSH で開く
            service = .ssh
            portText = "22"
            return
        }
        switch last {
        case let .ssh(_, p, u, system):
            service = .ssh; portText = String(p ?? 22); systemSSH = system; user = u ?? ""
        case let .telnet(_, p, i): service = .telnet; portText = String(p ?? 23); ip = i
        case let .tcp(_, p, i): service = .other; portText = String(p); ip = i
        default: break
        }
    }

    private func chooseDefaults() {
        hosts.reload()
        if let saved = SerialConfig.savedPort, devices.contains(saved) {
            serialPort = saved
        } else {
            serialPort = devices.devices.first(where: { $0.vid != nil })?.path ?? devices.devices.first?.path ?? ""
        }
        // 前回の種類。ただし USB シリアルが新しく挿さっていればシリアルを選ぶ
        kind = savedKind == "serial" ? .serial : savedKind == "localShell" ? .localShell : .tcpip
        if let latest = hosts.recent.first(where: { $0.host != nil }), let h = latest.host {
            pickHost(h)
        }
    }

    private func connect() {
        dismiss()
        let target: ConnectTarget
        switch kind {
        case .tcpip:
            savedKind = "tcpip"
            let h = host.trimmingCharacters(in: .whitespaces)
            switch service {
            case .ssh:
                // ホスト欄に user@host と書いてあれば、そちらを使う
                let u = user.trimmingCharacters(in: .whitespaces)
                target = .ssh(host: h, port: port == 22 ? nil : port,
                              user: u.isEmpty || h.contains("@") ? nil : u, systemSSH: systemSSH)
            case .telnet: target = .telnet(host: h, port: port == 23 ? nil : port, ip: ip)
            case .other: target = .tcp(host: h, port: port ?? 23, ip: ip)
            }
            controller.connect(target, remember: addToHistory)
        case .serial:
            savedKind = "serial"
            controller.connect(.serial(port: serialPort, config: SerialConfig.saved), remember: true)
        case .localShell:
            savedKind = "localShell"
            controller.connect(.localShell)
        }
    }
}

/// シリアルポートの選択。抜かれたポートも、選択中なら「（未接続）」として残す。
struct PortPicker: View {
    @Binding var port: String
    let devices: [SerialDevice]

    var body: some View {
        Picker("", selection: $port) {
            if port.isEmpty {
                Text("—").tag("")
            }
            ForEach(devices) { d in
                Text(d.displayName).tag(d.path)
            }
            if !port.isEmpty && !devices.contains(where: { $0.path == port }) {
                Text(L("%@（未接続）", port)).tag(port)
            }
        }
        .labelsHidden()
        .frame(minWidth: 280)
    }
}
