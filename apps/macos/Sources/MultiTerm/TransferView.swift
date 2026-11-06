import AppKit
import MTCore
import SwiftUI

/// 転送の種類（メニューと進み具合の表示用）。
enum TransferKind: String, CaseIterable, Identifiable {
    case xmodem = "XMODEM"
    case ymodem = "YMODEM"
    case zmodem = "ZMODEM"
    case kermit = "Kermit"

    var id: String { rawValue }

    var core: TransferProtocol {
        switch self {
        case .xmodem: return .xModem(crc: true, oneK: false)
        case .ymodem: return .yModem
        case .zmodem: return .zModem
        case .kermit: return .kermit
        }
    }

    /// 1 回に複数のファイルを送れるか（XMODEM は 1 つだけ）
    var batch: Bool { self != .xmodem }
}

/// 進行中（か、終わったばかり）の転送。端末上の転送（XMODEM など）と SSH SCP で共通。
struct ActiveTransfer {
    /// ダイアログの見出し
    let title: String
    let send: Bool
    let poll: () -> TransferProgress
    let cancel: () -> Void
    let started = Date()
    /// 終わった時刻（経過時間をそこで止める）
    var finished: Date?
    var progress: TransferProgress

    init(title: String, send: Bool, poll: @escaping () -> TransferProgress, cancel: @escaping () -> Void) {
        self.title = title
        self.send = send
        self.poll = poll
        self.cancel = cancel
        progress = poll()
    }
}

extension TerminalController {
    /// 「転送 > ○○ > 送信...」。ファイルを選んで送る。
    func beginSend(_ kind: TransferKind) {
        guard isConnected, view.session?.isTransferring() == false else { return }
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = kind.batch
        panel.canChooseDirectories = false
        panel.message = "\(kind.rawValue) で送るファイルを選んでください"
        guard panel.runModal() == .OK, !panel.urls.isEmpty else { return }
        startTransfer(kind, send: true, files: panel.urls.map(\.path), destination: nil)
    }

    /// 「転送 > ○○ > 受信...」。保存先を選んで受け取る。
    func beginReceive(_ kind: TransferKind) {
        guard isConnected, view.session?.isTransferring() == false else { return }
        if kind == .xmodem {
            // XMODEM はファイル名が送られてこないので、保存するファイルを選ぶ
            let panel = NSSavePanel()
            panel.directoryURL = Self.downloads
            panel.message = "XMODEM で受け取るファイルの保存先"
            guard panel.runModal() == .OK, let url = panel.url else { return }
            startTransfer(kind, send: false, files: [], destination: url.path)
        } else {
            let panel = NSOpenPanel()
            panel.canChooseFiles = false
            panel.canChooseDirectories = true
            panel.canCreateDirectories = true
            panel.directoryURL = Self.downloads
            panel.prompt = "ここに保存"
            panel.message = "\(kind.rawValue) で受け取るファイルの保存先フォルダ"
            guard panel.runModal() == .OK, let url = panel.url else { return }
            startTransfer(kind, send: false, files: [], destination: url.path)
        }
    }

    static var downloads: URL {
        FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory())
    }

    /// 相手で sz / rz が動いたとき（Tera Term の ZMODEM 自動切り替え）。
    func handleTransferRequest(_ request: TransferRequest) {
        // マクロの実行中は、マクロが転送を始める（zmodemsend など）ので自動では動かない
        guard sheet == nil, macro == nil, view.session?.isTransferring() == false else { return }
        switch request {
        case .zmodemReceive:
            // Tera Term と同じく、尋ねずにダウンロードフォルダへ受け取る
            startTransfer(.zmodem, send: false, files: [], destination: Self.downloads.path)
        case .zmodemSend:
            beginSend(.zmodem)
        }
    }

    func startTransfer(_ kind: TransferKind, send: Bool, files: [String], destination: String?) {
        guard let session = view.session else { return }
        do {
            let handle = try session.startTransfer(protocol: kind.core, send: send, files: files, destination: destination)
            let title = "\(send ? TT.text("DLG_FILETRANS_TITLE", "ファイル送信") : "ファイル受信")（\(kind.rawValue)）"
            show(ActiveTransfer(
                title: title, send: send,
                poll: { handle.progress() },
                cancel: { [weak session] in session?.cancelTransfer() }
            ))
        } catch {
            errorMessage = Self.describe(error)
        }
    }

    /// 進み具合のダイアログを出し、終わるまで見張る。
    func show(_ t: ActiveTransfer) {
        transfer = t
        sheet = .transfer
        pollTransfer()
    }

    /// 終わるまで 0.2 秒ごとに進み具合を取りに行く。
    private func pollTransfer() {
        guard var t = transfer else { return }
        t.progress = t.poll()
        if t.progress.status != .running, t.finished == nil { t.finished = Date() }
        transfer = t
        if t.progress.status == .running {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in self?.pollTransfer() }
        }
    }

    func cancelTransfer() {
        transfer?.cancel()
    }

    func closeTransfer() {
        if transfer?.progress.status == .running { cancelTransfer() }
        transfer = nil
        sheet = nil
    }
}

/// 転送の進み具合（Tera Term の「ファイル送信」ダイアログ）。
struct TransferView: View {
    @ObservedObject var controller: TerminalController

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let t = controller.transfer {
                Text(t.title).font(.headline)
                Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
                    GridRow {
                        Text(TT.text("DLG_FILETRANS_FILENAME", "ファイル名:")).gridColumnAlignment(.trailing)
                        Text(t.progress.file ?? "—").lineLimit(1).truncationMode(.middle)
                    }
                    GridRow {
                        Text(TT.text("DLG_FILETRANS_TRNAS", "転送済容量:"))
                        Text(sizeText(t.progress)).monospacedDigit()
                    }
                    GridRow {
                        Text(TT.text("DLG_FILETRANS_ELAPSED", "経過時間:"))
                        TimelineView(.periodic(from: t.started, by: 1)) { context in
                            Text(elapsed(t.started, until: t.finished ?? context.date)).monospacedDigit()
                        }
                    }
                    if t.progress.filesDone > 0 {
                        GridRow {
                            Text("完了:")
                            Text("\(t.progress.filesDone) ファイル")
                        }
                    }
                }
                if t.progress.status == .done {
                    ProgressView(value: 1)
                } else if let total = t.progress.total, total > 0 {
                    ProgressView(value: Double(min(t.progress.bytes, total)), total: Double(total))
                } else if t.progress.status == .running {
                    ProgressView().progressViewStyle(.linear)
                }
                statusLine(t)
                HStack {
                    if !t.progress.saved.isEmpty {
                        Button("Finder で表示") {
                            NSWorkspace.shared.activateFileViewerSelecting(t.progress.saved.map { URL(fileURLWithPath: $0) })
                        }
                    }
                    Spacer()
                    if t.progress.status == .running {
                        Button(TT.text("BTN_CANCEL", "キャンセル")) { controller.cancelTransfer() }
                            .keyboardShortcut(.cancelAction)
                    } else {
                        Button(TT.text("DLG_FILETRANS_CLOSE", "閉じる")) { controller.closeTransfer() }
                            .keyboardShortcut(.defaultAction)
                    }
                }
            }
        }
        .padding(20)
        .frame(width: 460)
    }

    @ViewBuilder
    private func statusLine(_ t: ActiveTransfer) -> some View {
        switch t.progress.status {
        case .running: Text(t.send ? "送信中…" : "受信中…").foregroundStyle(.secondary)
        case .done: Text("完了しました").foregroundStyle(.green)
        case .canceled: Text("中止しました").foregroundStyle(.orange)
        case .failed: Text("失敗しました：\(t.progress.message ?? "")").foregroundStyle(.red)
        }
    }

    private func sizeText(_ p: TransferProgress) -> String {
        let f = ByteCountFormatter()
        f.countStyle = .file
        let done = f.string(fromByteCount: Int64(p.bytes))
        guard let total = p.total else { return done }
        return "\(done) / \(f.string(fromByteCount: Int64(total)))"
    }

    private func elapsed(_ start: Date, until end: Date) -> String {
        let s = max(0, Int(end.timeIntervalSince(start)))
        return String(format: "%d:%02d", s / 60, s % 60)
    }
}
