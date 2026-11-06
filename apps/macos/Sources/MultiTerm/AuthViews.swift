import AppKit
import MTCore
import SwiftUI

/// SSH の問い合わせ（TTSSH の各ダイアログ）。文言は TTSSH セクションから引く。
struct AuthPromptView: View {
    @ObservedObject var controller: TerminalController

    var body: some View {
        Group {
            if let prompt = controller.authPrompt {
                switch prompt.kind {
                case let .hostKey(info): HostKeyView(info: info) { controller.answer(.hostKey($0)) }
                case let .credentials(request): SshAuthView(request: request) { controller.answer(.credentials($0)) }
                case let .keyboard(name, instructions, prompts):
                    KeyboardInteractiveView(host: controller.target?.host ?? "", name: name,
                                            instructions: instructions, prompts: prompts) {
                        controller.answer(.keyboard($0))
                    }
                case let .passphrase(path, failed):
                    PassphraseView(keyPath: path, failed: failed) { controller.answer(.passphrase($0)) }
                }
            } else {
                EmptyView()
            }
        }
        .padding(20)
        .frame(width: 560)
    }
}

private func ttssh(_ key: String, _ fallback: String) -> String {
    TT.text(key, fallback, section: "TTSSH")
}

/// 「%s」をホスト名に置き換える。
private func fill(_ s: String, _ host: String) -> String {
    s.replacingOccurrences(of: "%s", with: host)
}

// MARK: - ホスト鍵の確認（セキュリティ警告）

struct HostKeyView: View {
    let info: HostKeyInfo
    let done: (HostKeyDecision) -> Void
    @State private var save = true

    private var hostLabel: String { info.port == 22 ? info.host : "[\(info.host)]:\(info.port)" }

    private var prefix: String {
        switch info.status {
        case .unknown: return "DLG_UNKNOWNHOST"
        case .changed: return "DLG_DIFFERENTKEY"
        case .differentType: return "DLG_DIFFERENTTYPEKEY"
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Label(ttssh("\(prefix)_TITLE", "セキュリティ警告"), systemImage: "exclamationmark.shield")
                .font(.headline)
                .foregroundStyle(info.status == .changed ? .red : .primary)
            Text(fill(ttssh("\(prefix)_WARNING", "known hosts リストにサーバ \"%s\" のエントリはありません。"), hostLabel))
                .fixedSize(horizontal: false, vertical: true)
            GroupBox {
                VStack(alignment: .leading, spacing: 6) {
                    Text(ttssh("\(prefix)_FINGERPRINT", "サーバ側のホスト鍵指紋:")).foregroundStyle(.secondary)
                    Text(info.algorithm).font(.system(.body, design: .monospaced))
                    Text(info.fingerprint)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(4)
            }
            Text(ttssh("\(prefix)_WARNING2", "known hosts リストに追加して続行すると、次回からこの警告は出なくなります。"))
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            Toggle(info.status == .changed
                   ? ttssh("DLG_DIFFERENTKEY_REPLACE", "既存の鍵を、新しい鍵で上書きする")
                   : ttssh("\(prefix)_ADD", "このホストを known hosts リストに追加する"),
                   isOn: $save)
            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { done(.reject) }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_CONTINUE", "続行")) { done(save ? .acceptAndSave : .acceptOnce) }
                    // 鍵が変わったときは、うっかり Return で続行しないよう既定ボタンにしない
                    .keyboardShortcut(info.status == .changed ? nil : .defaultAction)
            }
        }
        // 鍵が変わったときは、上書きを既定にしない（TTSSH と同じ）
        .onAppear { save = info.status != .changed }
    }
}

// MARK: - SSH 認証

struct SshAuthView: View {
    enum Method: Hashable { case password, publicKey, keyboard, agent }

    let request: AuthRequest
    let done: (AuthResponse) -> Void

    @State private var user = ""
    @State private var secret = ""
    @State private var method: Method = .password
    @State private var keyPath = ""
    @State private var remember = false
    @State private var showSecret = false

    private func allowed(_ m: Method) -> Bool {
        switch m {
        case .password: return request.methods.contains(.password)
        case .publicKey: return request.methods.contains(.publicKey)
        case .keyboard: return request.methods.contains(.keyboardInteractive)
        case .agent: return request.methods.contains(.agent)
        }
    }

    private var hostLabel: String { request.port == 22 ? request.host : "\(request.host):\(request.port)" }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(request.failed ? ttssh("DLG_AUTH_TITLE_FAILED", "SSH認証を再試行中") : ttssh("DLG_AUTH_TITLE", "SSH認証"))
                .font(.headline)
            VStack(alignment: .leading, spacing: 2) {
                Text(fill(ttssh("DLG_AUTH_BANNER", "ログイン中: %s"), hostLabel))
                Text(request.failed ? ttssh("DLG_AUTH_BANNER2_FAILED", "認証に失敗しました. 再試行してください.")
                     : request.further ? ttssh("DLG_AUTH_BANNER2_FURTHER", "追加の認証が必要です.")
                     : ttssh("DLG_AUTH_BANNER2", "認証が必要です."))
                    .foregroundStyle(request.failed ? .red : .secondary)
            }
            if let banner = request.banner, !banner.isEmpty {
                ScrollView {
                    Text(banner).font(.system(.callout, design: .monospaced)).frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 80)
            }

            Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 10) {
                GridRow {
                    Text(ttssh("DLG_AUTH_USERNAME", "ユーザ名:")).gridColumnAlignment(.trailing)
                    TextField("", text: $user)
                }
                GridRow {
                    Text(ttssh("DLG_AUTH_PASSWORD", "パスフレーズ:"))
                    HStack {
                        if showSecret {
                            TextField("", text: $secret).onSubmit(ok)
                        } else {
                            SecureField("", text: $secret).onSubmit(ok)
                        }
                        Toggle(ttssh("DLG_AUTH_SHOW_PASSPHRASE", "パスフレーズを表示"), isOn: $showSecret)
                            .fixedSize()
                    }
                    .disabled(method == .keyboard || method == .agent)
                }
                GridRow {
                    Color.clear.frame(width: 1, height: 1)
                    Toggle(ttssh("DLG_AUTH_REMEMBER_PASSWORD", "パスワードをメモリ上に記憶する"), isOn: $remember)
                        .disabled(method != .password)
                }
            }

            GroupBox(ttssh("DLG_AUTH_METHOD", "認証方式")) {
                VStack(alignment: .leading, spacing: 8) {
                    Picker("", selection: $method) {
                        Text(ttssh("DLG_AUTH_METHOD_PASSWORD", "プレインパスワードを使う")).tag(Method.password)
                            .disabled(!allowed(.password))
                        Text(ttssh("DLG_AUTH_METHOD_RSA", "RSA/DSA/ECDSA/ED25519鍵を使う")).tag(Method.publicKey)
                            .disabled(!allowed(.publicKey))
                        Text(ttssh("DLG_AUTH_METHOD_CHALLENGE2", "キーボードインタラクティブ認証を使う")).tag(Method.keyboard)
                            .disabled(!allowed(.keyboard))
                        // Windows の Pageant に当たるのは ssh-agent
                        Text(ttssh("DLG_AUTH_METHOD_PAGEANT", "Pageantを使う").replacingOccurrences(of: "Pageant", with: "ssh-agent"))
                            .tag(Method.agent)
                            .disabled(!allowed(.agent))
                    }
                    .pickerStyle(.radioGroup)
                    .labelsHidden()
                    HStack {
                        Text(ttssh("DLG_AUTH_PRIVATEKEY", "秘密鍵:"))
                        TextField("", text: $keyPath)
                        Button("…", action: chooseKey)
                    }
                    .disabled(method != .publicKey)
                    .padding(.leading, 18)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(4)
            }

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { done(.cancel) }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK"), action: ok)
                    .keyboardShortcut(.defaultAction)
                    .disabled(user.trimmingCharacters(in: .whitespaces).isEmpty || (method == .publicKey && keyPath.isEmpty))
            }
        }
        .onAppear(perform: load)
    }

    private func load() {
        user = request.user
        keyPath = request.defaultKey ?? ""
        // サーバーが受け付ける方式のうち、TTSSH と同じ順で最初のものを選ぶ
        method = [Method.password, .keyboard, .publicKey, .agent].first(where: allowed) ?? .password
    }

    private func chooseKey() {
        let panel = NSOpenPanel()
        panel.showsHiddenFiles = true
        panel.canChooseDirectories = false
        panel.directoryURL = URL(fileURLWithPath: keyPath.isEmpty ? NSHomeDirectory() + "/.ssh" : keyPath)
            .deletingLastPathComponent()
        if panel.runModal() == .OK, let url = panel.url { keyPath = url.path }
    }

    private func ok() {
        let u = user.trimmingCharacters(in: .whitespaces)
        guard !u.isEmpty else { return }
        switch method {
        case .password:
            let key = PasswordMemory.key(user: u, host: request.host, port: request.port)
            PasswordMemory.set(key, remember ? secret : nil)
            done(.password(user: u, password: secret))
        case .publicKey:
            done(.publicKey(user: u, keyPath: keyPath, passphrase: secret.isEmpty ? nil : secret))
        case .keyboard:
            done(.keyboardInteractive(user: u))
        case .agent:
            done(.agent(user: u))
        }
    }
}

// MARK: - キーボードインタラクティブ（SSH 認証チャレンジ）

struct KeyboardInteractiveView: View {
    let host: String
    let name: String
    let instructions: String
    let prompts: [KeyboardPrompt]
    let done: ([String]?) -> Void
    @State private var answers: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(ttssh("DLG_TIS_TITLE", "SSH 認証チャレンジ")).font(.headline)
            Text(fill(ttssh("DLG_TIS_BANNER", "ログイン中: %s"), host)).foregroundStyle(.secondary)
            if !name.isEmpty { Text(name).fontWeight(.medium) }
            if !instructions.isEmpty {
                Text(instructions).fixedSize(horizontal: false, vertical: true)
            }
            ForEach(prompts.indices, id: \.self) { i in
                VStack(alignment: .leading, spacing: 4) {
                    Text(prompts[i].text)
                    if prompts[i].echo {
                        TextField("", text: binding(i)).onSubmit(ok)
                    } else {
                        SecureField("", text: binding(i)).onSubmit(ok)
                    }
                }
            }
            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { done(nil) }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK"), action: ok)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .onAppear { answers = Array(repeating: "", count: prompts.count) }
    }

    private func binding(_ i: Int) -> Binding<String> {
        Binding(get: { i < answers.count ? answers[i] : "" },
                set: { if i < answers.count { answers[i] = $0 } })
    }

    private func ok() { done(answers) }
}

// MARK: - パスフレーズ

struct PassphraseView: View {
    let keyPath: String
    let failed: Bool
    let done: (String?) -> Void
    @State private var passphrase = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(ttssh("DLG_AUTH_TITLE", "SSH認証")).font(.headline)
            Text(L("鍵ファイルのパスフレーズを入力してください。"))
            Text(keyPath).font(.system(.callout, design: .monospaced)).foregroundStyle(.secondary)
                .textSelection(.enabled)
            if failed {
                Text(ttssh("DLG_AUTH_BANNER2_FAILED", "認証に失敗しました. 再試行してください.")).foregroundStyle(.red)
            }
            HStack {
                Text(ttssh("DLG_AUTH_PASSWORD", "パスフレーズ:"))
                SecureField("", text: $passphrase).onSubmit { done(passphrase) }
            }
            HStack {
                Text(L("キャンセルすると、この鍵は使わずに次の方法を試します。")).font(.callout).foregroundStyle(.secondary)
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { done(nil) }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) { done(passphrase) }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }
}
