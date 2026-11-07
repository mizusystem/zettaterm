import MTCore
import SwiftUI

/// TTSSH の「SSH」（キープアライブ）と「SSH 転送」（ポート転送）。内蔵 SSH の接続すべてに効く。
enum SshSettings {
    private static let forwardsKey = "ssh.forwards"
    private static let keepaliveKey = "ssh.keepalive"

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
        setSshDefaults(forwards: forwards, keepaliveSecs: UInt32(clamping: keepalive))
    }
}

/// 「設定 > SSH...」：キープアライブとポート転送。
struct SshSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var keepalive = String(SshSettings.keepalive)
    @State private var forwards = SshSettings.forwards.joined(separator: "\n")
    @State private var bad: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_SSHSETUP_TITLE", "設定", section: "TTSSH")).font(.headline)

            HStack {
                Text(TT.text("DLG_SSHSETUP_HEARTBEAT", "Keepalive", section: "TTSSH"))
                TextField("", text: $keepalive).frame(width: 60)
                Text(TT.text("DLG_SSHSETUP_HEARTBEAT_UNIT", "秒(0を指定すると無効)", section: "TTSSH"))
                    .foregroundStyle(.secondary)
            }

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
        let rejected = setSshDefaults(forwards: lines, keepaliveSecs: UInt32(clamping: Int(keepalive) ?? 60))
        if !rejected.isEmpty {
            // 読めない行があれば保存せず、前の設定に戻して直してもらう
            bad = rejected
            SshSettings.apply()
            return
        }
        SshSettings.forwards = lines
        SshSettings.keepalive = Int(keepalive) ?? 60
        SshSettings.apply()
        dismiss()
    }
}
