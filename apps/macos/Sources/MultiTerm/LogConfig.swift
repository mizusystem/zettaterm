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
    /// この大きさ（KiB）を超えたらローテートする（0 はしない。LogRotateSize）
    var rotateKiB: UInt32 = 0
    /// 残す古いファイルの数（LogRotateStep）
    var rotateKeep: UInt32 = 5
    /// 「ログを表示」で開くアプリ（ViewlogEditor。空なら既定のアプリ）
    var viewer = ""

    static var documents: String {
        let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
        return (docs ?? URL(fileURLWithPath: NSHomeDirectory())).path
    }

    private enum Key {
        static let folder = "log.defaultFolder"
        static let name = "log.defaultName"
        static let autoStart = "log.autoStart"
        static let rotateKiB = "log.rotateKiB"
        static let rotateKeep = "log.rotateKeep"
        static let viewer = "log.viewer"
    }

    static var saved: LogConfig {
        let d = UserDefaults.standard
        var c = LogConfig()
        if let f = d.string(forKey: Key.folder), !f.isEmpty { c.folder = f }
        if let n = d.string(forKey: Key.name), !n.isEmpty { c.name = n }
        c.autoStart = d.bool(forKey: Key.autoStart)
        c.rotateKiB = UInt32(clamping: d.integer(forKey: Key.rotateKiB))
        if d.object(forKey: Key.rotateKeep) != nil { c.rotateKeep = UInt32(clamping: d.integer(forKey: Key.rotateKeep)) }
        c.viewer = d.string(forKey: Key.viewer) ?? ""
        return c
    }

    func save() {
        let d = UserDefaults.standard
        d.set(folder, forKey: Key.folder)
        d.set(name, forKey: Key.name)
        d.set(autoStart, forKey: Key.autoStart)
        d.set(Int(rotateKiB), forKey: Key.rotateKiB)
        d.set(Int(rotateKeep), forKey: Key.rotateKeep)
        d.set(viewer, forKey: Key.viewer)
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
        case let .unix(path): return ((path as NSString).lastPathComponent, "unix")
        case .localShell, .replay, .none: return ("localhost", "shell")
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
                Text(TT.text("DLG_TAB_LOG_EDITOR", "ログ表示用エディタ"))
                TextField(L("既定のアプリ"), text: Binding(
                    get: { config.viewer.isEmpty ? "" : (config.viewer as NSString).lastPathComponent },
                    set: { if $0.isEmpty { config.viewer = "" } }))
                    .disabled(true)
                Button("…") { chooseViewer() }
                if !config.viewer.isEmpty {
                    Button(L("既定に戻す")) { config.viewer = "" }
                }
            }

            HStack {
                Toggle(TT.text("DLG_TAB_LOG_ROTATE", "ログのローテート"), isOn: Binding(
                    get: { config.rotateKiB > 0 },
                    set: { config.rotateKiB = $0 ? max(config.rotateKiB, 10_240) : 0 }))
                TextField("", value: Binding(get: { Int(config.rotateKiB) / 1024 },
                                             set: { config.rotateKiB = UInt32(clamping: max(1, $0) * 1024) }),
                          format: .number)
                    .frame(width: 60)
                    .disabled(config.rotateKiB == 0)
                Text("MiB")
                Text(TT.text("DLG_TAB_LOG_ROTATESTEP", "世代"))
                Stepper(value: $config.rotateKeep, in: 1...99) { Text("\(config.rotateKeep)") }
                    .disabled(config.rotateKiB == 0)
            }

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

    private func chooseViewer() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [.application]
        panel.directoryURL = URL(fileURLWithPath: "/Applications")
        if panel.runModal() == .OK, let url = panel.url { config.viewer = url.path }
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
