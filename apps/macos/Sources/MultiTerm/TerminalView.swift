import AppKit
import CoreText
import MTCore
import QuartzCore

/// 端末ビュー。コアの snapshot を Core Text で描き、キー入力・IME・貼り付け・スクロールを扱う。
final class TerminalView: NSView, @preconcurrency NSTextInputClient {
    var onTitle: ((String) -> Void)?
    var onClosed: (() -> Void)?

    private let profile: Profile
    private var session: Session?
    private var bridge: SessionBridge?
    private var screen = ScreenBuffer()
    private var displayLink: CADisplayLink?
    private var gridSize: (cols: UInt16, rows: UInt16) = (0, 0)
    private var scrollRemainder: CGFloat = 0
    private var markedText: String?

    // フォントとセル寸法
    private let font: CTFont
    private let boldFont: CTFont
    private let italicFont: CTFont
    private let geometry: GridGeometry
    private let ascent: CGFloat
    private var colorCache: [UInt32: CGColor] = [:]

    init(profile: Profile) {
        self.profile = profile
        let base = NSFont(name: "Menlo", size: 14) ?? .monospacedSystemFont(ofSize: 14, weight: .regular)
        font = base as CTFont
        boldFont = CTFontCreateCopyWithSymbolicTraits(font, 0, nil, .traitBold, .traitBold) ?? font
        italicFont = CTFontCreateCopyWithSymbolicTraits(font, 0, nil, .traitItalic, .traitItalic) ?? font

        var glyph = CGGlyph()
        var advance = CGSize.zero
        var ch: UniChar = 0x4D // "M"
        CTFontGetGlyphsForCharacters(font, &ch, &glyph, 1)
        CTFontGetAdvancesForGlyphs(font, .horizontal, &glyph, &advance, 1)
        ascent = ceil(CTFontGetAscent(font))
        let height = ascent + ceil(CTFontGetDescent(font)) + ceil(CTFontGetLeading(font))
        geometry = GridGeometry(cellWidth: Double(ceil(advance.width * 2) / 2), cellHeight: Double(height), padding: 4)
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }

    private var cellW: CGFloat { CGFloat(geometry.cellWidth) }
    private var cellH: CGFloat { CGFloat(geometry.cellHeight) }
    private var pad: CGFloat { CGFloat(geometry.padding) }

    // MARK: - セッション

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil else { return }
        if session == nil { openSession() }
        window?.makeFirstResponder(self)
        if displayLink == nil {
            let link = displayLink(target: self, selector: #selector(tick))
            link.add(to: .main, forMode: .common)
            displayLink = link
        }
    }

    private func openSession() {
        let size = geometry.size(width: bounds.width, height: bounds.height)
        gridSize = size
        let bridge = SessionBridge(
            onState: { [weak self] state in
                MainActor.assumeIsolated {
                    if state == .closed { self?.onClosed?() }
                }
            },
            onTitle: { [weak self] title in
                MainActor.assumeIsolated { self?.onTitle?(title) }
            },
            onBell: {
                MainActor.assumeIsolated { NSSound.beep() }
            }
        )
        self.bridge = bridge
        do {
            session = try Session.open(profile: profile, cols: size.cols, rows: size.rows, listener: bridge)
        } catch {
            NSAlert(error: error).runModal()
        }
    }

    func closeSession() {
        displayLink?.invalidate()
        displayLink = nil
        session?.close()
        session = nil
    }

    /// 画面の更新タイミングごとに呼ばれる。変更があったときだけ snapshot を取る。
    @objc private func tick() {
        guard let session, bridge?.takeDirty() == true else { return }
        let snapshot = session.snapshot(since: screen.generation)
        let changed = screen.apply(snapshot)
        if screen.scrollOffset > 0 {
            // さかのぼって見ている間は画面全体を描き直す（行の対応がずれるため）
            needsDisplay = true
        } else if let changed {
            for row in changed { setNeedsDisplay(rowRect(row)) }
        } else {
            needsDisplay = true
        }
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        let size = geometry.size(width: newSize.width, height: newSize.height)
        guard size != gridSize else { return }
        gridSize = size
        let scale = window?.backingScaleFactor ?? 2
        session?.resize(
            cols: size.cols, rows: size.rows,
            pixelWidth: UInt16(clamping: Int(newSize.width * scale)),
            pixelHeight: UInt16(clamping: Int(newSize.height * scale))
        )
    }

    // MARK: - 描画

    private func rowRect(_ row: Int) -> NSRect {
        NSRect(x: 0, y: pad + CGFloat(row) * cellH, width: bounds.width, height: cellH)
    }

    private func color(_ rgb: UInt32) -> CGColor {
        if let c = colorCache[rgb] { return c }
        let c = CGColor(
            srgbRed: CGFloat((rgb >> 16) & 0xFF) / 255,
            green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255,
            alpha: 1
        )
        colorCache[rgb] = c
        return c
    }

    override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.setFillColor(color(screen.defaultBg))
        ctx.fill(dirtyRect)

        let rows: [Row]
        if screen.scrollOffset > 0, let session {
            rows = session.scrollback(from: Int64(-screen.scrollOffset), rows: UInt16(clamping: screen.rows))
        } else {
            rows = screen.lines
        }

        let first = max(0, Int(((dirtyRect.minY - pad) / cellH).rounded(.down)))
        let last = min(rows.count - 1, Int(((dirtyRect.maxY - pad) / cellH).rounded(.up)))
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        if first <= last {
            for i in first...last { drawRow(rows[i], at: i, in: ctx) }
        }

        if screen.scrollOffset == 0 {
            drawCursor(in: ctx, rows: rows)
            drawMarkedText(in: ctx)
        }
    }

    private func drawRow(_ row: Row, at index: Int, in ctx: CGContext) {
        let y = pad + CGFloat(index) * cellH
        // 背景：既定色以外の連続区間をまとめて塗る
        var col = 0
        while col < row.cells.count {
            let bg = row.cells[col].bg
            var end = col + 1
            while end < row.cells.count && row.cells[end].bg == bg { end += 1 }
            if bg != screen.defaultBg {
                ctx.setFillColor(color(bg))
                ctx.fill(CGRect(x: pad + CGFloat(col) * cellW, y: y, width: CGFloat(end - col) * cellW, height: cellH))
            }
            col = end
        }
        // 文字：セル位置に 1 文字ずつ置く（全角やフォールバックフォントでも桁がずれない）
        for (c, cell) in row.cells.enumerated() {
            drawCell(cell, x: pad + CGFloat(c) * cellW, y: y, fg: cell.fg, in: ctx)
        }
    }

    private func drawCell(_ cell: Cell, x: CGFloat, y: CGFloat, fg: UInt32, in ctx: CGContext) {
        let flags = CellFlags(rawValue: cell.flags)
        if flags.contains(.wideSpacer) { return }
        let width = flags.contains(.wide) ? cellW * 2 : cellW
        if !cell.text.isEmpty && cell.text != " " {
            let f = flags.contains(.bold) ? boldFont : (flags.contains(.italic) ? italicFont : font)
            let attrs: [NSAttributedString.Key: Any] = [
                NSAttributedString.Key(kCTFontAttributeName as String): f,
                NSAttributedString.Key(kCTForegroundColorAttributeName as String): color(fg),
            ]
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: cell.text, attributes: attrs))
            ctx.textPosition = CGPoint(x: x, y: y + ascent)
            CTLineDraw(line, ctx)
        }
        if flags.contains(.underline) {
            ctx.setFillColor(color(fg))
            ctx.fill(CGRect(x: x, y: y + ascent + 2, width: width, height: 1))
        }
        if flags.contains(.strikeout) {
            ctx.setFillColor(color(fg))
            ctx.fill(CGRect(x: x, y: y + cellH / 2, width: width, height: 1))
        }
    }

    private func cursorRect() -> CGRect {
        CGRect(
            x: pad + CGFloat(screen.cursor.col) * cellW,
            y: pad + CGFloat(screen.cursor.row) * cellH,
            width: cellW, height: cellH
        )
    }

    private func drawCursor(in ctx: CGContext, rows: [Row]) {
        let r = Int(screen.cursor.row), c = Int(screen.cursor.col)
        guard r < rows.count else { return }
        let cell = c < rows[r].cells.count ? rows[r].cells[c] : nil
        let wide = cell.map { CellFlags(rawValue: $0.flags).contains(.wide) } ?? false
        var rect = cursorRect()
        if wide { rect.size.width *= 2 }
        let focused = window?.isKeyWindow == true && window?.firstResponder === self

        ctx.setFillColor(color(screen.cursorColor))
        switch screen.cursor.shape {
        case .hidden:
            return
        case .underline:
            ctx.fill(CGRect(x: rect.minX, y: rect.maxY - 2, width: rect.width, height: 2))
        case .beam:
            ctx.fill(CGRect(x: rect.minX, y: rect.minY, width: 2, height: rect.height))
        case .block where !focused:
            ctx.setStrokeColor(color(screen.cursorColor))
            ctx.stroke(rect.insetBy(dx: 0.5, dy: 0.5))
        case .block:
            ctx.fill(rect)
            if let cell { drawCell(cell, x: rect.minX, y: rect.minY, fg: screen.defaultBg, in: ctx) }
        }
    }

    private func drawMarkedText(in ctx: CGContext) {
        guard let markedText, !markedText.isEmpty else { return }
        let rect = cursorRect()
        let attrs: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTFontAttributeName as String): font,
            NSAttributedString.Key(kCTForegroundColorAttributeName as String): color(screen.defaultFg),
        ]
        let line = CTLineCreateWithAttributedString(NSAttributedString(string: markedText, attributes: attrs))
        let width = CTLineGetTypographicBounds(line, nil, nil, nil)
        ctx.setFillColor(color(screen.defaultBg))
        ctx.fill(CGRect(x: rect.minX, y: rect.minY, width: width, height: cellH))
        ctx.textPosition = CGPoint(x: rect.minX, y: rect.minY + ascent)
        CTLineDraw(line, ctx)
        ctx.setFillColor(color(screen.defaultFg))
        ctx.fill(CGRect(x: rect.minX, y: rect.minY + ascent + 2, width: width, height: 1))
    }

    // MARK: - 入力

    private func send(_ key: Key, _ mods: Modifiers = Modifiers(shift: false, ctrl: false, alt: false)) {
        if screen.scrollToBottom() { needsDisplay = true }
        session?.key(key: key, mods: mods)
    }

    override func keyDown(with event: NSEvent) {
        let flags = event.modifierFlags
        if flags.contains(.command) {
            super.keyDown(with: event)
            return
        }
        // 変換中の文字があるときは、Return や矢印も IME に渡す
        if !hasMarkedText() {
            if let key = KeyMapping.specialKey(keyCode: event.keyCode) {
                send(key, KeyMapping.modifiers(flags))
                return
            }
            if flags.contains(.control), let chars = event.charactersIgnoringModifiers, chars.count == 1 {
                send(.text(text: chars), KeyMapping.modifiers(flags))
                return
            }
        }
        interpretKeyEvents([event])
    }

    override func doCommand(by selector: Selector) {
        if let key = KeyMapping.key(forCommand: selector) {
            send(key)
        }
    }

    @objc func paste(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string) else { return }
        if screen.scrollToBottom() { needsDisplay = true }
        session?.paste(text: text)
    }

    override func scrollWheel(with event: NSEvent) {
        guard !screen.altScreen else { return }
        let lines = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY / cellH : event.scrollingDeltaY
        scrollRemainder += lines
        let whole = Int(scrollRemainder.rounded(.towardZero))
        scrollRemainder -= CGFloat(whole)
        if whole != 0 && screen.scroll(by: whole) { needsDisplay = true }
    }

    override func becomeFirstResponder() -> Bool {
        needsDisplay = true
        return true
    }

    override func resignFirstResponder() -> Bool {
        needsDisplay = true
        return true
    }

    // MARK: - NSTextInputClient（IME）

    func insertText(_ string: Any, replacementRange: NSRange) {
        let text = (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
        markedText = nil
        needsDisplay = true
        if !text.isEmpty { send(.text(text: text)) }
    }

    func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        markedText = (string as? NSAttributedString)?.string ?? (string as? String)
        needsDisplay = true
    }

    func unmarkText() {
        markedText = nil
        needsDisplay = true
    }

    func selectedRange() -> NSRange { NSRange(location: NSNotFound, length: 0) }

    func markedRange() -> NSRange {
        guard let markedText, !markedText.isEmpty else { return NSRange(location: NSNotFound, length: 0) }
        return NSRange(location: 0, length: (markedText as NSString).length)
    }

    func hasMarkedText() -> Bool { !(markedText ?? "").isEmpty }

    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? { nil }

    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        guard let window else { return .zero }
        return window.convertToScreen(convert(cursorRect(), to: nil))
    }

    func characterIndex(for point: NSPoint) -> Int { NSNotFound }
}

/// mt_vt::CellFlags と同じビット。
struct CellFlags: OptionSet {
    let rawValue: UInt16
    static let bold = CellFlags(rawValue: 1 << 0)
    static let italic = CellFlags(rawValue: 1 << 1)
    static let underline = CellFlags(rawValue: 1 << 2)
    static let strikeout = CellFlags(rawValue: 1 << 3)
    static let dim = CellFlags(rawValue: 1 << 4)
    static let wide = CellFlags(rawValue: 1 << 5)
    static let wideSpacer = CellFlags(rawValue: 1 << 6)
}
