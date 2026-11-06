import Foundation
import MTCore
import SwiftUI

/// Tera Term の「その他の設定 > コピーと貼り付け」と、クリッカブル URL。全ウィンドウで共通。
struct CopyPasteConfig: Equatable {
    /// マウスで選んだらすぐクリップボードへコピーする（AutoTextCopy）
    var autoCopy = true
    /// 右クリックで貼り付ける（オフならコンテキストメニュー）
    var rightClickPaste = false
    /// 中クリックで貼り付ける
    var middleClickPaste = true
    /// 改行や危険な語を含む貼り付けを確認する（ConfirmChangePaste）
    var confirmPaste = true
    /// 危険な語（ConfirmChangePasteStringFile の中身）。1 行に 1 つ
    var dangerousWords = defaultDangerousWords()
    /// 貼り付け時に末尾の改行を削除する（TrimTrailingNLonPaste）
    var trimTrailingNewline = false
    /// ダブルクリックで単語を区切る文字（DelimList）
    var delimiters = defaultWordDelimiters()
    /// URL に下線を引き、Cmd + クリックで開く（EnableClickableUrl）
    var clickableUrl = true

    private enum Key {
        static let autoCopy = "copyPaste.autoCopy"
        static let rightClickPaste = "copyPaste.rightClickPaste"
        static let middleClickPaste = "copyPaste.middleClickPaste"
        static let confirmPaste = "copyPaste.confirmPaste"
        static let dangerousWords = "copyPaste.dangerousWords"
        static let trimTrailingNewline = "copyPaste.trimTrailingNewline"
        static let delimiters = "copyPaste.delimiters"
        static let clickableUrl = "copyPaste.clickableUrl"
    }

    static var saved: CopyPasteConfig {
        let d = UserDefaults.standard
        var c = CopyPasteConfig()
        if d.object(forKey: Key.autoCopy) != nil { c.autoCopy = d.bool(forKey: Key.autoCopy) }
        if d.object(forKey: Key.rightClickPaste) != nil { c.rightClickPaste = d.bool(forKey: Key.rightClickPaste) }
        if d.object(forKey: Key.middleClickPaste) != nil { c.middleClickPaste = d.bool(forKey: Key.middleClickPaste) }
        if d.object(forKey: Key.confirmPaste) != nil { c.confirmPaste = d.bool(forKey: Key.confirmPaste) }
        if let words = d.stringArray(forKey: Key.dangerousWords) { c.dangerousWords = words }
        if d.object(forKey: Key.trimTrailingNewline) != nil {
            c.trimTrailingNewline = d.bool(forKey: Key.trimTrailingNewline)
        }
        if let delims = d.string(forKey: Key.delimiters) { c.delimiters = delims }
        if d.object(forKey: Key.clickableUrl) != nil { c.clickableUrl = d.bool(forKey: Key.clickableUrl) }
        return c
    }

    func save() {
        let d = UserDefaults.standard
        d.set(autoCopy, forKey: Key.autoCopy)
        d.set(rightClickPaste, forKey: Key.rightClickPaste)
        d.set(middleClickPaste, forKey: Key.middleClickPaste)
        d.set(confirmPaste, forKey: Key.confirmPaste)
        d.set(dangerousWords, forKey: Key.dangerousWords)
        d.set(trimTrailingNewline, forKey: Key.trimTrailingNewline)
        d.set(delimiters, forKey: Key.delimiters)
        d.set(clickableUrl, forKey: Key.clickableUrl)
        NotificationCenter.default.post(name: .copyPasteConfigChanged, object: nil)
    }
}

extension Notification.Name {
    static let copyPasteConfigChanged = Notification.Name("MultiTerm.copyPasteConfigChanged")
}

/// 「設定 > コピーと貼り付け」。
struct CopyPasteSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var config = CopyPasteConfig.saved
    @State private var wordsText = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_TABSHEET_TITLE_COPYPASTE", "コピーと貼り付け")).font(.headline)

            Toggle(TT.text("DLG_TAB_COPYPASTE_AUTO_TEXT_COPY", "マウスで文字選択時、自動でクリップボードへのコピーを行う"),
                   isOn: $config.autoCopy)
            Toggle(L("右クリックで貼り付ける（オフならメニューを出す）"), isOn: $config.rightClickPaste)
            Toggle(L("中クリックで貼り付ける"), isOn: $config.middleClickPaste)
            Toggle(TT.text("DLG_TAB_COPYPASTE_TRIM_TRAILING_NL", "貼り付け時に末尾の改行を削除する"),
                   isOn: $config.trimTrailingNewline)
            Toggle(TT.text("DLG_TAB_GENERAL_CLICKURL", "クリッカブルURLを有効にする"), isOn: $config.clickableUrl)

            HStack {
                Text(TT.text("DLG_TAB_COPYPASTE_DELIMITER", "区切り文字"))
                TextField("", text: $config.delimiters)
                    .font(.system(.body, design: .monospaced))
            }

            GroupBox {
                VStack(alignment: .leading, spacing: 6) {
                    Toggle(TT.text("DLG_TAB_COPYPASTE_CONFIRM_CHANGE_PASTE", "危険なクリップボードの貼り付けを確認する"),
                           isOn: $config.confirmPaste)
                    Text(L("改行を含むときと、次の語を含むときに、貼り付ける前に内容を確かめます（1 行に 1 つ）。"))
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                    TextEditor(text: $wordsText)
                        .font(.system(.body, design: .monospaced))
                        .frame(height: 120)
                        .disabled(!config.confirmPaste)
                    Button(L("既定に戻す")) {
                        wordsText = defaultDangerousWords().joined(separator: "\n")
                    }
                    .disabled(!config.confirmPaste)
                }
                .padding(6)
            }

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    config.dangerousWords = wordsText
                        .split(whereSeparator: \.isNewline)
                        .map { $0.trimmingCharacters(in: .whitespaces) }
                        .filter { !$0.isEmpty }
                    config.save()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 480)
        .onAppear { wordsText = config.dangerousWords.joined(separator: "\n") }
    }
}

/// 貼り付けの確認（Tera Term の「クリップボードの確認」）。内容を直してから送れる。
struct PasteConfirmView: View {
    @ObservedObject var controller: TerminalController
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""

    var body: some View {
        let words = CopyPasteConfig.saved.dangerousWords
        let check = checkPaste(text: text, words: words)
        VStack(alignment: .leading, spacing: 12) {
            Text(TT.text("DLG_CLIPBOARD_TITLE", "クリップボードの確認")).font(.headline)
            if !check.dangerous.isEmpty {
                Label(L("危険な語が含まれています: %@", check.dangerous.joined(separator: ", ")),
                      systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text(L("%@ 行を貼り付けます。改行の位置でそれぞれ実行されます。", String(check.lines)))
                .font(.callout).foregroundStyle(.secondary)
            TextEditor(text: $text)
                .font(.system(.body, design: .monospaced))
                .frame(minHeight: 160, maxHeight: 320)
                .border(Color.secondary.opacity(0.4))
            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) {
                    controller.pendingPaste = nil
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)
                Button(L("貼り付け")) {
                    controller.pendingPaste = nil
                    dismiss()
                    controller.sendPaste(text)
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 520)
        .onAppear { text = controller.pendingPaste ?? "" }
    }
}
