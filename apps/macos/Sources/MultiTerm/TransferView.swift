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
    /// リトライ数を出すか（SCP は TCP の上なので出さない）
    let showsRetries: Bool
    let started = Date()
    /// 終わった時刻（経過時間をそこで止める）
    var finished: Date?
    var progress: TransferProgress

    init(
        title: String, send: Bool, showsRetries: Bool = true,
        poll: @escaping () -> TransferProgress, cancel: @escaping () -> Void
    ) {
        self.title = title
        self.send = send
        self.showsRetries = showsRetries
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
        panel.message = L("%@ で送るファイルを選んでください", kind.rawValue)
        // XMODEM はブロックの大きさ（128 / 1K）とチェックサム / CRC を選べる（Tera Term の「オプション」）
        let popup = kind == .xmodem ? Self.xmodemOptions(send: true) : nil
        if let popup { panel.accessoryView = Self.accessory(popup); panel.isAccessoryViewDisclosed = true }
        guard panel.runModal() == .OK, !panel.urls.isEmpty else {
            // 相手の rz に応じなかった。とっておいた受信を画面に出す
            view.session?.declineTransfer()
            return
        }
        let proto = popup.map { Self.xmodemProtocol(send: true, index: $0.indexOfSelectedItem) }
        startTransfer(kind, send: true, files: panel.urls.map(\.path), destination: nil, protocol: proto)
    }

    /// 「転送 > ○○ > 受信...」。保存先を選んで受け取る。
    func beginReceive(_ kind: TransferKind) {
        guard isConnected, view.session?.isTransferring() == false else { return }
        if kind == .xmodem {
            // XMODEM はファイル名が送られてこないので、保存するファイルを選ぶ
            let panel = NSSavePanel()
            panel.directoryURL = Self.downloads
            panel.message = L("XMODEM で受け取るファイルの保存先")
            let popup = Self.xmodemOptions(send: false)
            panel.accessoryView = Self.accessory(popup)
            guard panel.runModal() == .OK, let url = panel.url else { return }
            let proto = Self.xmodemProtocol(send: false, index: popup.indexOfSelectedItem)
            startTransfer(kind, send: false, files: [], destination: url.path, protocol: proto)
        } else {
            let panel = NSOpenPanel()
            panel.canChooseFiles = false
            panel.canChooseDirectories = true
            panel.canCreateDirectories = true
            panel.directoryURL = Self.downloads
            panel.prompt = L("ここに保存")
            panel.message = L("%@ で受け取るファイルの保存先フォルダ", kind.rawValue)
            guard panel.runModal() == .OK, let url = panel.url else { return }
            startTransfer(kind, send: false, files: [], destination: url.path)
        }
    }

    /// 「転送 > Kermit > 取得」：サーバーになっている相手から、名前を指定して受け取る（ダウンロードフォルダへ）。
    func beginKermitGet() {
        guard isConnected, let session = view.session, !session.isTransferring() else { return }
        let alert = NSAlert()
        alert.messageText = TT.text("DLG_GETFN_TITLE", "Tera Term: Kermit 取得")
        alert.informativeText = TT.text("DLG_GETFN_FILENAME", "ファイル名(&F):")
        let field = NSTextField(string: "")
        field.frame = NSRect(x: 0, y: 0, width: 280, height: 22)
        alert.accessoryView = field
        alert.addButton(withTitle: TT.text("BTN_OK", "OK"))
        alert.addButton(withTitle: TT.text("BTN_CANCEL", "キャンセル"))
        alert.window.initialFirstResponder = field
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        let names = field.stringValue.trimmingCharacters(in: .whitespaces)
        guard !names.isEmpty else { return }
        do {
            let handle = try session.startKermitGet(names: names, destination: Self.downloads.path)
            show(ActiveTransfer(
                title: "\(L("ファイル受信"))（Kermit）", send: false,
                poll: { handle.progress() },
                cancel: { [weak session] in session?.cancelTransfer() }
            ))
        } catch {
            errorMessage = Self.describe(error)
        }
    }

    /// 「転送 > Kermit > 終了」：サーバーになっている相手を終わらせる。
    func kermitFinish() {
        guard isConnected, let session = view.session, !session.isTransferring() else { return }
        do {
            _ = try session.startKermitFinish()
        } catch {
            errorMessage = Self.describe(error)
        }
    }

    /// 「ファイル > ファイル受信」：プロトコルなしで受信をそのままファイルへ（機器の出力を丸ごと取るとき）。
    /// データが来ないまま指定の秒数が過ぎたら止まる（0 なら止めるまで）。
    func beginRawReceive() {
        guard isConnected, let session = view.session, !session.isTransferring() else { return }
        let panel = NSSavePanel()
        panel.directoryURL = Self.downloads
        panel.nameFieldStringValue = "capture.bin"
        panel.message = L("受信をそのまま保存するファイル")
        let key = "rawReceive.idleSecs"
        let field = NSTextField(string: String(UserDefaults.standard.integer(forKey: key)))
        field.frame = NSRect(x: 0, y: 0, width: 60, height: 22)
        let label = NSTextField(labelWithString: L("この秒数データが来なければ止める（0 は止めない）:"))
        let stack = NSStackView(views: [label, field])
        stack.orientation = .horizontal
        stack.edgeInsets = NSEdgeInsets(top: 8, left: 8, bottom: 8, right: 8)
        panel.accessoryView = stack
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let secs = max(0, Int(field.stringValue) ?? 0)
        UserDefaults.standard.set(secs, forKey: key)
        do {
            let handle = try session.startTransfer(
                protocol: .raw(idleMs: UInt32(clamping: secs * 1000)), send: false, files: [], destination: url.path)
            show(ActiveTransfer(
                title: TT.text("MENU_FILE_RECVFILE", "ファイル受信").replacingOccurrences(of: "...", with: ""),
                send: false,
                poll: { handle.progress() },
                cancel: { [weak session] in session?.cancelTransfer() }
            ))
        } catch {
            errorMessage = Self.describe(error)
        }
    }

    // MARK: XMODEM のオプション

    /// 送信はチェックサム / CRC / 1K、受信はチェックサム / CRC。前回の選択を覚えておく。
    private static func xmodemOptions(send: Bool) -> NSPopUpButton {
        let popup = NSPopUpButton(frame: .zero, pullsDown: false)
        var titles = [TT.text("DLG_XOPT_CHECKSUM", "チェックサム"), TT.text("DLG_XOPT_CRC", "CRC")]
        if send { titles.append(TT.text("DLG_XOPT_1K", "1K")) }
        popup.addItems(withTitles: titles)
        let key = send ? "xmodem.sendOption" : "xmodem.receiveOption"
        let saved = UserDefaults.standard.object(forKey: key) as? Int ?? 1
        popup.selectItem(at: min(saved, titles.count - 1))
        return popup
    }

    static func xmodemProtocol(send: Bool, index: Int) -> TransferProtocol {
        UserDefaults.standard.set(index, forKey: send ? "xmodem.sendOption" : "xmodem.receiveOption")
        switch index {
        case 0: return .xModem(crc: false, oneK: false)
        case 2: return .xModem(crc: true, oneK: true)
        default: return .xModem(crc: true, oneK: false)
        }
    }

    private static func accessory(_ popup: NSPopUpButton) -> NSView {
        let label = NSTextField(labelWithString: TT.text("DLG_XOPT", "オプション") + ":")
        let stack = NSStackView(views: [label, popup])
        stack.orientation = .horizontal
        stack.edgeInsets = NSEdgeInsets(top: 8, left: 8, bottom: 8, right: 8)
        return stack
    }

    /// XON/XOFF のシリアルで XMODEM / YMODEM を始めようとしたら確かめる。
    /// 0x11 / 0x13 がフロー制御として OS に消され、転送が壊れるため（ZMODEM と Kermit は制御文字を逃がすので大丈夫）。
    private func confirmFlowControl(_ kind: TransferKind) -> Bool {
        guard kind == .xmodem || kind == .ymodem, isSerialConnected, serialConfig.flow == .xonXoff else { return true }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = L("フロー制御が XON/XOFF になっています")
        alert.informativeText = L(
            "%@ はデータをそのまま送るので、ファイルの中の 0x11 / 0x13 がフロー制御として扱われ、転送が失敗します。「設定 > シリアルポート」でフロー制御を「なし」か「RTS/CTS」にしてから転送してください。",
            kind.rawValue)
        alert.addButton(withTitle: TT.text("BTN_CANCEL", "キャンセル"))
        alert.addButton(withTitle: L("このまま転送"))
        return alert.runModal() == .alertSecondButtonReturn
    }

    /// 「ファイル > ファイル送信...」。プロトコルを使わず中身をそのまま流す（バイナリなら一切変換しない）。
    func beginSendFile() {
        guard isConnected, let session = view.session else { return }
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.message = TT.text("MENU_FILE_SENDFILE", "ファイル送信")
        let binary = NSButton(checkboxWithTitle: TT.text("DLG_SENDFILE_BINARY", "バイナリ"), target: nil, action: nil)
        binary.state = UserDefaults.standard.bool(forKey: "sendFile.binary") ? .on : .off
        let box = NSStackView(views: [binary])
        box.edgeInsets = NSEdgeInsets(top: 8, left: 8, bottom: 8, right: 8)
        panel.accessoryView = box
        panel.isAccessoryViewDisclosed = true
        guard panel.runModal() == .OK, let url = panel.url else { return }
        UserDefaults.standard.set(binary.state == .on, forKey: "sendFile.binary")
        do {
            try session.sendFile(path: url.path, binary: binary.state == .on)
        } catch {
            errorMessage = Self.describe(error)
        }
    }

    /// 「転送 > ZMODEM の自動受信」（既定はオン）
    static var autoZmodemReceive: Bool {
        UserDefaults.standard.object(forKey: "zmodem.autoReceive") as? Bool ?? true
    }

    /// ファイルを受け取るフォルダ（「ファイル > ディレクトリを変更」で決めたもの。無ければダウンロード）。
    static var downloads: URL {
        if let p = UserDefaults.standard.string(forKey: "transfer.folder"), !p.isEmpty {
            var isDir: ObjCBool = false
            if FileManager.default.fileExists(atPath: p, isDirectory: &isDir), isDir.boolValue {
                return URL(fileURLWithPath: p)
            }
        }
        return FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSHomeDirectory())
    }

    /// 「ファイル > ディレクトリを変更」：受け取るフォルダ（マクロの受信を含む）を選ぶ。
    static func chooseTransferFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.directoryURL = downloads
        panel.message = TT.text("DLG_SELECT_DIR_TITLE", "新しいディレクトリを選択してください")
        if panel.runModal() == .OK, let url = panel.url {
            UserDefaults.standard.set(url.path, forKey: "transfer.folder")
        }
    }

    /// 「設定 > 設定フォルダ」：接続先の履歴などを置いているフォルダを Finder で開く。
    static func openSettingsFolder() {
        let dir = URL(fileURLWithPath: HostStore.historyPath).deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        NSWorkspace.shared.activateFileViewerSelecting([dir])
    }

    /// 相手で sz / rz が動いたとき（Tera Term の ZMODEM 自動切り替え）。
    func handleTransferRequest(_ request: TransferRequest) {
        // マクロの実行中は、マクロが転送を始める（zmodemsend など）ので自動では動かない
        guard sheet == nil, macro == nil, view.session?.isTransferring() == false else {
            view.session?.declineTransfer()
            return
        }
        switch request {
        case .zmodemReceive:
            // 既定は Tera Term と同じく、尋ねずに受け取りのフォルダ（「ディレクトリを変更」）へ受け取る。
            // 「転送 > ZMODEM の自動受信」を切ると、受け取る前に確かめる
            if !Self.autoZmodemReceive {
                let alert = NSAlert()
                alert.messageText = L("相手が ZMODEM でファイルを送ろうとしています")
                alert.informativeText = L("%@ に受け取りますか？", Self.downloads.path)
                alert.addButton(withTitle: L("受け取る"))
                alert.addButton(withTitle: TT.text("BTN_CANCEL", "キャンセル"))
                guard alert.runModal() == .alertFirstButtonReturn else {
                    // 相手の sz を止める（CAN を 8 つと BS を 8 つ。ZMODEM の中止の合図）
                    view.session?.write(bytes: Data(repeating: 0x18, count: 8) + Data(repeating: 0x08, count: 8))
                    view.session?.declineTransfer()
                    return
                }
            }
            startTransfer(.zmodem, send: false, files: [], destination: Self.downloads.path)
        case .zmodemSend:
            beginSend(.zmodem)
        }
    }

    func startTransfer(
        _ kind: TransferKind, send: Bool, files: [String], destination: String?, protocol proto: TransferProtocol? = nil
    ) {
        guard let session = view.session, confirmFlowControl(kind) else {
            view.session?.declineTransfer()
            return
        }
        do {
            let handle = try session.startTransfer(
                protocol: proto ?? kind.core, send: send, files: files, destination: destination)
            let title = "\(send ? TT.text("DLG_FILETRANS_TITLE", "ファイル送信") : L("ファイル受信"))（\(kind.rawValue)）"
            show(ActiveTransfer(
                title: title, send: send,
                poll: { handle.progress() },
                cancel: { [weak session] in session?.cancelTransfer() }
            ))
        } catch {
            session.declineTransfer()
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
                    GridRow {
                        Text(L("速度:"))
                        TimelineView(.periodic(from: t.started, by: 1)) { context in
                            Text(rateText(t, now: t.finished ?? context.date)).monospacedDigit()
                        }
                    }
                    if t.showsRetries {
                        GridRow {
                            Text(L("リトライ:"))
                            Text(L("%@ 回", String(t.progress.retries)))
                                .monospacedDigit()
                                .foregroundStyle(t.progress.retries > 0 ? .orange : .primary)
                        }
                    }
                    if t.progress.filesDone > 0 {
                        GridRow {
                            Text(L("完了:"))
                            Text(L("%@ ファイル", String(t.progress.filesDone)))
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
                        Button(L("Finder で表示")) {
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
        case .running: Text(t.send ? L("送信中…") : L("受信中…")).foregroundStyle(.secondary)
        case .done: Text(L("完了しました")).foregroundStyle(.green)
        case .canceled: Text(L("中止しました")).foregroundStyle(.orange)
        case .failed: Text(L("失敗しました：%@", t.progress.message ?? "")).foregroundStyle(.red)
        }
    }

    private func sizeText(_ p: TransferProgress) -> String {
        let f = ByteCountFormatter()
        f.countStyle = .file
        let done = f.string(fromByteCount: Int64(p.bytes))
        guard let total = p.total else { return done }
        return "\(done) / \(f.string(fromByteCount: Int64(total)))"
    }

    /// 「12.3 KB/s（残り 0:42）」。残りは大きさがわかっていて、転送中のときだけ
    private func rateText(_ t: ActiveTransfer, now: Date) -> String {
        let secs = now.timeIntervalSince(t.started)
        // 転送中は 1 秒たつまで出さない（ぶれるので）。終わったら実際の時間で平均を出す
        guard t.progress.bytes > 0, secs > 0, secs >= 1 || t.finished != nil else { return "—" }
        let rate = Double(t.progress.bytes) / secs
        let f = ByteCountFormatter()
        f.countStyle = .file
        var text = "\(f.string(fromByteCount: Int64(rate)))/s"
        if t.progress.status == .running, let total = t.progress.total, total > t.progress.bytes, rate > 0 {
            let left = Int(Double(total - t.progress.bytes) / rate)
            text += L("（残り %@）", String(format: "%d:%02d", left / 60, left % 60))
        }
        return text
    }

    private func elapsed(_ start: Date, until end: Date) -> String {
        let s = max(0, Int(end.timeIntervalSince(start)))
        return String(format: "%d:%02d", s / 60, s % 60)
    }
}
