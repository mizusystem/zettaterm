import ZettaCore
import SwiftUI

/// 「ファイル > ログ記録中ダイアログを表示」：記録中のログのファイル名・大きさと、一時停止・終了のボタン。
/// ウィンドウの左下に出し、1 秒ごとに大きさを取り直す。
struct LogPanel: View {
    @ObservedObject var controller: TerminalController

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            if let status = controller.view.session?.logStatus() {
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Text(TT.text("FILEDLG_TRANS_TITLE_LOG", "ログ記録中")).font(.caption.bold())
                        Spacer()
                        Button {
                            controller.showLogPanel = false
                        } label: {
                            Image(systemName: "xmark")
                        }
                        .buttonStyle(.borderless)
                        .help(TT.text("DLG_FILETRANS_CLOSE", "閉じる"))
                    }
                    Text(TT.text("DLG_FILETRANS_FILENAME", "ファイル名:") + " "
                         + URL(fileURLWithPath: status.path).lastPathComponent)
                        .font(.caption).lineLimit(1).truncationMode(.middle)
                        .help(status.path)
                    Text(TT.text("DLG_FILETRANS_TRNAS", "転送済容量:") + " "
                         + ByteCountFormatter.string(fromByteCount: Int64(status.bytes), countStyle: .file))
                        .font(.caption).monospacedDigit()
                    HStack {
                        Button(status.paused ? TT.text("DLG_FILETRANS_START", "再開")
                                             : TT.text("DLG_FILETRANS_PAUSE", "一時停止")) {
                            controller.toggleLogPause()
                        }
                        Button(TT.text("MENU_FILE_STOPLOG", "ログを終了")) {
                            controller.stopLog()
                            controller.showLogPanel = false
                        }
                    }
                    .controlSize(.small)
                }
                .padding(10)
                .frame(width: 240, alignment: .leading)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                .padding(8)
            }
        }
    }
}
