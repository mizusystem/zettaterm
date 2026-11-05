import MTCore
import SwiftUI

/// 「新しい接続」ダイアログ。Tera Term の TCP/IP の代わりに、当面はローカルシェルを選べる。
struct NewConnectionView: View {
    enum Kind: Hashable { case localShell, serial }

    @ObservedObject var controller: TerminalController
    @ObservedObject private var devices = DeviceStore.shared
    @Environment(\.dismiss) private var dismiss

    @State private var kind: Kind = .localShell
    @State private var port = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(TT.text("DLG_HOST_TITLE", "新しい接続")).font(.headline)

            Picker("", selection: $kind) {
                Text("ローカルシェル").tag(Kind.localShell)
                Text(TT.text("DLG_HOST_SERIAL", "シリアル")).tag(Kind.serial)
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()

            HStack {
                Text(TT.text("DLG_HOST_SERIALPORT", "ポート:"))
                PortPicker(port: $port, devices: devices.devices)
            }
            .disabled(kind != .serial)
            .padding(.leading, 18)

            if kind == .serial && devices.devices.isEmpty {
                Text("シリアルデバイスが見つかりません。USB シリアルを挿すと一覧に出ます。")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.leading, 18)
            }

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) { connect() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(kind == .serial && port.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 460)
        .onAppear(perform: chooseDefaults)
        .onChange(of: devices.devices) { _, _ in
            // 開いている間に挿されたら、そのデバイスを選ぶ
            if port.isEmpty, let first = devices.devices.first { port = first.path }
        }
    }

    private func chooseDefaults() {
        let saved = SerialConfig.savedPort
        if let saved, devices.contains(saved) {
            port = saved
        } else {
            port = devices.devices.first(where: { $0.vid != nil })?.path ?? devices.devices.first?.path ?? ""
        }
        // USB シリアルがつながっていればシリアルを先に選んでおく
        kind = devices.devices.contains(where: { $0.vid != nil }) ? .serial : .localShell
    }

    private func connect() {
        dismiss()
        switch kind {
        case .localShell: controller.connectLocalShell()
        case .serial: controller.connectSerial(port: port, config: SerialConfig.saved)
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
                Text("\(port)（未接続）").tag(port)
            }
        }
        .labelsHidden()
        .frame(minWidth: 280)
    }
}
