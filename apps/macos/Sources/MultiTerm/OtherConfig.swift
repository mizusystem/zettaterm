import AppKit
import MTCore
import SwiftUI

/// Tera Term の「設定 > その他の設定」のうち、全般・制御シーケンス・表示にある項目。全ウィンドウで共通。
struct OtherConfig: Equatable {
    enum Bell: String, CaseIterable { case off, on, visual }
    /// 相手からのタイトルの変更（AcceptTitleChangeRequest）
    enum TitleChange: String, CaseIterable { case off, overwrite, ahead, last }

    /// つないだとき・切れたときに鳴らす（BeepOnConnect）
    var beepOnConnect = false
    /// 切れたら画面を消す（ClearScreenOnCloseConnection）
    var clearOnClose = false
    /// 「接続断」の前に確かめる（ConfirmDisconnect）
    var confirmDisconnect = true
    var bell: Bell = .on
    var titleChange: TitleChange = .overwrite
    /// 相手からのスクロールバッファの消去（ClearScrollBufferFromRemote）
    var remoteClearScrollback = false
    /// 相手からのカーソルの形・点滅の変更（CursorCtrlSequence）
    var remoteCursorStyle = false
    /// 貼り付けの行間遅延（ミリ秒。PasteDelayPerLine）
    var pasteDelay: UInt32 = 10
    /// ウィンドウの不透明度（0〜255。AlphaBlendActive / AlphaBlend）
    var alphaActive: Int = 255
    var alphaInactive: Int = 255
    /// シリアルが抜けたら自動でつなぎ直す（AutoComPortReconnect）と、その間隔（ミリ秒）
    var autoReconnect = true
    var reconnectMs: Int = 500
    /// 相手が求めたらマウスの操作を送る（MouseEventTracking）。Shift を押していれば送らずに選択する
    var mouseTracking = true
    /// Ctrl を押している間も送らない（DisableMouseTrackingByCtrl）
    var mouseCtrlBypass = true

    private static let prefix = "other."

    static var saved: OtherConfig {
        let d = UserDefaults.standard
        var c = OtherConfig()
        func bool(_ k: String, _ v: inout Bool) { if d.object(forKey: prefix + k) != nil { v = d.bool(forKey: prefix + k) } }
        bool("beepOnConnect", &c.beepOnConnect)
        bool("clearOnClose", &c.clearOnClose)
        bool("confirmDisconnect", &c.confirmDisconnect)
        bool("remoteClearScrollback", &c.remoteClearScrollback)
        bool("remoteCursorStyle", &c.remoteCursorStyle)
        bool("autoReconnect", &c.autoReconnect)
        bool("mouseTracking", &c.mouseTracking)
        bool("mouseCtrlBypass", &c.mouseCtrlBypass)
        if d.object(forKey: prefix + "reconnectMs") != nil { c.reconnectMs = d.integer(forKey: prefix + "reconnectMs") }
        if let b = d.string(forKey: prefix + "bell").flatMap(Bell.init) { c.bell = b }
        if let t = d.string(forKey: prefix + "titleChange").flatMap(TitleChange.init) { c.titleChange = t }
        if d.object(forKey: prefix + "pasteDelay") != nil { c.pasteDelay = UInt32(clamping: d.integer(forKey: prefix + "pasteDelay")) }
        if d.object(forKey: prefix + "alphaActive") != nil { c.alphaActive = d.integer(forKey: prefix + "alphaActive") }
        if d.object(forKey: prefix + "alphaInactive") != nil { c.alphaInactive = d.integer(forKey: prefix + "alphaInactive") }
        return c
    }

    func save() {
        let d = UserDefaults.standard
        let p = Self.prefix
        d.set(beepOnConnect, forKey: p + "beepOnConnect")
        d.set(clearOnClose, forKey: p + "clearOnClose")
        d.set(confirmDisconnect, forKey: p + "confirmDisconnect")
        d.set(remoteClearScrollback, forKey: p + "remoteClearScrollback")
        d.set(remoteCursorStyle, forKey: p + "remoteCursorStyle")
        d.set(bell.rawValue, forKey: p + "bell")
        d.set(titleChange.rawValue, forKey: p + "titleChange")
        d.set(Int(pasteDelay), forKey: p + "pasteDelay")
        d.set(alphaActive, forKey: p + "alphaActive")
        d.set(alphaInactive, forKey: p + "alphaInactive")
        d.set(autoReconnect, forKey: p + "autoReconnect")
        d.set(mouseTracking, forKey: p + "mouseTracking")
        d.set(mouseCtrlBypass, forKey: p + "mouseCtrlBypass")
        d.set(reconnectMs, forKey: p + "reconnectMs")
    }

    /// 開いている接続と窓にすぐ効かせる。
    @MainActor
    func applyEverywhere() {
        for c in SessionRegistry.shared.controllers {
            c.view.applyOther(self)
            c.objectWillChange.send()
        }
    }

    /// 相手のタイトルと接続先から、ウィンドウのタイトル。
    func title(remote: String, base: String) -> String {
        guard !remote.isEmpty else { return base }
        switch titleChange {
        case .off: return base
        case .overwrite: return remote
        case .ahead: return base.isEmpty ? remote : "\(remote) - \(base)"
        case .last: return base.isEmpty ? remote : "\(base) - \(remote)"
        }
    }
}

/// 「設定 > その他の設定」。
struct OtherSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var c = OtherConfig.saved

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_TABSHEET_TITLE", "その他の設定").replacingOccurrences(of: "Tera Term: ", with: ""))
                .font(.headline)

            GroupBox(TT.text("DLG_TABSHEET_TITLE_GENERAL", "全般")) {
                VStack(alignment: .leading, spacing: 8) {
                    Toggle(L("接続時・切断時にビープ音を鳴らす"), isOn: $c.beepOnConnect)
                    Toggle(L("切断したら画面をクリアする"), isOn: $c.clearOnClose)
                    Toggle(L("接続断の前に確認する"), isOn: $c.confirmDisconnect)
                    HStack {
                        Toggle(L("シリアルを抜いたら自動で再接続する"), isOn: $c.autoReconnect)
                        TextField("", value: $c.reconnectMs, format: .number.grouping(.never)).frame(width: 60)
                            .disabled(!c.autoReconnect)
                        Text(L("ミリ秒ごと"))
                    }
                }
                .padding(8).frame(maxWidth: .infinity, alignment: .leading)
            }

            GroupBox(TT.text("DLG_TABSHEET_TITLE_SEQUENCE", "制御シーケンス")) {
                VStack(alignment: .leading, spacing: 8) {
                    Toggle(TT.text("DLG_TAB_SEQUENCE_ACCEPT_CLEAR_SBUFF", "リモートからのスクロールバッファの消去を受け入れる"),
                           isOn: $c.remoteClearScrollback)
                    Toggle(TT.text("DLG_TAB_SEQUENCE_CURSOR_CTRL", "カーソル形状/点滅変更制御シーケンス"),
                           isOn: $c.remoteCursorStyle)
                    Toggle(TT.text("DLG_TAB_SEQUENCE_ACCEPT_MOUSE_EVENT_TRACKING", "マウスイベント通知要求を受け入れる"),
                           isOn: $c.mouseTracking)
                    Toggle(TT.text("DLG_TAB_SEQUENCE_DISABLE_MOUSE_TRACKING_CTRL", "Controlキーを押している間はマウスイベントを通知しない"),
                           isOn: $c.mouseCtrlBypass)
                        .disabled(!c.mouseTracking)
                        .padding(.leading, 20)
                    HStack {
                        Text(TT.text("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING", "ウィンドウタイトルの変更要求を受け入れる"))
                        Picker("", selection: $c.titleChange) {
                            Text(TT.text("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING_OFF", "無効")).tag(OtherConfig.TitleChange.off)
                            Text(TT.text("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING_OVERWRITE", "上書き")).tag(OtherConfig.TitleChange.overwrite)
                            Text(TT.text("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING_AHEAD", "前に追加")).tag(OtherConfig.TitleChange.ahead)
                            Text(TT.text("DLG_TAB_SEQUENCE_ACCEPT_TITLE_CHANGING_LAST", "後に追加")).tag(OtherConfig.TitleChange.last)
                        }
                        .labelsHidden().fixedSize()
                    }
                    HStack {
                        Text(TT.text("DLG_TAB_SEQUENCE_BEEP_LABEL", "ベル"))
                        Picker("", selection: $c.bell) {
                            Text(TT.text("DLG_TAB_SEQUENCE_BEEP_DISABLE", "無視")).tag(OtherConfig.Bell.off)
                            Text(TT.text("DLG_TAB_SEQUENCE_BEEP_SOUND", "サウンド")).tag(OtherConfig.Bell.on)
                            Text(TT.text("DLG_TAB_SEQUENCE_BEEP_VISUALBELL", "ビジュアルベル")).tag(OtherConfig.Bell.visual)
                        }
                        .labelsHidden().fixedSize()
                    }
                }
                .padding(8).frame(maxWidth: .infinity, alignment: .leading)
            }

            GroupBox(TT.text("DLG_TABSHEET_TITLE_COPYPASTE", "コピーと貼り付け")) {
                HStack {
                    Text(TT.text("DLG_TAB_COPYPASTE_PASTEDELAY", "貼り付けの行間遅延"))
                    TextField("", value: $c.pasteDelay, format: .number).frame(width: 60)
                    Text(TT.text("DLG_TAB_COPYPASTE_PASTEDELAY2", "ミリ秒"))
                }
                .padding(8).frame(maxWidth: .infinity, alignment: .leading)
            }

            GroupBox(TT.text("DLG_TAB_VISUAL_ALPHA", "ウィンドウの不透明度")) {
                Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 6) {
                    GridRow {
                        Text(TT.text("DLG_TAB_VISUAL_ALPHA_ACTIVE", "アクティブ時"))
                        Slider(value: alpha(\.alphaActive), in: 64...255)
                        Text("\(c.alphaActive)").monospacedDigit().frame(width: 32, alignment: .trailing)
                    }
                    GridRow {
                        Text(TT.text("DLG_TAB_VISUAL_ALPHA_INACTIVE", "非アクティブ時"))
                        Slider(value: alpha(\.alphaInactive), in: 64...255)
                        Text("\(c.alphaInactive)").monospacedDigit().frame(width: 32, alignment: .trailing)
                    }
                }
                .padding(8).frame(maxWidth: .infinity, alignment: .leading)
            }

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    c.pasteDelay = min(c.pasteDelay, 5000)
                    c.reconnectMs = max(100, min(c.reconnectMs, 60_000))
                    c.save()
                    c.applyEverywhere()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 520)
    }

    private func alpha(_ key: WritableKeyPath<OtherConfig, Int>) -> Binding<Double> {
        Binding(get: { Double(c[keyPath: key]) }, set: { c[keyPath: key] = Int($0.rounded()) })
    }
}
