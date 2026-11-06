import MTCore
import SwiftUI

/// 「シリアルポート」設定。Tera Term と同じく、状況に応じて
/// 「新規オープン」「クローズ＆新規オープン」「現在の接続を再設定」を出し分ける。
struct SerialSettingsView: View {
    @ObservedObject var controller: TerminalController
    @ObservedObject private var devices = DeviceStore.shared
    @Environment(\.dismiss) private var dismiss

    @State private var port = ""
    @State private var config = SerialConfig()
    @State private var baudText = ""

    private enum Action { case open, closeAndOpen, reset }

    private var action: Action {
        guard controller.status != .disconnected else { return .open }
        if let current = controller.serialPort, current == port { return .reset }
        return .closeAndOpen
    }

    private var actionLabel: String {
        switch action {
        case .open: return TT.text("DLG_SERIAL_OK_OPEN", "新規オープン")
        case .closeAndOpen: return TT.text("DLG_SERIAL_OK_CLOSEOPEN", "クローズ＆新規オープン")
        case .reset: return TT.text("DLG_SERIAL_OK_RESET", "現在の接続を再設定")
        }
    }

    private var baud: UInt32? {
        guard let v = UInt32(baudText.trimmingCharacters(in: .whitespaces)), v > 0 else { return nil }
        return v
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(TT.text("DLG_SERIAL_TITLE", "シリアルポート")).font(.headline)

            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                GridRow {
                    label("DLG_SERIAL_PORT", "ポート:")
                    PortPicker(port: $port, devices: devices.devices)
                }
                GridRow {
                    label("DLG_SERIAL_BAUD", "スピード:")
                    HStack(spacing: 4) {
                        TextField("", text: $baudText)
                            .frame(width: 120)
                        Menu {
                            ForEach(SerialConfig.baudPresets, id: \.self) { b in
                                Button(String(b)) { baudText = String(b) }
                            }
                        } label: {
                            Image(systemName: "chevron.down")
                        }
                        .menuStyle(.borderlessButton)
                        .menuIndicator(.hidden)
                        .fixedSize()
                        if baud == nil {
                            Text(L("数値を入れてください")).font(.caption).foregroundStyle(.red)
                        }
                    }
                }
                GridRow {
                    label("DLG_SERIAL_DATA", "データ:")
                    Picker("", selection: $config.dataBits) {
                        ForEach([UInt8(7), 8], id: \.self) { Text(SerialConfig.dataLabel($0)).tag($0) }
                    }
                    .labelsHidden().fixedSize()
                }
                GridRow {
                    label("DLG_SERIAL_PARITY", "パリティ:")
                    Picker("", selection: $config.parity) {
                        ForEach(SerialConfig.ParityChoice.allCases, id: \.self) {
                            Text(SerialConfig.label($0)).tag($0)
                        }
                    }
                    .labelsHidden().fixedSize()
                }
                GridRow {
                    label("DLG_SERIAL_STOP", "ストップビット:")
                    Picker("", selection: $config.stopBits) {
                        ForEach(SerialConfig.StopChoice.allCases, id: \.self) {
                            Text(SerialConfig.label($0)).tag($0)
                        }
                    }
                    .labelsHidden().fixedSize()
                }
                GridRow {
                    label("DLG_SERIAL_FLOW", "フロー制御:")
                    Picker("", selection: $config.flow) {
                        ForEach(SerialConfig.FlowChoice.allCases, id: \.self) {
                            Text(SerialConfig.label($0)).tag($0)
                        }
                    }
                    .labelsHidden().fixedSize()
                }
                GridRow {
                    Text(TT.text("DLG_SERIAL_DELAY", "送信遅延"))
                    HStack {
                        TextField("", value: $config.delayPerCharMs, format: .number).frame(width: 60)
                        Text(TT.text("DLG_SERIAL_DELAYCHAR", "ミリ秒/字"))
                        TextField("", value: $config.delayPerLineMs, format: .number).frame(width: 60)
                        Text(TT.text("DLG_SERIAL_DELAYLINE", "ミリ秒/行"))
                    }
                }
            }

            if config.parity == .mark || config.parity == .space || config.flow == .dsrDtr {
                Text(L("この設定は macOS ではまだ使えません（mark / space パリティ、DSR/DTR フロー制御）。"))
                    .font(.callout)
                    .foregroundStyle(.orange)
            }

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(actionLabel, action: apply)
                    .keyboardShortcut(.defaultAction)
                    .disabled(port.isEmpty || baud == nil)
            }
        }
        .padding(20)
        .frame(width: 520)
        .onAppear(perform: load)
    }

    private func label(_ key: String, _ fallback: String) -> some View {
        Text(TT.text(key, fallback)).gridColumnAlignment(.trailing)
    }

    private func load() {
        config = controller.isSerialConnected ? controller.serialConfig : SerialConfig.saved
        baudText = String(config.baud)
        if let current = controller.serialPort {
            port = current
        } else if let saved = SerialConfig.savedPort, devices.contains(saved) {
            port = saved
        } else {
            port = devices.devices.first(where: { $0.vid != nil })?.path ?? devices.devices.first?.path ?? ""
        }
    }

    private func apply() {
        guard let baud else { return }
        var c = config
        c.baud = baud
        dismiss()
        switch action {
        case .open, .closeAndOpen: controller.connect(.serial(port: port, config: c), remember: true)
        case .reset: controller.reconfigureSerial(c)
        }
    }
}
