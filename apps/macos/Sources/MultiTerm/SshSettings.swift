import MTCore
import SwiftUI

/// TTSSH の「SSH」（キープアライブ）と「SSH 転送」（ポート転送）。内蔵 SSH の接続すべてに効く。
enum SshSettings {
    private static let forwardsKey = "ssh.forwards"
    private static let keepaliveKey = "ssh.keepalive"
    private static let agentKey = "ssh.forwardAgent"

    /// エージェント転送（オフでも ~/.ssh/config の ForwardAgent yes は効く）
    static var forwardAgent: Bool {
        get { UserDefaults.standard.bool(forKey: agentKey) }
        set { UserDefaults.standard.set(newValue, forKey: agentKey) }
    }

    /// `L 8080:host:80` / `R 2222:localhost:22` / `D 1080` の形
    static var forwards: [String] {
        get { UserDefaults.standard.stringArray(forKey: forwardsKey) ?? [] }
        set { UserDefaults.standard.set(newValue, forKey: forwardsKey) }
    }

    /// 秒。0 なら送らない（TTSSH の HeartBeat の既定と同じ 60 秒）
    static var keepalive: Int {
        get { UserDefaults.standard.object(forKey: keepaliveKey) == nil ? 60 : UserDefaults.standard.integer(forKey: keepaliveKey) }
        set { UserDefaults.standard.set(max(0, newValue), forKey: keepaliveKey) }
    }

    /// コアに渡す（起動時と、設定を変えたとき）。読めなかった行を返す。
    @discardableResult
    static func apply() -> [String] {
        setSshDefaults(forwards: forwards, keepaliveSecs: UInt32(clamping: keepalive), forwardAgent: forwardAgent)
    }
}

/// 「設定 > SSH...」：キープアライブとポート転送。
struct SshSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var keepalive = String(SshSettings.keepalive)
    @State private var forwards = SshSettings.forwards.joined(separator: "\n")
    @State private var bad: [String] = []
    @State private var agent = SshSettings.forwardAgent

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_SSHSETUP_TITLE", "設定", section: "TTSSH")).font(.headline)

            HStack {
                Text(TT.text("DLG_SSHSETUP_HEARTBEAT", "Keepalive", section: "TTSSH"))
                TextField("", text: $keepalive).frame(width: 60)
                Text(TT.text("DLG_SSHSETUP_HEARTBEAT_UNIT", "秒(0を指定すると無効)", section: "TTSSH"))
                    .foregroundStyle(.secondary)
            }

            Toggle(TT.text("DLG_SSHSETUP_FORWARDAGENT", "エージェント転送を有効にする", section: "TTSSH"), isOn: $agent)
                .help(L("接続先でさらに ssh するとき、手元の ssh-agent の鍵で認証できるようにします。信頼できるサーバーでだけ使ってください。"))

            VStack(alignment: .leading, spacing: 6) {
                Text(TT.text("DLG_FWDSETUP_LIST", "ポート転送", section: "TTSSH"))
                Text(L("1 行に 1 つ。L はローカル（L 8080:intranet:80）、R はリモート（R 2222:localhost:22）、D は SOCKS プロキシ（D 1080）。~/.ssh/config の LocalForward なども効きます。"))
                    .font(.callout).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                TextEditor(text: $forwards)
                    .font(.system(.body, design: .monospaced))
                    .frame(height: 120)
                if !bad.isEmpty {
                    Text(L("読めない行があります: %@", bad.joined(separator: ", ")))
                        .font(.callout).foregroundStyle(.red)
                }
            }

            Text(L("次の接続から効きます。"))
                .font(.callout).foregroundStyle(.secondary)

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) { save() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 520)
    }

    private func save() {
        let lines = forwards.split(whereSeparator: \.isNewline).map(String.init)
            .filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        let rejected = setSshDefaults(forwards: lines, keepaliveSecs: UInt32(clamping: Int(keepalive) ?? 60), forwardAgent: agent)
        if !rejected.isEmpty {
            // 読めない行があれば保存せず、前の設定に戻して直してもらう
            bad = rejected
            SshSettings.apply()
            return
        }
        SshSettings.forwards = lines
        SshSettings.keepalive = Int(keepalive) ?? 60
        SshSettings.forwardAgent = agent
        SshSettings.apply()
        dismiss()
    }
}

/// 「設定 > SSH 鍵生成...」（TTSSH の鍵生成）。OpenSSH の形式で ~/.ssh などに書く。
struct SshKeyGenView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var kind = SshKeyKind.ed25519
    @State private var comment = "\(NSUserName())@\(ProcessInfo.processInfo.hostName)"
    @State private var passphrase = ""
    @State private var confirm = ""
    @State private var path = ""
    @State private var working = false
    @State private var result: GeneratedSshKey?
    @State private var error: String?

    private static let kinds: [(SshKeyKind, String, String)] = [
        (.ed25519, "Ed25519", "id_ed25519"),
        (.ecdsaP256, "ECDSA (P-256)", "id_ecdsa"),
        (.ecdsaP384, "ECDSA (P-384)", "id_ecdsa"),
        (.ecdsaP521, "ECDSA (P-521)", "id_ecdsa"),
        (.rsa3072, "RSA 3072", "id_rsa"),
        (.rsa4096, "RSA 4096", "id_rsa"),
    ]

    private func defaultPath(_ k: SshKeyKind) -> String {
        let name = Self.kinds.first { $0.0 == k }?.2 ?? "id_ed25519"
        return (NSHomeDirectory() as NSString).appendingPathComponent(".ssh/\(name)")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(TT.text("DLG_KEYGEN_TITLE", "鍵生成", section: "TTSSH")).font(.headline)
            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                GridRow {
                    Text(TT.text("DLG_KEYGEN_KEYTYPE", "鍵の種類", section: "TTSSH")).gridColumnAlignment(.trailing)
                    Picker("", selection: $kind) {
                        ForEach(Self.kinds, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    .labelsHidden().fixedSize()
                    .onChange(of: kind) { old, new in
                        if path == defaultPath(old) { path = defaultPath(new) }
                    }
                }
                GridRow {
                    Text(TT.text("DLG_KEYGEN_COMMENT", "コメント:", section: "TTSSH"))
                    TextField("", text: $comment)
                }
                GridRow {
                    Text(TT.text("DLG_KEYGEN_PASSPHRASE", "鍵のパスフレーズ:", section: "TTSSH"))
                    SecureField("", text: $passphrase)
                }
                GridRow {
                    Text(TT.text("DLG_KEYGEN_PASSPHRASE2", "パスフレーズの確認:", section: "TTSSH"))
                    SecureField("", text: $confirm)
                }
                GridRow {
                    Text(TT.text("DLG_KEYGEN_SAVEPRIVATE", "秘密鍵の保存", section: "TTSSH"))
                    HStack {
                        TextField("", text: $path)
                        Button("…") { choose() }
                    }
                }
            }
            if let error {
                Text(error).foregroundStyle(.red).font(.callout).fixedSize(horizontal: false, vertical: true)
            }
            if let result {
                Text(TT.text("MSG_KEYGEN_GENERATED", "鍵を生成しました。", section: "TTSSH")).font(.callout)
                Text(result.fingerprint).font(.system(.caption, design: .monospaced))
                TextEditor(text: .constant(result.publicKey))
                    .font(.system(.caption, design: .monospaced))
                    .frame(height: 70)
                Text(L("公開鍵: %@", result.publicPath)).font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                if working { ProgressView().controlSize(.small) }
                Spacer()
                if let result {
                    Button(L("公開鍵をコピー")) {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(result.publicKey, forType: .string)
                    }
                }
                Button(result == nil ? TT.text("BTN_CANCEL", "キャンセル") : L("閉じる")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                if result == nil {
                    Button(TT.text("DLG_KEYGEN_GENERATE", "生成", section: "TTSSH")) { generate() }
                        .keyboardShortcut(.defaultAction)
                        .disabled(working || path.isEmpty)
                }
            }
        }
        .padding(20)
        .frame(width: 520)
        .onAppear { if path.isEmpty { path = defaultPath(kind) } }
    }

    private func choose() {
        let panel = NSSavePanel()
        let url = URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
        panel.directoryURL = url.deletingLastPathComponent()
        panel.nameFieldStringValue = url.lastPathComponent
        panel.canCreateDirectories = true
        if panel.runModal() == .OK, let chosen = panel.url { path = chosen.path }
    }

    private func generate() {
        guard passphrase == confirm else {
            error = L("パスフレーズが一致しません。")
            return
        }
        error = nil
        working = true
        let (k, c, p, file) = (kind, comment, passphrase, (path as NSString).expandingTildeInPath)
        Task {
            let outcome: Result<GeneratedSshKey, Error> = await Task.detached {
                Result { try generateSshKey(kind: k, comment: c, passphrase: p, path: file) }
            }.value
            working = false
            switch outcome {
            case let .success(key): result = key
            case let .failure(e): error = TerminalController.describe(e)
            }
        }
    }
}
