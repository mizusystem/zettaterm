import SwiftUI

/// 「端末」設定。当面は改行コード（受信・送信）とローカルエコー。
struct TerminalSettingsView: View {
    @ObservedObject var controller: TerminalController
    @Environment(\.dismiss) private var dismiss
    @State private var config = TerminalConfig()

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(TT.text("DLG_TERM_TITLE", "端末")).font(.headline)

            GroupBox(TT.text("DLG_TERM_NEWLINE", "改行コード")) {
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                    GridRow {
                        Text(TT.text("DLG_TERM_CRRCV", "受信:")).gridColumnAlignment(.trailing)
                        Picker("", selection: $config.receive) {
                            ForEach(TerminalConfig.Receive.allCases, id: \.self) {
                                Text(TerminalConfig.label($0)).tag($0)
                            }
                        }
                        .labelsHidden().fixedSize()
                    }
                    GridRow {
                        Text(TT.text("DLG_TERM_CRSEND", "送信:"))
                        Picker("", selection: $config.send) {
                            ForEach(TerminalConfig.Send.allCases, id: \.self) {
                                Text(TerminalConfig.label($0)).tag($0)
                            }
                        }
                        .labelsHidden().fixedSize()
                    }
                }
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            GroupBox(L("エンコーディング")) {
                Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                    GridRow {
                        Text(TT.text("DLG_TERM_KANJI", "エンコーディング-受信:")).gridColumnAlignment(.trailing)
                        Picker("", selection: $config.charsetReceive) {
                            ForEach(TerminalConfig.CharsetChoice.allCases, id: \.self) { Text($0.label).tag($0) }
                        }
                        .labelsHidden().fixedSize()
                    }
                    GridRow {
                        Text(TT.text("DLG_TERM_KANJISEND", "エンコーディング-送信:"))
                        Picker("", selection: $config.charsetSend) {
                            ForEach(TerminalConfig.CharsetChoice.allCases, id: \.self) { Text($0.label).tag($0) }
                        }
                        .labelsHidden().fixedSize()
                    }
                }
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            Toggle(TT.text("DLG_TERM_LOCALECHO", "ローカルエコー:").trimmingCharacters(in: CharacterSet(charactersIn: ":：")),
                   isOn: $config.localEcho)

            Text(controller.target?.serialPort != nil
                 ? L("シリアル接続用の設定として保存します。LF だけで改行する機器（ESP32 など）は受信を LF か AUTO にします。")
                 : L("シェル・SSH・Telnet 用の設定として保存します（シリアル用とは別）。通常は受信 CR のままにします。"))
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    dismiss()
                    controller.applyTerminal(config)
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 420)
        .onAppear { config = controller.terminalConfig }
    }
}
