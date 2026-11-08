import AppKit
import MTCore

/// 「設定 > 設定の保存」：今の設定を TERATERM.INI の形で書き出す（Windows の Tera Term で読める）。
/// パスワードは書かない。
enum SettingsExport {
    @MainActor
    static func chooseAndExport(_ controller: TerminalController?) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "TERATERM.INI"
        panel.message = L("Tera Term の形で設定を保存します。")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try exportTeratermIni(path: url.path, settings: current())
            controller?.showNotice(L("%@ に保存しました", url.lastPathComponent))
        } catch {
            controller?.errorMessage = TerminalController.describe(error)
        }
    }

    /// 書き出す設定（シリアル、シェル・SSH・Telnet 用の端末、見た目、ログ、コピーと貼り付け、その他、接続先）。
    @MainActor
    static func current() -> IniImport {
        let d = UserDefaults.standard
        let term = TerminalConfig.saved(.other).settings
        let tc = TerminalConfig.saved(.other)
        let look = AppearanceConfig.saved
        let log = LogConfig.saved
        let cp = CopyPasteConfig.saved
        let other = OtherConfig.saved
        let shape: String
        switch look.cursorShape {
        case .beam: shape = "beam"
        case .underline: shape = "underline"
        default: shape = "block"
        }
        let hosts: [MacroConnect] = HostStore.shared.recent.compactMap { t in
            switch t {
            case let .serial(port, config): return .serial(port: port, baud: config.baud)
            case let .telnet(host, port, _): return .telnet(host: host, port: port ?? 23)
            case let .tcp(host, port, _): return .tcp(host: host, port: port)
            case let .ssh(host, port, user, _): return .ssh(host: host, port: port, user: user, password: nil)
            default: return nil
            }
        }
        // 96 dpi のピクセル（Tera Term の VTFont の高さ）
        let px = UInt32((look.fontSize * 96 / 72).rounded())
        return IniImport(
            serial: SerialConfig.saved.settings,
            newlineReceive: term.newlineReceive,
            newlineSend: term.newlineSend,
            localEcho: term.localEcho,
            charsetReceive: term.charsetReceive,
            charsetSend: term.charsetSend,
            terminalCols: UInt16(TerminalConfig.startupSize.cols),
            terminalRows: UInt16(TerminalConfig.startupSize.rows),
            terminalId: tc.terminalId,
            clipboardWrite: cp.remoteClipboardWrite,
            notifyClipboard: cp.notifyClipboard,
            pasteDelay: other.pasteDelay,
            remoteClearScrollback: other.remoteClearScrollback,
            remoteCursorStyle: other.remoteCursorStyle,
            bell: other.bell.rawValue,
            beepOnConnect: other.beepOnConnect,
            clearOnClose: other.clearOnClose,
            confirmDisconnect: other.confirmDisconnect,
            acceptTitle: other.titleChange.rawValue,
            alpha: UInt8(clamping: other.alphaActive),
            ansi: look.palette.map { Optional($0) },
            kanjiIn: tc.kanjiIn,
            kanjiOut: tc.kanjiOut,
            kana7bit: tc.kana7bit,
            serialReconnect: other.autoReconnect,
            ambiguousWide: d.bool(forKey: "terminal.ambiguousWide"),
            termType: d.string(forKey: "tcpip.termType").flatMap { $0.isEmpty ? nil : $0 },
            mouseTracking: other.mouseTracking,
            mouseCtrlBypass: other.mouseCtrlBypass,
            serialReconnectMs: UInt32(clamping: other.reconnectMs),
            foreground: look.foreground,
            background: look.background,
            fontName: look.fontName,
            fontPixels: px,
            cursorShape: shape,
            logDefaultName: log.name,
            logDefaultPath: log.folder,
            logAutoStart: log.autoStart,
            logBinary: d.bool(forKey: "log.binary"),
            logAppend: d.object(forKey: "log.append") == nil ? true : d.bool(forKey: "log.append"),
            logTimestamp: d.bool(forKey: "log.timestamp"),
            logTimestampUtc: d.bool(forKey: "log.utc"),
            autoTextCopy: cp.autoCopy,
            confirmChangePaste: cp.confirmPaste,
            delimList: cp.delimiters,
            clickableUrl: cp.clickableUrl,
            trimTrailingNl: cp.trimTrailingNewline,
            rightClickPaste: cp.rightClickPaste,
            middleClickPaste: cp.middleClickPaste,
            sshKeepalive: UInt32(clamping: SshSettings.keepalive),
            telnetKeepalive: UInt32(clamping: d.integer(forKey: "telnet.keepalive")),
            sendBreakMs: SerialConfig.breakMs,
            sshForwards: SshSettings.forwards,
            hosts: hosts)
    }
}
