import AppKit
import ZettaCore
import SwiftUI

/// 「ログ」ダイアログ。ファイル名、書き込みモード、テキスト/バイナリ、タイムスタンプ、現在のバッファ。
/// ファイル名は開くたびに「設定 > ログ」の標準のファイル名（`&h_%Y%m%d_%H%M%S.log` など）から作る。
struct LogOptionsView: View {
    @ObservedObject var controller: TerminalController
    @Environment(\.dismiss) private var dismiss

    @State private var path = ""
    @AppStorage("log.includeScreen") private var includeScreen = false
    @AppStorage("log.append") private var append = true
    @AppStorage("log.binary") private var binary = false
    @AppStorage("log.timestamp") private var timestamp = false
    @AppStorage("log.utc") private var utc = false
    /// 0 ローカル、1 UTC、2 ログ開始からの経過、3 接続からの経過（log.utc は前の版からの値）
    @AppStorage("log.timestampKind") private var stampKind = -1
    @AppStorage("log.ttyrec") private var ttyrec = false
    @AppStorage("log.bom") private var bom = false

    /// ログの形（テキスト / バイナリ / ttyrec）
    private enum Kind: Int { case text, binary, ttyrec }
    private var kind: Binding<Kind> {
        Binding(get: { ttyrec ? .ttyrec : (binary ? .binary : .text) },
                set: { ttyrec = $0 == .ttyrec; binary = $0 == .binary })
    }

    static func stamp(_ d: UserDefaults = .standard) -> LogTimestamp {
        guard d.bool(forKey: "log.timestamp") else { return .none }
        let kind = d.object(forKey: "log.timestampKind") == nil ? -1 : d.integer(forKey: "log.timestampKind")
        switch kind {
        case 1: return .utc
        case 2: return .elapsedLogging
        case 3: return .elapsedConnection
        case 0: return .local
        default: return d.bool(forKey: "log.utc") ? .utc : .local
        }
    }

    /// ダイアログで最後に選んだ書き込みモードなどで、`path` に書くログの設定（自動開始に使う）。
    static func savedOptions(path: String, includeScreen: Bool = false) -> LogOptions {
        let d = UserDefaults.standard
        let binary = d.bool(forKey: "log.binary")
        let ttyrec = d.bool(forKey: "log.ttyrec")
        let append = d.object(forKey: "log.append") == nil ? true : d.bool(forKey: "log.append")
        let stamp: LogTimestamp = (binary || ttyrec) ? .none : Self.stamp(d)
        let config = LogConfig.saved
        return LogOptions(path: path, append: append, binary: binary, timestamp: stamp, includeScreen: includeScreen,
                          ttyrec: ttyrec, bom: d.bool(forKey: "log.bom"),
                          rotateKib: config.rotateKiB, rotateKeep: config.rotateKeep)
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
                    Picker("", selection: kind) {
                        Text(TT.text("DLG_FOPT_TEXT", "テキスト")).tag(Kind.text)
                        Text(TT.text("DLG_FOPT_BINARY", "バイナリ")).tag(Kind.binary)
                        Text(L("ttyrec（再生できる記録）")).tag(Kind.ttyrec)
                    }
                    .pickerStyle(.radioGroup).labelsHidden()
                }
            }

            HStack {
                Toggle(TT.text("DLG_FOPT_TIMESTAMP", "タイムスタンプ"), isOn: $timestamp)
                Picker("", selection: Binding(get: { stampKind < 0 ? (utc ? 1 : 0) : stampKind },
                                              set: { stampKind = $0 })) {
                    Text(TT.text("DLG_FOPT_TIMESTAMP_LOCAL", "ローカルタイム")).tag(0)
                    Text(TT.text("DLG_FOPT_TIMESTAMP_UTC", "UTC")).tag(1)
                    Text(TT.text("DLG_FOPT_TIMESTAMP_ELAPSED_LOGGING", "経過時間(Logging)")).tag(2)
                    Text(TT.text("DLG_FOPT_TIMESTAMP_ELAPSED_CONNECTION", "経過時間(Connection)")).tag(3)
                }
                .labelsHidden().fixedSize()
                .disabled(!timestamp)
            }
            .disabled(binary || ttyrec)

            Toggle(TT.text("DLG_FOPT_BOM", "BOM"), isOn: $bom)
                .disabled(binary || ttyrec)

            Toggle(TT.text("DLG_FOPT_ALLBUFFINFIRST", "現在バッファを含む"), isOn: $includeScreen)
                .help(L("スクロールバックを含めて、今画面に出ている内容を先に書きます。"))

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
        .onAppear { path = LogConfig.saved.path(for: controller.target) }
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
        dismiss()
        let folder = URL(fileURLWithPath: path).deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        controller.startLog(Self.savedOptions(path: path, includeScreen: includeScreen))
    }
}
