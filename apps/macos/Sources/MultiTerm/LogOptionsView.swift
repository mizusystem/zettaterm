import AppKit
import MTCore
import SwiftUI

/// 「ログ」ダイアログ。ファイル名、書き込みモード、テキスト/バイナリ、タイムスタンプ。
struct LogOptionsView: View {
    @ObservedObject var controller: TerminalController
    @Environment(\.dismiss) private var dismiss

    @AppStorage("log.path") private var path = LogOptionsView.defaultPath
    @AppStorage("log.append") private var append = true
    @AppStorage("log.binary") private var binary = false
    @AppStorage("log.timestamp") private var timestamp = false
    @AppStorage("log.utc") private var utc = false

    static var defaultPath: String {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
        return (docs ?? URL(fileURLWithPath: NSHomeDirectory())).appendingPathComponent("multiterm.log").path
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_FOPT_TITLE", "ログ")).font(.headline)

            HStack {
                TextField("", text: $path)
                Button("…") { choose() }
            }

            HStack(alignment: .top, spacing: 32) {
                VStack(alignment: .leading, spacing: 6) {
                    Text(TT.text("DLG_FOPT_APPEND_LABEL", "書き込みモード")).foregroundStyle(.secondary)
                    Picker("", selection: $append) {
                        Text(TT.text("DLG_FOPT_NEW_OVERWRITE", "新規/上書き")).tag(false)
                        Text(TT.text("DLG_FOPT_APPEND", "追記")).tag(true)
                    }
                    .pickerStyle(.radioGroup).labelsHidden()
                }
                VStack(alignment: .leading, spacing: 6) {
                    Text(TT.text("DLG_FOPT_BINARY_LABEL", "テキスト/バイナリモード")).foregroundStyle(.secondary)
                    Picker("", selection: $binary) {
                        Text(TT.text("DLG_FOPT_TEXT", "テキスト")).tag(false)
                        Text(TT.text("DLG_FOPT_BINARY", "バイナリ")).tag(true)
                    }
                    .pickerStyle(.radioGroup).labelsHidden()
                }
            }

            HStack {
                Toggle(TT.text("DLG_FOPT_TIMESTAMP", "タイムスタンプ"), isOn: $timestamp)
                Picker("", selection: $utc) {
                    Text(TT.text("DLG_FOPT_TIMESTAMP_LOCAL", "ローカルタイム")).tag(false)
                    Text(TT.text("DLG_FOPT_TIMESTAMP_UTC", "UTC")).tag(true)
                }
                .labelsHidden().fixedSize()
                .disabled(!timestamp)
            }
            .disabled(binary)

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) { start() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(path.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 520)
    }

    private func choose() {
        let panel = NSSavePanel()
        let url = URL(fileURLWithPath: path)
        panel.directoryURL = url.deletingLastPathComponent()
        panel.nameFieldStringValue = url.lastPathComponent
        panel.canCreateDirectories = true
        if panel.runModal() == .OK, let chosen = panel.url {
            path = chosen.path
        }
    }

    private func start() {
        let stamp: LogTimestamp = (binary || !timestamp) ? .none : (utc ? .utc : .local)
        dismiss()
        controller.startLog(LogOptions(path: path, append: append, binary: binary, timestamp: stamp))
    }
}
