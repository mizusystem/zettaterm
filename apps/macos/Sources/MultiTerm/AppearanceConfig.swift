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

    init() {
        let base = themes().first?.appearance
        foreground = base?.foreground ?? 0xE5E5E5
        background = base?.background ?? 0
        cursor = base?.cursor ?? 0xC0C0C0
    }

    /// コアに渡す見た目（ANSI の 16 色は配色から）。
    var appearance: Appearance {
        let ansi = themes().first(where: { $0.name == theme })?.appearance.ansi
            ?? themes().first?.appearance.ansi ?? []
        return Appearance(foreground: foreground, background: background, cursor: cursor, ansi: ansi,
                          cursorShape: cursorShape, cursorBlink: cursorBlink)
    }

    /// 配色を選んだとき、前景・背景・カーソルもその配色の色にする。
    mutating func choose(theme name: String) {
        guard let t = themes().first(where: { $0.name == name }) else { return }
        theme = name
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
        let ansi = config.appearance.ansi
        return VStack(alignment: .leading, spacing: 2) {
            Text("user@host:~$ ls").foregroundStyle(Color(rgb: config.foreground))
            HStack(spacing: 0) {
                ForEach(Array(ansi.prefix(8).enumerated()), id: \.offset) { _, c in
                    Text("■ ").foregroundStyle(Color(rgb: c))
                }
            }
            HStack(spacing: 0) {
                ForEach(Array(ansi.dropFirst(8).enumerated()), id: \.offset) { _, c in
                    Text("■ ").foregroundStyle(Color(rgb: c))
                }
            }
        }
        .font(.system(.body, design: .monospaced))
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(rgb: config.background))
        .clipShape(RoundedRectangle(cornerRadius: 4))
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
