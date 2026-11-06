import AppKit
import MTCore
import SwiftUI
import UniformTypeIdentifiers

extension TerminalController {
    /// 内蔵の SSH で接続していて、SSH SCP が使えるか
    var canScp: Bool { isConnected && view.session?.canScp() == true }

    /// ローカルのファイルをリモートのフォルダ（空ならホーム）へ送る。
    func scpUpload(_ files: [String], to remoteDir: String) {
        guard let session = view.session, !files.isEmpty else { return }
        do {
            let handle = try session.scpUpload(files: files, remoteDir: remoteDir)
            show(ActiveTransfer(
                title: TT.text("DLG_SCP_PROGRESS_TITLE_SENDFILE", "SCP ファイル送信中", section: "TTSSH"),
                send: true, showsRetries: false, poll: { handle.progress() }, cancel: { handle.cancel() }
            ))
        } catch {
            errorMessage = Self.describe(error)
        }
    }

    /// リモートのファイルをローカルのフォルダへ受け取る。
    func scpDownload(_ remotePath: String, to localDir: String) {
        guard let session = view.session else { return }
        do {
            let handle = try session.scpDownload(remotePath: remotePath, localDir: localDir)
            show(ActiveTransfer(
                title: TT.text("DLG_SCP_PROGRESS_TITLE_RECEIVEFILE", "SCP ファイル受信中", section: "TTSSH"),
                send: false, showsRetries: false, poll: { handle.progress() }, cancel: { handle.cancel() }
            ))
        } catch {
            errorMessage = Self.describe(error)
        }
    }
}

/// TTSSH の「SSH SCP」ダイアログ。上が送信、下が受信。
struct ScpView: View {
    @ObservedObject var controller: TerminalController

    @State private var sendFiles: [String] = []
    @AppStorage("scp.sendTo") private var sendTo = ""
    @AppStorage("scp.receiveFrom") private var receiveFrom = ""
    @AppStorage("scp.receiveTo") private var receiveTo = TerminalController.downloads.path
    @State private var dropTargeted = false

    private func ssh(_ key: String, _ fallback: String) -> String {
        TT.text(key, fallback, section: "TTSSH")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(ssh("DLG_SCP_TITLE", "Secure Copy Protocol (SCP)")).font(.headline)

            GroupBox {
                Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 8) {
                    GridRow {
                        Text(ssh("DLG_SCP_SENDFILE_FROM", "From:")).gridColumnAlignment(.trailing)
                        TextField("", text: Binding(
                            get: { sendFiles.joined(separator: ", ") },
                            set: { sendFiles = $0.isEmpty ? [] : [$0] }
                        ))
                        .truncationMode(.middle)
                        Button("...") { chooseFiles() }
                    }
                    GridRow {
                        Text(ssh("DLG_SCP_SENDFILE_TO", "To:"))
                        TextField(TT.text("DLG_DANDD_SCP_DEST_NOTE", "空のときはホームディレクトリに送信されます"), text: $sendTo)
                        Button(ssh("DLG_SCP_SENDFILE_SEND", "送信")) {
                            let files = sendFiles, dest = sendTo
                            controller.sheet = nil
                            DispatchQueue.main.async { controller.scpUpload(files, to: dest) }
                        }
                        .disabled(sendFiles.isEmpty)
                    }
                }
                Text(ssh("DLG_SCP_SENDFILE_DRAG", "このウィンドウにファイルをドラッグできます"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(Color.accentColor, lineWidth: dropTargeted ? 2 : 0))
            .onDrop(of: [.fileURL], isTargeted: $dropTargeted) { providers in
                drop(providers)
                return true
            }

            GroupBox {
                Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 8) {
                    GridRow {
                        Text(ssh("DLG_SCP_RECEIVEFILE_FROM", "From:")).gridColumnAlignment(.trailing)
                        TextField("", text: $receiveFrom)
                        Color.clear.frame(width: 1, height: 1)
                    }
                    GridRow {
                        Text(ssh("DLG_SCP_RECEIVEFILE_TO", "To:"))
                        TextField("", text: $receiveTo).truncationMode(.middle)
                        Button("...") { chooseFolder() }
                    }
                }
                HStack {
                    Spacer()
                    Button(ssh("DLG_SCP_RECEIVEFILE_RECEIVE", "受信")) {
                        let from = receiveFrom, to = receiveTo
                        controller.sheet = nil
                        DispatchQueue.main.async { controller.scpDownload(from, to: to) }
                    }
                    .disabled(receiveFrom.trimmingCharacters(in: .whitespaces).isEmpty || receiveTo.isEmpty)
                }
            }

            HStack {
                Spacer()
                Button(ssh("DLG_SCP_SENDFILE_CANCEL", "キャンセル")) { controller.sheet = nil }
                    .keyboardShortcut(.cancelAction)
            }
        }
        .padding(20)
        .frame(width: 520)
    }

    private func chooseFiles() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        // フォルダは中身ごと送る
        panel.canChooseDirectories = true
        panel.message = ssh("DLG_SCP_SELECT_FILE_TITLE", "送信ファイル選択")
        if panel.runModal() == .OK { sendFiles = panel.urls.map(\.path) }
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.directoryURL = URL(fileURLWithPath: receiveTo)
        panel.message = ssh("DLG_SCP_SELECT_DEST_TITLE", "受信フォルダ選択")
        if panel.runModal() == .OK, let url = panel.url { receiveTo = url.path }
    }

    private func drop(_ providers: [NSItemProvider]) {
        for provider in providers {
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                guard let url, url.isFileURL else { return }
                DispatchQueue.main.async {
                    if !sendFiles.contains(url.path) { sendFiles.append(url.path) }
                }
            }
        }
    }
}
