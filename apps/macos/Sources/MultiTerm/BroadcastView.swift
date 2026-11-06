import SwiftUI

/// 「ブロードキャストコマンド」。入力した文字列を、選んだ複数のセッションへ一度に送る。
struct BroadcastView: View {
    @ObservedObject var controller: TerminalController
    @ObservedObject private var registry = SessionRegistry.shared
    @Environment(\.dismiss) private var dismiss

    @State private var text = ""
    @AppStorage("broadcast.enter") private var enter = true
    @State private var excluded: Set<UUID> = []
    @State private var sentCount: Int?

    private var targets: [TerminalController] {
        registry.controllers.filter(\.isConnected)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_BROADCAST_TITLE", "コマンドのブロードキャスト")).font(.headline)

            HStack {
                TextField("", text: $text)
                    .onSubmit(send)
                Toggle(TT.text("DLG_BROADCAST_ENTER", "Enterキー"), isOn: $enter)
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
                if let sentCount {
                    Text(L("%@ 件に送信しました", String(sentCount))).font(.callout).foregroundStyle(.secondary)
                }
                Spacer()
                Button(TT.text("DLG_BROADCAST_CLOSE", "閉じる")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("DLG_BROADCAST_SUBMIT", "送信"), action: send)
                    .keyboardShortcut(.defaultAction)
                    .disabled(targets.allSatisfy { excluded.contains($0.id) } || (text.isEmpty && !enter))
            }
        }
        .padding(20)
        .frame(width: 520)
    }

    private func send() {
        let chosen = targets.filter { !excluded.contains($0.id) }
        for c in chosen { c.broadcast(text, enter: enter) }
        sentCount = chosen.count
    }
}
