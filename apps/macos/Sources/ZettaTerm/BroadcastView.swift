import ZettaCore
import SwiftUI

/// 「コントロール > ブロードキャストコマンド」。つながっている接続を選んで、同じ文字列を送る。
/// リアルタイムにすると、入力欄で打った文字をその場で送る。送ったコマンドは履歴に残せる。
struct BroadcastView: View {
    @ObservedObject var controller: TerminalController
    @ObservedObject private var registry = SessionRegistry.shared
    @Environment(\.dismiss) private var dismiss

    /// Enter キーで送る改行（Tera Term の CRLF / CR / LF。session は送り先の設定）
    enum Newline: String, CaseIterable { case session, crlf, cr, lf }

    @State private var text = ""
    @AppStorage("broadcast.enter") private var enter = true
    @AppStorage("broadcast.newline") private var newline: Newline = .session
    @AppStorage("broadcast.realtime") private var realtime = false
    @AppStorage("broadcast.saveHistory") private var saveHistory = true
    @State private var history: [String] = BroadcastView.savedHistory
    @State private var excluded: Set<UUID> = []
    @State private var sentCount: Int?
    /// リアルタイムで送り終えた入力欄の中身（差分だけを送るため）
    @State private var sentText = ""

    private static let historyKey = "broadcast.history"
    private static var savedHistory: [String] {
        UserDefaults.standard.stringArray(forKey: historyKey) ?? []
    }

    private var targets: [TerminalController] {
        registry.controllers.filter(\.isConnected)
    }

    private var chosen: [TerminalController] {
        targets.filter { !excluded.contains($0.id) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_BROADCAST_TITLE", "コマンドのブロードキャスト")).font(.headline)

            HStack {
                TextField("", text: $text)
                    .onSubmit(submit)
                    .onChange(of: text) { _, new in
                        if realtime { sendRealtime(new) }
                    }
                if saveHistory && !history.isEmpty && !realtime {
                    Menu {
                        ForEach(history, id: \.self) { h in
                            Button(h) { text = h }
                        }
                    } label: {
                        Image(systemName: "clock.arrow.circlepath")
                    }
                    .menuStyle(.borderlessButton)
                    .fixedSize()
                    .help(TT.text("DLG_BROADCAST_HISTORY", "ヒストリ"))
                }
            }

            HStack(spacing: 16) {
                Toggle(TT.text("DLG_BROADCAST_ENTER", "Enterキー"), isOn: $enter)
                Picker("", selection: $newline) {
                    Text(L("送り先の設定")).tag(Newline.session)
                    Text("CRLF").tag(Newline.crlf)
                    Text("CR").tag(Newline.cr)
                    Text("LF").tag(Newline.lf)
                }
                .labelsHidden().fixedSize()
                .disabled(!enter)
                Toggle(TT.text("DLG_BROADCAST_REALTIME", "リアルタイム"), isOn: $realtime)
                    .onChange(of: realtime) { _, _ in
                        text = ""
                        sentText = ""
                    }
                Toggle(TT.text("DLG_BROADCAST_HISTORY", "ヒストリ"), isOn: $saveHistory)
            }

            Text(L("送り先")).foregroundStyle(.secondary)
            if targets.isEmpty {
                Text(L("接続中のセッションがありません。")).foregroundStyle(.secondary)
            } else {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(targets, id: \.id) { c in
                        Toggle(isOn: Binding(
                            get: { !excluded.contains(c.id) },
                            set: { on in if on { excluded.remove(c.id) } else { excluded.insert(c.id) } }
                        )) {
                            Text(c.windowTitle + (c === controller ? L("（このウィンドウ）") : ""))
                        }
                    }
                }
            }

            HStack {
                if realtime {
                    Text(L("打った文字をその場で送ります。")).font(.callout).foregroundStyle(.secondary)
                } else if let sentCount {
                    Text(L("%@ 件に送信しました", String(sentCount))).font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Button(TT.text("DLG_BROADCAST_CLOSE", "閉じる")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("DLG_BROADCAST_SUBMIT", "送信"), action: submit)
                    .keyboardShortcut(.defaultAction)
                    .disabled(chosen.isEmpty || (text.isEmpty && !enter && !realtime))
            }
        }
        .padding(20)
        .frame(width: 560)
    }

    /// Enter（入力欄の確定）か「送信」ボタン。
    private func submit() {
        if realtime {
            // 打った文字は送ってあるので、改行だけ
            for c in chosen { sendNewline(c) }
            sentText = ""
            text = ""
            return
        }
        for c in chosen {
            if !text.isEmpty { c.broadcast(text, enter: false) }
            if enter { sendNewline(c) }
        }
        sentCount = chosen.count
        if saveHistory {
            history = pushBroadcastHistory(history: history, command: text, max: 99)
            UserDefaults.standard.set(history, forKey: Self.historyKey)
        }
        text = ""
    }

    private func sendNewline(_ c: TerminalController) {
        switch newline {
        case .session: c.broadcast("", enter: true)
        case .crlf: c.broadcast("\r\n", enter: false)
        case .cr: c.broadcast("\r", enter: false)
        case .lf: c.broadcast("\n", enter: false)
        }
    }

    /// リアルタイム：前に送った中身との差分（消した文字は BackSpace、足した文字はそのまま）を送る。
    private func sendRealtime(_ new: String) {
        let diff = broadcastRealtimeDiff(old: sentText, new: new)
        sentText = new
        for c in chosen {
            c.broadcastBackspace(Int(diff.erase))
            if !diff.insert.isEmpty { c.broadcast(diff.insert, enter: false) }
        }
    }
}
