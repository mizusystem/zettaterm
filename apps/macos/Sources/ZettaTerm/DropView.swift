import AppKit
import ZettaCore
import SwiftUI

extension TerminalController {
    /// 端末にファイルがドロップされた。送り方を尋ねる（Tera Term の「ファイル ドラッグ&ドロップ」）。
    func handleDrop(_ urls: [URL]) {
        guard sheet == nil, !urls.isEmpty else { return }
        // 未接続でも .ttl ならマクロとして動かせる
        guard isConnected || (urls.count == 1 && urls[0].pathExtension.lowercased() == "ttl") else { return }
        dropped = urls
        sheet = .drop
    }
}

/// 「ファイル ドラッグ&ドロップ」。SCP / ファイル送信 / ファイル名の貼り付けに加え、
/// この実装では XMODEM などのプロトコル転送と、.ttl のマクロ実行も選べる。
struct DropView: View {
    enum Action: Hashable { case scp, sendFile, transfer, paste, macro }

    @ObservedObject var controller: TerminalController
    @State private var action: Action = .sendFile
    @AppStorage("drop.scpDest") private var scpDest = ""
    @AppStorage("drop.binary") private var binary = true
    @AppStorage("drop.kind") private var kindRaw = TransferKind.xmodem.rawValue
    @AppStorage("drop.escape") private var escape = true
    @AppStorage("drop.newline") private var newline = false

    private var files: [URL] { controller.dropped }
    private var isMacro: Bool { files.count == 1 && files[0].pathExtension.lowercased() == "ttl" }
    private var kind: TransferKind { TransferKind(rawValue: kindRaw) ?? .xmodem }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(TT.text("DLG_DANDD_TITLE", "ファイル ドラッグ&ドロップ")).font(.headline)
            Text(files.map(\.lastPathComponent).joined(separator: ", "))
                .lineLimit(2).truncationMode(.middle).foregroundStyle(.secondary)
            Text(TT.text("DLG_DANDD_TEXT", "ファイル転送を行いますか？"))

            Picker("", selection: $action) {
                if isMacro { Text(L("マクロとして実行")).tag(Action.macro) }
                Text(TT.text("DLG_DANDD_SCP_RADIO", "SCP")).tag(Action.scp).disabled(!controller.canScp)
                Text(TT.text("DLG_DANDD_SENDFILE_RADIO", "ファイル送信（ファイルの内容を貼り付け）")).tag(Action.sendFile)
                Text(L("プロトコルで送る")).tag(Action.transfer)
                Text(TT.text("DLG_DANDD_PASTE_RADIO", "ファイル名を貼り付け")).tag(Action.paste)
            }
            .pickerStyle(.radioGroup)
            .labelsHidden()
            .disabled(!controller.isConnected)

            Group {
                switch action {
                case .scp:
                    HStack {
                        Text(TT.text("DLG_DANDD_SCP_DEST_LABEL", "送信先:"))
                        TextField(TT.text("DLG_DANDD_SCP_DEST_NOTE", "空のときはホームディレクトリに送信されます"), text: $scpDest)
                    }
                case .sendFile:
                    Toggle(TT.text("DLG_DANDD_SEND_BINARY_CHECK", "バイナリ"), isOn: $binary)
                case .transfer:
                    Picker("", selection: $kindRaw) {
                        ForEach(TransferKind.allCases) { Text($0.rawValue).tag($0.rawValue) }
                    }
                    .labelsHidden()
                    .frame(width: 160)
                case .paste:
                    HStack(spacing: 16) {
                        Toggle(TT.text("DLG_DANDD_PASTE_ESCAPE_CHECK", "エスケープ"), isOn: $escape)
                        Picker("", selection: $newline) {
                            Text(TT.text("DLG_DANDD_PASTE_SPACE_RADIO", "スペースで区切る")).tag(false)
                            Text(TT.text("DLG_DANDD_PASTE_NEWLINE_RADIO", "改行で区切る")).tag(true)
                        }
                        .pickerStyle(.radioGroup)
                        .labelsHidden()
                    }
                case .macro:
                    EmptyView()
                }
            }
            .padding(.leading, 18)

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { close() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK"), action: run)
                    .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 480)
        .onAppear {
            action = isMacro ? .macro : .sendFile
        }
    }

    private func close() {
        controller.dropped = []
        controller.sheet = nil
    }

    private func run() {
        let files = self.files
        let action = self.action
        close()
        // シートを閉じてから次のシートやパネルを出す
        DispatchQueue.main.async { [controller, scpDest, binary, kind, escape, newline] in
            switch action {
            case .macro:
                controller.runMacro(files[0])
            case .scp:
                controller.scpUpload(files.map(\.path), to: scpDest)
            case .sendFile:
                guard let session = controller.view.session else { return }
                for f in files {
                    do { try session.sendFile(path: f.path, binary: binary) } catch {
                        controller.errorMessage = TerminalController.describe(error)
                        return
                    }
                }
            case .transfer:
                let sendFiles = kind.batch ? files.map(\.path) : [files[0].path]
                let proto = kind == .xmodem
                    ? TerminalController.xmodemProtocol(
                        send: true, index: UserDefaults.standard.object(forKey: "xmodem.sendOption") as? Int ?? 1)
                    : nil
                controller.startTransfer(kind, send: true, files: sendFiles, destination: nil, protocol: proto)
            case .paste:
                let names = files.map { escape ? Self.escaped($0.path) : $0.path }
                controller.view.session?.paste(text: names.joined(separator: newline ? "\n" : " "))
            }
        }
    }

    /// シェルで使える形（空白や記号の前に \）
    static func escaped(_ path: String) -> String {
        let special = Set(" \t\\'\"()[]{}&;|<>*?!$`#~")
        return String(path.flatMap { special.contains($0) ? ["\\", $0] : [$0] })
    }
}
