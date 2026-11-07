import AppKit
import MTCore

/// TERATERM.INI の読み込み（「設定 > 設定の読み込み」）と、起動引数（ttermpro と同じ書き方）。
enum SettingsImport {
    /// ファイルを選んで読み込み、結果を知らせる。
    @MainActor
    static func chooseAndImport(_ controller: TerminalController?) {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.message = L("Tera Term の設定ファイル（TERATERM.INI）を選んでください。")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            let ini = try importTeratermIni(path: url.path)
            let hosts = apply(ini)
            controller?.errorMessage = L("%@ を読み込みました（接続先 %@ 件）。", url.lastPathComponent, String(hosts))
        } catch {
            controller?.errorMessage = TerminalController.describe(error)
        }
    }

    /// 読めた設定をこのアプリの設定に移す。足した接続先の数を返す。
    @MainActor
    @discardableResult
    static func apply(_ ini: IniImport) -> Int {
        let d = UserDefaults.standard

        // シリアル（ポート名は Windows の COMn なので移さない）
        if let s = ini.serial, case let .serial(_, config) = ConnectTarget(.serial(port: "", settings: s)) {
            config.save()
        }

        // 端末（Tera Term ではシリアルとそれ以外で共通なので両方に）
        for scope in [TerminalConfig.Scope.serial, .other] {
            var t = TerminalConfig.saved(scope)
            if let n = ini.newlineReceive {
                t.receive = [.cr: .cr, .crLf: .crLf, .lf: .lf, .auto: .auto][n] ?? t.receive
            }
            if let n = ini.newlineSend { t.send = [.cr: .cr, .crLf: .crLf, .lf: .lf][n] ?? t.send }
            if let e = ini.localEcho { t.localEcho = e }
            if let c = ini.charsetReceive { t.charsetReceive = choice(c) }
            if let c = ini.charsetSend { t.charsetSend = choice(c) }
            t.save(scope)
        }

        // 見た目
        var look = AppearanceConfig.saved
        if let fg = ini.foreground { look.foreground = fg }
        if let bg = ini.background { look.background = bg }
        switch ini.cursorShape {
        case "beam": look.cursorShape = .beam
        case "underline": look.cursorShape = .underline
        case "block": look.cursorShape = .block
        default: break
        }
        if let name = ini.fontName, NSFont(name: name, size: 12) != nil { look.fontName = name }
        // Tera Term の高さはピクセル（96 dpi）。ポイントに直す
        if let px = ini.fontPixels { look.fontSize = max(6, min(72, (Double(px) * 72 / 96).rounded())) }
        look.save()

        // ログ
        var log = LogConfig.saved
        if let n = ini.logDefaultName { log.name = n }
        if let p = ini.logDefaultPath, !p.isEmpty, !p.contains("\\") { log.folder = p }
        if let a = ini.logAutoStart { log.autoStart = a }
        log.save()
        if let v = ini.logBinary { d.set(v, forKey: "log.binary") }
        if let v = ini.logAppend { d.set(v, forKey: "log.append") }
        if let v = ini.logTimestamp { d.set(v, forKey: "log.timestamp") }
        if let v = ini.logTimestampUtc { d.set(v, forKey: "log.utc") }

        // コピーと貼り付け
        var cp = CopyPasteConfig.saved
        if let v = ini.autoTextCopy { cp.autoCopy = v }
        if let v = ini.confirmChangePaste { cp.confirmPaste = v }
        if let v = ini.delimList { cp.delimiters = v }
        if let v = ini.clickableUrl { cp.clickableUrl = v }
        if let v = ini.trimTrailingNl { cp.trimTrailingNewline = v }
        if let v = ini.rightClickPaste { cp.rightClickPaste = v }
        if let v = ini.middleClickPaste { cp.middleClickPaste = v }
        cp.save()

        // SSH
        if let k = ini.sshKeepalive { SshSettings.keepalive = Int(k) }
        if !ini.sshForwards.isEmpty { SshSettings.forwards = ini.sshForwards }
        SshSettings.apply()

        // 接続先の履歴
        let targets = ini.hosts.compactMap(target)
        for t in targets.reversed() { HostStore.shared.add(t) }
        return targets.count
    }

    private static func choice(_ c: Charset) -> TerminalConfig.CharsetChoice {
        switch c {
        case .utf8: return .utf8
        case .shiftJis: return .shiftJis
        case .eucJp: return .eucJp
        case .jis: return .jis
        }
    }

    /// 起動引数やホスト一覧の接続先を、このアプリの接続先に。
    static func target(_ c: MacroConnect) -> ConnectTarget? {
        switch c {
        case let .serial(port, baud):
            // Windows の COMn はこの OS では開けない
            if port.uppercased().hasPrefix("COM") { return nil }
            var config = SerialConfig.saved
            if let baud { config.baud = baud }
            return .serial(port: port, config: config)
        case let .telnet(host, port):
            return .telnet(host: host, port: port == 23 ? nil : port, ip: .auto)
        case let .tcp(host, port):
            return .tcp(host: host, port: port, ip: .auto)
        case let .ssh(host, port, user, _):
            return .ssh(host: host, port: port, user: user)
        }
    }

    // MARK: 起動引数

    /// 起動引数から読んだもの。最初のウィンドウが 1 回だけ使う。
    @MainActor static var pending: LaunchOptions?

    /// macOS が付ける引数（-AppleLanguages (ja) や -psn_ など）を除いて読む。
    @MainActor
    static func readLaunchArguments() {
        var args: [String] = []
        var skipNext = false
        for a in CommandLine.arguments.dropFirst() {
            if skipNext { skipNext = false; continue }
            if a.hasPrefix("-psn_") { continue }
            // -NSFoo value / -AppleFoo value の形
            if a.hasPrefix("-") && !a.hasPrefix("--") && a.count > 1 && a.dropFirst().first?.isUppercase == true {
                skipNext = true
                continue
            }
            args.append(a)
        }
        guard !args.isEmpty else { return }
        do {
            let launch = try parseLaunchArgs(args: args)
            if let ini = launch.ini, let imported = try? importTeratermIni(path: ini) {
                apply(imported)
            }
            if launch.connect != nil || launch.macroFile != nil { pending = launch }
        } catch {
            NSLog("Multi-Term: %@", TerminalController.describe(error))
        }
    }
}
