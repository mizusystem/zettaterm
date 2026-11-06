import AppKit
import MTCore
import SwiftUI

/// Tera Term の「その他の設定 > ログ」：標準の保存先フォルダ、標準のファイル名、自動開始。
struct LogConfig: Equatable {
    /// 標準のログ保存先フォルダ（LogDefaultPath）
    var folder = LogConfig.documents
    /// 標準のログファイル名（LogDefaultName）。`&h` はホスト名、`&p` はポート、`&u` はユーザー名、`%Y` などは strftime
    var name = defaultLogName()
    /// つないだら自動でログを取り始める（LogAutoStart）
    var autoStart = false

    static var documents: String {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
        return (docs ?? URL(fileURLWithPath: NSHomeDirectory())).path
    }

    private enum Key {
        static let folder = "log.defaultFolder"
        static let name = "log.defaultName"
        static let autoStart = "log.autoStart"
    }

    static var saved: LogConfig {
        let d = UserDefaults.standard
        var c = LogConfig()
        if let f = d.string(forKey: Key.folder), !f.isEmpty { c.folder = f }
        if let n = d.string(forKey: Key.name), !n.isEmpty { c.name = n }
        c.autoStart = d.bool(forKey: Key.autoStart)
        return c
    }

    func save() {
        let d = UserDefaults.standard
        d.set(folder, forKey: Key.folder)
        d.set(name, forKey: Key.name)
        d.set(autoStart, forKey: Key.autoStart)
    }

    /// `target` 向けのログファイルのパス（テンプレートを今の時刻で展開したもの）。
    func path(for target: ConnectTarget?) -> String {
        let (host, port) = LogConfig.vars(target)
        let file = expandLogName(template: name, host: host, port: port)
        return URL(fileURLWithPath: (folder as NSString).expandingTildeInPath).appendingPathComponent(file).path
    }

    /// テンプレートの `&h` と `&p` に入れる値。
    static func vars(_ target: ConnectTarget?) -> (host: String, port: String) {
        switch target {
        case let .ssh(host, port, _, _): return (host, String(port ?? 22))
        case let .telnet(host, port, _): return (host, String(port ?? 23))
        case let .tcp(host, port, _): return (host, String(port))
        case let .serial(port, _): return (port, port)
        case .localShell, .none: return ("localhost", "shell")
        }
    }
}

/// 「設定 > ログ」。
struct LogSettingsView: View {
    @ObservedObject var controller: TerminalController
    @Environment(\.dismiss) private var dismiss
    @State private var config = LogConfig.saved

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_FOPT_TITLE", "ログ")).font(.headline)

            VStack(alignment: .leading, spacing: 4) {
                Text(TT.text("DLG_TAB_LOG_FILEPATH", "標準のログ保存先フォルダ"))
                HStack {
                    TextField("", text: $config.folder)
                    Button("…") { chooseFolder() }
                }
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(TT.text("DLG_TAB_LOG_FILENAME", "標準ログファイル名(strftimeフォーマット可)"))
                TextField("", text: $config.name)
                    .font(.system(.body, design: .monospaced))
                Text(L("&h はホスト名、&p はポート、&u はユーザー名。例：%@", config.path(for: controller.target)))
                    .font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Toggle(TT.text("DLG_TAB_LOG_AUTOSTART", "自動的にログ採取を開始する"), isOn: $config.autoStart)

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    if config.name.trimmingCharacters(in: .whitespaces).isEmpty { config.name = defaultLogName() }
                    config.save()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 520)
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.directoryURL = URL(fileURLWithPath: (config.folder as NSString).expandingTildeInPath)
        if panel.runModal() == .OK, let url = panel.url { config.folder = url.path }
    }
}

/// 「ファイル > ログにコメントを付加」。
struct LogCommentView: View {
    @ObservedObject var controller: TerminalController
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_COMMENT_TITLE", "ログにコメントを付加")).font(.headline)
            TextField("", text: $text)
                .frame(minWidth: 380)
                .onSubmit { write() }
            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) { write() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(text.isEmpty)
            }
        }
        .padding(20)
    }

    private func write() {
        dismiss()
        controller.writeLogComment(text)
    }
}
