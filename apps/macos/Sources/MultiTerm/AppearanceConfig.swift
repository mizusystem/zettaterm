import AppKit
import MTCore
import SwiftUI

/// 「設定 > ウィンドウ」（配色・カーソル形状）と「設定 > フォント」。全ウィンドウで共通。
struct AppearanceConfig: Equatable {
    /// 配色の名前（themes() の name）。色を個別に変えたら前景・背景・カーソルはそちらが優先
    var theme = "xterm"
    var foreground: UInt32
    var background: UInt32
    var cursor: UInt32
    var cursorShape: CursorShape = .block
    var cursorBlink = false
    var fontName = "Menlo"
    var fontSize: Double = 14
    /// 太字（既定の文字色のとき）と URL を別の色で出す（Tera Term の VTBoldColor / URLColor。nil で出さない）
    var boldColor: UInt32?
    var urlColor: UInt32?
    /// スクロールバッファの行数（ScrollBuffSize。0 で持たない）
    var scrollback = 10_000
    /// ウィンドウのタイトル（Title。空なら Multi-Term）
    var title = ""
    /// 背景画像（空なら無し）と、その濃さ（0〜1）
    var backgroundImage = ""
    var backgroundAlpha: Double = 0.25
    /// 行間と文字間に足す余白（ポイント。Tera Term の「フォント」の文字間隔）
    var lineSpacing: Double = 0
    var charSpacing: Double = 0
    /// 個別に変えた ANSI の 16 色（nil なら配色のまま。Tera Term の ANSIColor）
    var ansi: [UInt32]?

    /// 配色の ANSI の 16 色
    var themeAnsi: [UInt32] {
        themes().first(where: { $0.name == theme })?.appearance.ansi ?? themes().first?.appearance.ansi ?? []
    }

    /// 今の ANSI の 16 色
    var palette: [UInt32] {
        if let ansi, ansi.count == 16 { return ansi }
        return themeAnsi
    }

    init() {
        let base = themes().first?.appearance
        foreground = base?.foreground ?? 0xE5E5E5
        background = base?.background ?? 0
        cursor = base?.cursor ?? 0xC0C0C0
    }

    /// コアに渡す見た目（ANSI の 16 色は配色から）。
    var appearance: Appearance {
        return Appearance(foreground: foreground, background: background, cursor: cursor, ansi: palette,
                          cursorShape: cursorShape, cursorBlink: cursorBlink)
    }

    /// 配色を選んだとき、前景・背景・カーソルもその配色の色にする。
    mutating func choose(theme name: String) {
        guard let t = themes().first(where: { $0.name == name }) else { return }
        theme = name
        ansi = nil
        foreground = t.appearance.foreground
        background = t.appearance.background
        cursor = t.appearance.cursor
    }

    private enum Key {
        static let theme = "appearance.theme"
        static let fg = "appearance.foreground"
        static let bg = "appearance.background"
        static let cursor = "appearance.cursor"
        static let shape = "appearance.cursorShape"
        static let blink = "appearance.cursorBlink"
        static let font = "appearance.fontName"
        static let size = "appearance.fontSize"
        static let ansi = "appearance.ansi"
        static let boldColor = "appearance.boldColor"
        static let urlColor = "appearance.urlColor"
        static let scrollback = "appearance.scrollback"
        static let title = "appearance.title"
        static let backgroundImage = "appearance.backgroundImage"
        static let backgroundAlpha = "appearance.backgroundAlpha"
        static let lineSpacing = "appearance.lineSpacing"
        static let charSpacing = "appearance.charSpacing"
    }

    static var saved: AppearanceConfig {
        let d = UserDefaults.standard
        var c = AppearanceConfig()
        if let t = d.string(forKey: Key.theme) { c.choose(theme: t) }
        if d.object(forKey: Key.fg) != nil { c.foreground = UInt32(truncatingIfNeeded: d.integer(forKey: Key.fg)) }
        if d.object(forKey: Key.bg) != nil { c.background = UInt32(truncatingIfNeeded: d.integer(forKey: Key.bg)) }
        if d.object(forKey: Key.cursor) != nil { c.cursor = UInt32(truncatingIfNeeded: d.integer(forKey: Key.cursor)) }
        switch d.string(forKey: Key.shape) {
        case "underline": c.cursorShape = .underline
        case "beam": c.cursorShape = .beam
        default: c.cursorShape = .block
        }
        c.cursorBlink = d.bool(forKey: Key.blink)
        if let f = d.string(forKey: Key.font), NSFont(name: f, size: 12) != nil { c.fontName = f }
        if d.object(forKey: Key.size) != nil { c.fontSize = min(72, max(6, d.double(forKey: Key.size))) }
        if d.object(forKey: Key.boldColor) != nil { c.boldColor = UInt32(truncatingIfNeeded: d.integer(forKey: Key.boldColor)) }
        if d.object(forKey: Key.urlColor) != nil { c.urlColor = UInt32(truncatingIfNeeded: d.integer(forKey: Key.urlColor)) }
        if d.object(forKey: Key.scrollback) != nil { c.scrollback = max(0, min(1_000_000, d.integer(forKey: Key.scrollback))) }
        c.title = d.string(forKey: Key.title) ?? ""
        c.backgroundImage = d.string(forKey: Key.backgroundImage) ?? ""
        if d.object(forKey: Key.backgroundAlpha) != nil { c.backgroundAlpha = min(1, max(0, d.double(forKey: Key.backgroundAlpha))) }
        c.lineSpacing = min(20, max(0, d.double(forKey: Key.lineSpacing)))
        c.charSpacing = min(20, max(0, d.double(forKey: Key.charSpacing)))
        if let a = d.array(forKey: Key.ansi) as? [Int], a.count == 16 {
            c.ansi = a.map { UInt32(truncatingIfNeeded: $0) }
        }
        return c
    }

    func save() {
        let d = UserDefaults.standard
        d.set(theme, forKey: Key.theme)
        d.set(Int(foreground), forKey: Key.fg)
        d.set(Int(background), forKey: Key.bg)
        d.set(Int(cursor), forKey: Key.cursor)
        d.set(cursorShape == .underline ? "underline" : cursorShape == .beam ? "beam" : "block", forKey: Key.shape)
        d.set(cursorBlink, forKey: Key.blink)
        d.set(fontName, forKey: Key.font)
        d.set(fontSize, forKey: Key.size)
        for (value, key) in [(boldColor, Key.boldColor), (urlColor, Key.urlColor)] {
            if let value { d.set(Int(value), forKey: key) } else { d.removeObject(forKey: key) }
        }
        d.set(scrollback, forKey: Key.scrollback)
        d.set(title, forKey: Key.title)
        d.set(backgroundImage, forKey: Key.backgroundImage)
        d.set(backgroundAlpha, forKey: Key.backgroundAlpha)
        d.set(lineSpacing, forKey: Key.lineSpacing)
        d.set(charSpacing, forKey: Key.charSpacing)
        // 配色と同じなら覚えない（配色を変えたときに付いていく）
        if let ansi, ansi != themeAnsi {
            d.set(ansi.map { Int($0) }, forKey: Key.ansi)
        } else {
            d.removeObject(forKey: Key.ansi)
        }
        NotificationCenter.default.post(name: .appearanceConfigChanged, object: nil)
    }

    var font: NSFont {
        NSFont(name: fontName, size: fontSize) ?? .monospacedSystemFont(ofSize: fontSize, weight: .regular)
    }
}

extension Notification.Name {
    static let appearanceConfigChanged = Notification.Name("MultiTerm.appearanceConfigChanged")
}

extension Color {
    init(rgb: UInt32) {
        self.init(.sRGB,
                  red: Double((rgb >> 16) & 0xFF) / 255,
                  green: Double((rgb >> 8) & 0xFF) / 255,
                  blue: Double(rgb & 0xFF) / 255)
    }

    /// 0xRRGGBB（sRGB）。
    var rgb: UInt32 {
        guard let c = NSColor(self).usingColorSpace(.sRGB) else { return 0 }
        let v = { (x: CGFloat) in UInt32(max(0, min(255, (x * 255).rounded()))) }
        return (v(c.redComponent) << 16) | (v(c.greenComponent) << 8) | v(c.blueComponent)
    }
}

/// 「設定 > ウィンドウ」：配色、文字色・背景色・カーソル色、カーソルの形と点滅。
struct WindowSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var config = AppearanceConfig.saved

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("DLG_WIN_TITLE", "ウィンドウの設定")).font(.headline)

            Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 10) {
                GridRow {
                    Text(L("配色:")).gridColumnAlignment(.trailing)
                    Picker("", selection: Binding(get: { config.theme }, set: { config.choose(theme: $0) })) {
                        ForEach(themes(), id: \.name) { Text($0.name).tag($0.name) }
                    }
                    .labelsHidden().fixedSize()
                }
                GridRow {
                    Text(L("文字色:"))
                    ColorPicker("", selection: color(\.foreground), supportsOpacity: false).labelsHidden()
                }
                GridRow {
                    Text(L("背景色:"))
                    ColorPicker("", selection: color(\.background), supportsOpacity: false).labelsHidden()
                }
                GridRow {
                    Text(L("カーソル色:"))
                    ColorPicker("", selection: color(\.cursor), supportsOpacity: false).labelsHidden()
                }
                GridRow {
                    Text(L("カーソルの形:"))
                    Picker("", selection: $config.cursorShape) {
                        Text(L("ブロック")).tag(CursorShape.block)
                        Text(L("下線")).tag(CursorShape.underline)
                        Text(L("縦線")).tag(CursorShape.beam)
                    }
                    .pickerStyle(.segmented).labelsHidden().fixedSize()
                }
                GridRow {
                    Color.clear.frame(width: 1, height: 1)
                    Toggle(L("カーソルを点滅させる"), isOn: $config.cursorBlink)
                }
                GridRow {
                    Text(TT.text("DLG_WIN_BOLD", "太字") + ":")
                    optionalColor(\.boldColor, default: 0xFFFF00)
                }
                GridRow {
                    Text(TT.text("DLG_WIN_URL", "URL") + ":")
                    optionalColor(\.urlColor, default: 0x00FFFF)
                }
                GridRow {
                    Text(TT.text("DLG_WIN_SCROLL1", "スクロールバッファ:"))
                    HStack {
                        TextField("", value: $config.scrollback, format: .number.grouping(.never)).frame(width: 80)
                        Text(TT.text("DLG_WIN_SCROLL3", "行"))
                    }
                }
                GridRow {
                    Text(TT.text("DLG_WIN_TITLELABEL", "タイトル:"))
                    TextField("Multi-Term", text: $config.title).frame(width: 200)
                }
                GridRow {
                    Text(L("背景画像:"))
                    HStack {
                        Text(config.backgroundImage.isEmpty ? L("なし") : (config.backgroundImage as NSString).lastPathComponent)
                            .lineLimit(1).truncationMode(.middle).frame(maxWidth: 160, alignment: .leading)
                        Button("…") {
                            let panel = NSOpenPanel()
                            panel.allowedContentTypes = [.image]
                            if panel.runModal() == .OK, let url = panel.url { config.backgroundImage = url.path }
                        }
                        if !config.backgroundImage.isEmpty {
                            Button(L("外す")) { config.backgroundImage = "" }
                            Slider(value: $config.backgroundAlpha, in: 0.05...1).frame(width: 100)
                        }
                    }
                }
            }

            preview

            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    config.save()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 440)
    }

    private var preview: some View {
        return VStack(alignment: .leading, spacing: 4) {
            Text("user@host:~$ ls").foregroundStyle(Color(rgb: config.foreground))
            // ANSI の 16 色（押すと変えられる）
            ForEach([0, 8], id: \.self) { start in
                HStack(spacing: 4) {
                    ForEach(start..<start + 8, id: \.self) { i in
                        ColorPicker("", selection: ansiColor(i), supportsOpacity: false).labelsHidden()
                    }
                }
            }
        }
        .font(.system(.body, design: .monospaced))
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(rgb: config.background))
        .clipShape(RoundedRectangle(cornerRadius: 4))
    }

    private func ansiColor(_ i: Int) -> Binding<Color> {
        Binding(get: { Color(rgb: config.palette[safe: i] ?? 0) }, set: {
            var p = config.palette
            guard i < p.count else { return }
            p[i] = $0.rgb
            config.ansi = p
        })
    }

    /// 「別の色で出す」のチェックと色。
    private func optionalColor(_ key: WritableKeyPath<AppearanceConfig, UInt32?>, default fallback: UInt32) -> some View {
        HStack {
            Toggle(L("別の色で出す"), isOn: Binding(
                get: { config[keyPath: key] != nil },
                set: { config[keyPath: key] = $0 ? (config[keyPath: key] ?? fallback) : nil }))
            if config[keyPath: key] != nil {
                ColorPicker("", selection: Binding(
                    get: { Color(rgb: config[keyPath: key] ?? fallback) },
                    set: { config[keyPath: key] = $0.rgb }), supportsOpacity: false).labelsHidden()
            }
        }
    }

    private func color(_ key: WritableKeyPath<AppearanceConfig, UInt32>) -> Binding<Color> {
        Binding(get: { Color(rgb: config[keyPath: key]) }, set: { config[keyPath: key] = $0.rgb })
    }
}

/// 「設定 > フォント」：等幅フォントと大きさ。
struct FontSettingsView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var config = AppearanceConfig.saved

    private static let families: [String] = {
        let names = NSFontManager.shared.availableFontNames(with: .fixedPitchFontMask) ?? []
        // 太字などの書体違いは除く（ファミリーの標準の書体だけ）
        return names.filter { name in
            guard let f = NSFont(name: name, size: 12) else { return false }
            return !NSFontManager.shared.traits(of: f).contains(.boldFontMask)
                && !NSFontManager.shared.traits(of: f).contains(.italicFontMask)
        }
        .sorted()
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(TT.text("MENU_SETUP_FONT", "フォント").replacingOccurrences(of: "...", with: "")).font(.headline)
            HStack {
                Picker("", selection: $config.fontName) {
                    ForEach(Self.families, id: \.self) { Text($0).tag($0) }
                    if !Self.families.contains(config.fontName) { Text(config.fontName).tag(config.fontName) }
                }
                .labelsHidden()
                .frame(minWidth: 240)
                Stepper(value: $config.fontSize, in: 6...72, step: 1) {
                    Text(L("%@ pt", String(Int(config.fontSize))))
                }
            }
            HStack(spacing: 20) {
                Stepper(value: $config.lineSpacing, in: 0...20, step: 1) {
                    Text(L("行間 +%@", String(Int(config.lineSpacing))))
                }
                Stepper(value: $config.charSpacing, in: 0...20, step: 1) {
                    Text(L("文字間 +%@", String(Int(config.charSpacing))))
                }
            }
            Text("ABCabc 0123 日本語 ┌─┐")
                .font(Font(config.font))
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.secondary.opacity(0.15))
            HStack {
                Spacer()
                Button(TT.text("BTN_CANCEL", "キャンセル")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(TT.text("BTN_OK", "OK")) {
                    config.save()
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 440)
    }
}

extension Array {
    subscript(safe i: Int) -> Element? { indices.contains(i) ? self[i] : nil }
}
