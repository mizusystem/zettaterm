import AppKit
import CoreText
import MTCore
import QuartzCore

/// 端末ビュー。コアの snapshot を Core Text で描き、キー入力・IME・選択・貼り付け・スクロールを扱う。
/// 選択や検索の一致は端末上の位置（TermPoint：0 が画面の最上行、負はスクロールバック）で持ち、
/// 文字列や単語の範囲の計算はコア（mt_vt::select）に任せる。
final class TerminalView: NSView, @preconcurrency NSTextInputClient {
    var onTitle: ((String) -> Void)?
    var onState: ((SessionState) -> Void)?
    var onError: ((Error) -> Void)?
    var onTransferRequest: ((TransferRequest) -> Void)?
    /// ファイルがドロップされた（Tera Term の「ファイル ドラッグ&ドロップ」）
    var onDropFiles: (([URL]) -> Void)?
    /// 貼り付けの要求（文字列と、末尾に改行を足すか）。確認ダイアログを出すかは受け取った側が決める。
    /// nil ならそのまま送る
    var onPaste: ((String, Bool) -> Void)?
    /// 出力が増えて、行が `delta` 行ぶんスクロールバックへ押し出された（検索の一致をずらすため）
    var onHistoryShift: ((Int32) -> Void)?
    /// コピーと貼り付けの設定
    private var copyPaste = CopyPasteConfig.saved

    private(set) var session: Session?
    /// 切断した接続（画面の文字の読み取りだけに使う）
    private var closedSession: Session?
    /// 画面の文字を読むセッション（接続中か、最後に切断したもの）
    var textSession: Session? { session ?? closedSession }
    /// 次に接続するときの改行コードとローカルエコー
    var terminalSettings = defaultTerminalSettings()
    /// ENQ への返事（次の接続から）
    var answerback = ""
    /// 端末 ID（ESC [ c への応答）
    var terminalId: TerminalId = .vt100
    /// 相手がクリップボードに書いた（知らせる設定のとき）
    var onClipboardWritten: ((String) -> Void)?
    /// SSH の認証・ホスト鍵確認の問い合わせ先
    var prompter: SshPrompter?
    private var bridge: SessionBridge?
    /// 今の接続の識別子。前の接続から遅れて届いた通知を捨てるのに使う
    private var sessionToken = UUID()
    private var screen = ScreenBuffer()
    private var displayLink: CADisplayLink?
    private var gridSize: (cols: UInt16, rows: UInt16) = (0, 0)
    /// 今の端末の桁数と行数（まだ決まっていなければウィンドウの大きさから）
    var terminalSize: (cols: UInt16, rows: UInt16) {
        gridSize.cols > 0 ? gridSize : geometry.size(width: bounds.width - scrollerWidth, height: bounds.height)
    }
    private var scrollRemainder: CGFloat = 0
    private var markedText: String?

    // 選択と検索
    private enum DragUnit { case cell, word, line }
    private(set) var selection: TermRange?
    private var dragUnit = DragUnit.cell
    /// ダブル・トリプルクリックで最初に選んだ範囲（ドラッグで広げるときの起点）
    private var dragAnchor: TermRange?
    private var matches: [TermRange] = []
    private var currentMatch: Int?
    /// 表示中の行の URL（世代とスクロール位置が変わったら取り直す）
    private var urlCache: (generation: UInt64, offset: Int, urls: [TermRange])?
    private let scroller = NSScroller()
    private var scrollerWidth: CGFloat { NSScroller.scrollerWidth(for: .regular, scrollerStyle: .legacy) }

    // フォントとセル寸法
    private var font: CTFont
    private var boldFont: CTFont
    private var italicFont: CTFont
    private var geometry: GridGeometry
    private var ascent: CGFloat
    /// カーソルの点滅で、今は見せているか
    private var blinkOn = true
    private var blinkTimer: Timer?
    private var colorCache: [UInt32: CGColor] = [:]

    /// フォントからセルの寸法を求める。
    private static func metrics(_ base: NSFont) -> (CTFont, CTFont, CTFont, GridGeometry, CGFloat) {
        let font = base as CTFont
        let bold = CTFontCreateCopyWithSymbolicTraits(font, 0, nil, .traitBold, .traitBold) ?? font
        let italic = CTFontCreateCopyWithSymbolicTraits(font, 0, nil, .traitItalic, .traitItalic) ?? font
        var glyph = CGGlyph()
        var advance = CGSize.zero
        var ch: UniChar = 0x4D // "M"
        CTFontGetGlyphsForCharacters(font, &ch, &glyph, 1)
        CTFontGetAdvancesForGlyphs(font, .horizontal, &glyph, &advance, 1)
        let ascent = ceil(CTFontGetAscent(font))
        let height = ascent + ceil(CTFontGetDescent(font)) + ceil(CTFontGetLeading(font))
        let geometry = GridGeometry(cellWidth: Double(ceil(advance.width * 2) / 2), cellHeight: Double(height), padding: 4)
        return (font, bold, italic, geometry, ascent)
    }

    init() {
        (font, boldFont, italicFont, geometry, ascent) = Self.metrics(AppearanceConfig.saved.font)
        super.init(frame: .zero)
        registerForDraggedTypes([.fileURL])

        scroller.scrollerStyle = .legacy
        scroller.knobProportion = 1
        scroller.doubleValue = 1
        scroller.isEnabled = false
        scroller.target = self
        scroller.action = #selector(scrollerMoved(_:))
        addSubview(scroller)

        NotificationCenter.default.addObserver(
            forName: .appearanceConfigChanged, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.applyAppearance() }
        }
        blinkTimer = Timer.scheduledTimer(withTimeInterval: 0.55, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.blink() }
        }

        NotificationCenter.default.addObserver(
            forName: .copyPasteConfigChanged, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.copyPaste = CopyPasteConfig.saved
                self?.urlCache = nil
                self?.needsDisplay = true
            }
        }
    }

    // MARK: 見た目

    /// 「設定 > ウィンドウ / フォント」を変えたとき。フォントが変われば行数・桁数も変わる。
    private func applyAppearance() {
        let config = AppearanceConfig.saved
        (font, boldFont, italicFont, geometry, ascent) = Self.metrics(config.font)
        textSession?.setAppearance(appearance: config.appearance)
        urlCache = nil
        gridSize = (0, 0)
        setFrameSize(frame.size)
        needsDisplay = true
    }

    private func blink() {
        guard screen.cursor.blinking, window?.isKeyWindow == true else {
            if !blinkOn { blinkOn = true; setNeedsDisplay(cursorRect().insetBy(dx: -cellW, dy: 0)) }
            return
        }
        blinkOn.toggle()
        var r = cursorRect()
        r.size.width *= 2
        setNeedsDisplay(r)
    }

    // MARK: ドラッグ＆ドロップ

    private func droppedFiles(_ info: NSDraggingInfo) -> [URL] {
        let urls = info.draggingPasteboard.readObjects(
            forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] ?? []
        return urls.filter { !$0.hasDirectoryPath }
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        droppedFiles(sender).isEmpty || onDropFiles == nil ? [] : .copy
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        let files = droppedFiles(sender)
        guard !files.isEmpty, let onDropFiles else { return false }
        onDropFiles(files)
        return true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }
    override var isOpaque: Bool { true }

    /// ウィンドウの大きさを、端末が `cols` × `rows` になるように変える（「設定 > 端末」の端末サイズ）。
    func fitWindow(cols: Int, rows: Int) {
        guard let window else { return }
        let want = NSSize(width: CGFloat(cols) * cellW + pad * 2 + scrollerWidth,
                          height: CGFloat(rows) * cellH + pad * 2)
        let content = window.contentRect(forFrameRect: window.frame).size
        let size = NSSize(width: content.width + want.width - bounds.width,
                          height: content.height + want.height - bounds.height)
        var frame = window.frameRect(forContentRect: NSRect(origin: .zero, size: size))
        // 左上をそのままにする
        frame.origin = NSPoint(x: window.frame.minX, y: window.frame.maxY - frame.height)
        window.setFrame(frame, display: true, animate: false)
    }

    private var cellW: CGFloat { CGFloat(geometry.cellWidth) }
    private var cellH: CGFloat { CGFloat(geometry.cellHeight) }
    private var pad: CGFloat { CGFloat(geometry.padding) }

    // MARK: - セッション

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard window != nil else { return }
        window?.makeFirstResponder(self)
        // 不透明度は前に出ているかで変える
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            NotificationCenter.default.addObserver(self, selector: #selector(keyChanged), name: name, object: window)
        }
        applyAlpha()
        if displayLink == nil {
            let link = displayLink(target: self, selector: #selector(tick))
            link.add(to: .main, forMode: .common)
            displayLink = link
        }
    }

    /// 接続する。つながっていれば先に閉じる。画面は空にする。
    /// コアの接続（Telnet の TCP 接続など）は別スレッドで待つ。待っている間に別の接続や切断が
    /// 頼まれたら、できあがった接続は閉じて CancellationError を投げる。
    func connect(_ profile: Profile) async throws {
        disconnect()
        screen = ScreenBuffer()
        closedSession = nil
        selection = nil
        matches = []
        currentMatch = nil
        urlCache = nil
        updateScroller()
        needsDisplay = true
        let token = UUID()
        sessionToken = token
        // 端末の大きさを決めるため、ウィンドウに載って大きさが決まるのを待つ（タブとして開いた直後は 0）
        for _ in 0..<100 where window == nil || bounds.width < CGFloat(geometry.cellWidth * 4) {
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        let size = geometry.size(width: bounds.width - scrollerWidth, height: bounds.height)
        gridSize = size
        let bridge = SessionBridge(
            onState: { [weak self] state in
                MainActor.assumeIsolated {
                    guard let self, self.sessionToken == token else { return }
                    if state == .closed { self.needsDisplay = true }
                    self.onState?(state)
                }
            },
            onTitle: { [weak self] title in
                MainActor.assumeIsolated {
                    guard let self, self.sessionToken == token else { return }
                    self.onTitle?(title)
                }
            },
            onBell: { [weak self] in
                MainActor.assumeIsolated { self?.ring() }
            },
            onTransferRequest: { [weak self] request in
                MainActor.assumeIsolated {
                    guard let self, self.sessionToken == token else { return }
                    self.onTransferRequest?(request)
                }
            },
            onClipboard: { [weak self] text in
                MainActor.assumeIsolated {
                    guard let self, self.sessionToken == token else { return }
                    // 設定を切ったあとに届いたものは捨てる
                    let config = CopyPasteConfig.saved
                    guard config.remoteClipboardWrite else { return }
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text, forType: .string)
                    if config.notifyClipboard { self.onClipboardWritten?(text) }
                }
            }
        )
        let terminal = terminalSettings
        let prompter: SshPrompter = self.prompter ?? SshPrompterBridge()
        // SSH はこのスレッドで認証ダイアログの答えを待つことがある。Swift の並行処理のスレッドを
        // 塞がないよう、GCD のスレッドで開く
        let opened: Session = try await withCheckedThrowingContinuation { cont in
            DispatchQueue.global(qos: .userInitiated).async {
                do {
                    cont.resume(returning: try Session.open(
                        profile: profile, cols: size.cols, rows: size.rows,
                        terminal: terminal, listener: bridge, prompter: prompter
                    ))
                } catch {
                    cont.resume(throwing: error)
                }
            }
        }
        guard sessionToken == token else {
            opened.close()
            throw CancellationError()
        }
        self.bridge = bridge
        session = opened
        opened.setAppearance(appearance: AppearanceConfig.saved.appearance)
        opened.setKeyboard(keyboard: KeyboardConfig.saved)
        opened.setAnswerback(text: answerback)
        opened.setTerminalId(id: terminalId)
        opened.setClipboardWrite(allow: CopyPasteConfig.saved.remoteClipboardWrite)
        let other = OtherConfig.saved
        opened.setRemoteControl(clearScrollback: other.remoteClearScrollback, cursorStyle: other.remoteCursorStyle)
        opened.setPasteDelay(msPerLine: other.pasteDelay)
        // 待っている間にウィンドウの大きさが変わっていれば合わせる
        let now = geometry.size(width: bounds.width - scrollerWidth, height: bounds.height)
        if now != size {
            gridSize = now
            opened.resize(cols: now.cols, rows: now.rows, pixelWidth: 0, pixelHeight: 0)
        }
    }

    @objc private func keyChanged(_ note: Notification) {
        applyAlpha()
    }

    /// 接続を閉じる。画面の内容は残す。
    func disconnect() {
        sessionToken = UUID()
        session?.close()
        // 切断後も画面の文字を選んでコピー・検索できるよう、閉じたセッションを読み取り用に残す
        if let session { closedSession = session }
        session = nil
        bridge = nil
        needsDisplay = true
    }

    /// ウィンドウを閉じるとき。
    func closeSession() {
        displayLink?.invalidate()
        displayLink = nil
        disconnect()
    }

    /// 画面の更新タイミングごとに呼ばれる。変更があったときだけ snapshot を取る。
    @objc private func tick() {
        guard let session, bridge?.takeDirty() == true else { return }
        let snapshot = session.snapshot(since: screen.generation)
        let historyBefore = screen.history
        let changed = screen.apply(snapshot)
        // 行がスクロールバックへ押し出されたぶん、選択と一致の行番号をずらす
        let pushed = Int32(screen.history - historyBefore)
        if pushed > 0 {
            selection = selection?.shifted(by: pushed)
            dragAnchor = dragAnchor?.shifted(by: pushed)
            matches = matches.map { $0.shifted(by: pushed) }
            onHistoryShift?(pushed)
        } else if screen.history < historyBefore {
            // バッファのクリアなど
            selection = nil
        }
        updateScroller()
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
        scroller.frame = NSRect(x: newSize.width - scrollerWidth, y: 0, width: scrollerWidth, height: newSize.height)
        let size = geometry.size(width: newSize.width - scrollerWidth, height: newSize.height)
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
        if screen.scrollOffset > 0, let session = textSession {
            rows = session.scrollback(from: Int64(-screen.scrollOffset), rows: UInt16(clamping: screen.rows))
        } else {
            rows = screen.lines
        }

        let first = max(0, Int(((dirtyRect.minY - pad) / cellH).rounded(.down)))
        let last = min(rows.count - 1, Int(((dirtyRect.maxY - pad) / cellH).rounded(.up)))
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        if first <= last {
            let marks = visibleMarks(rows)
            for i in first...last { drawRow(rows[i], at: i, in: ctx, marks: marks) }
        }

        if screen.scrollOffset == 0 && session != nil {
            drawCursor(in: ctx, rows: rows)
            drawMarkedText(in: ctx)
        }
        if visualBell {
            ctx.setFillColor(NSColor.white.withAlphaComponent(0.35).cgColor)
            ctx.fill(dirtyRect)
        }
    }

    // MARK: - その他の設定

    /// ビジュアルベルで画面を光らせている間
    private var visualBell = false

    /// ベル（BEL）。「その他の設定」のベルに従う。
    func ring() {
        switch OtherConfig.saved.bell {
        case .off: break
        case .on: NSSound.beep()
        case .visual:
            visualBell = true
            needsDisplay = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.12) { [weak self] in
                MainActor.assumeIsolated {
                    self?.visualBell = false
                    self?.needsDisplay = true
                }
            }
        }
    }

    /// 「その他の設定」を今の接続とウィンドウに効かせる。
    func applyOther(_ c: OtherConfig) {
        session?.setRemoteControl(clearScrollback: c.remoteClearScrollback, cursorStyle: c.remoteCursorStyle)
        session?.setPasteDelay(msPerLine: c.pasteDelay)
        applyAlpha(c)
    }

    /// ウィンドウの不透明度（前に出ているときと、そうでないとき）。
    func applyAlpha(_ c: OtherConfig = .saved) {
        guard let window else { return }
        let a = window.isKeyWindow ? c.alphaActive : c.alphaInactive
        window.alphaValue = CGFloat(max(64, min(255, a))) / 255
    }

    /// 切れたあとの画面を消す（「切断したら画面をクリアする」）。
    func clearClosedScreen() {
        closedSession = nil
        screen = ScreenBuffer()
        selection = nil
        needsDisplay = true
    }

    /// 描画中の画面にかかる選択・検索の一致・URL。
    private struct Marks {
        var selection: TermRange?
        var matches: [TermRange] = []
        var current: TermRange?
        var urls: [TermRange] = []
    }

    private static let matchColors: (fg: UInt32, bg: UInt32) = (0x000000, 0xF2D64B)
    private static let currentColors: (fg: UInt32, bg: UInt32) = (0x000000, 0xFF9632)

    private func visibleMarks(_ rows: [Row]) -> Marks {
        guard let firstLine = rows.first?.line, let lastLine = rows.last?.line else { return Marks() }
        let visible = { (r: TermRange) -> Bool in
            let (a, b) = r.ordered
            return b.line >= firstLine && a.line <= lastLine
        }
        var marks = Marks()
        if let selection, !selection.isEmpty { marks.selection = selection }
        marks.matches = matches.filter(visible)
        if let i = currentMatch, i < matches.count { marks.current = matches[i] }
        if copyPaste.clickableUrl, let session = textSession {
            if let cache = urlCache, cache.generation == screen.generation, cache.offset == screen.scrollOffset {
                marks.urls = cache.urls
            } else {
                let urls = session.urls(first: firstLine, last: lastLine).map(\.range)
                urlCache = (screen.generation, screen.scrollOffset, urls)
                marks.urls = urls
            }
        }
        return marks
    }

    /// セルの描画色（選択は前景と背景を入れ替え、検索の一致は塗る）。
    private func colors(_ cell: Cell, line: Int32, col: UInt16, _ marks: Marks) -> (fg: UInt32, bg: UInt32) {
        if let sel = marks.selection, sel.contains(line: line, col: col) { return (cell.bg, cell.fg) }
        if let cur = marks.current, cur.contains(line: line, col: col) { return Self.currentColors }
        if marks.matches.contains(where: { $0.contains(line: line, col: col) }) { return Self.matchColors }
        return (cell.fg, cell.bg)
    }

    private func drawRow(_ row: Row, at index: Int, in ctx: CGContext, marks: Marks) {
        let y = pad + CGFloat(index) * cellH
        let painted = row.cells.enumerated().map { c, cell in colors(cell, line: row.line, col: UInt16(c), marks) }
        // 背景：既定色以外の連続区間をまとめて塗る
        var col = 0
        while col < row.cells.count {
            let bg = painted[col].bg
            var end = col + 1
            while end < row.cells.count && painted[end].bg == bg { end += 1 }
            if bg != screen.defaultBg {
                ctx.setFillColor(color(bg))
                ctx.fill(CGRect(x: pad + CGFloat(col) * cellW, y: y, width: CGFloat(end - col) * cellW, height: cellH))
            }
            col = end
        }
        // 文字：セル位置に 1 文字ずつ置く（全角やフォールバックフォントでも桁がずれない）
        for (c, cell) in row.cells.enumerated() {
            let x = pad + CGFloat(c) * cellW
            drawCell(cell, x: x, y: y, fg: painted[c].fg, in: ctx)
            if marks.urls.contains(where: { $0.contains(line: row.line, col: UInt16(c)) }) {
                ctx.setFillColor(color(painted[c].fg))
                ctx.fill(CGRect(x: x, y: y + ascent + 2, width: cellW, height: 1))
            }
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
        if screen.cursor.blinking && focused && !blinkOn { return }

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
        // 打っている間は点滅させずに見せる
        blinkOn = true
        if screen.scrollToBottom() { scrollChanged() }
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
            let meta = KeyboardConfig.saved.metaSendsEscape
            let mods = KeyMapping.modifiers(flags, meta: meta)
            if let key = KeyMapping.specialKey(keyCode: event.keyCode) {
                send(key, mods)
                return
            }
            if flags.contains(.control), let chars = event.charactersIgnoringModifiers, chars.count == 1 {
                send(.text(text: chars), mods)
                return
            }
            // Option をメタキーにしているときは、合成した文字（π など）ではなく元のキーを ESC 付きで送る
            if mods.alt, let chars = event.charactersIgnoringModifiers, !chars.isEmpty {
                send(.text(text: chars), mods)
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

    // MARK: - コピーと貼り付け（編集メニュー）

    /// クリップボードの文字列を貼り付ける。`appendCR` なら末尾に改行を足す（「貼り付け<CR>」）。
    func pasteClipboard(appendCR: Bool = false) {
        guard var text = NSPasteboard.general.string(forType: .string) else { return }
        if copyPaste.trimTrailingNewline {
            while let last = text.last, last.isNewline { text.removeLast() }
        }
        requestPaste(text, appendCR: appendCR)
    }

    /// 貼り付けの要求。確認するかは onPaste の受け手が決める。
    func requestPaste(_ text: String, appendCR: Bool = false) {
        guard !text.isEmpty || appendCR else { return }
        if let onPaste {
            onPaste(text, appendCR)
        } else {
            sendPaste(appendCR ? text + "\r" : text)
        }
    }

    /// 確認を済ませた文字列を送る。
    func sendPaste(_ text: String) {
        if screen.scrollToBottom() { scrollChanged() }
        session?.paste(text: text)
    }

    @objc func paste(_ sender: Any?) {
        pasteClipboard()
    }

    @objc func pasteWithCR(_ sender: Any?) {
        pasteClipboard(appendCR: true)
    }

    var selectedText: String? {
        guard let session = textSession, let selection, !selection.isEmpty else { return nil }
        let text = session.selectionText(range: selection)
        return text.isEmpty ? nil : text
    }

    private func writeClipboard(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    @objc func copy(_ sender: Any?) {
        if let text = selectedText { writeClipboard(text) }
    }

    /// 「テーブルをコピー」：空白の並びをタブにしてコピーする（表計算に貼れる）。
    @objc func copyTable(_ sender: Any?) {
        if let text = selectedText { writeClipboard(tableToTsv(text: text)) }
    }

    @objc override func selectAll(_ sender: Any?) {
        guard let session = textSession else { return }
        selection = session.selectAll()
        needsDisplay = true
    }

    /// 「表示画面を選択」
    @objc func selectScreen(_ sender: Any?) {
        let top = Int32(-screen.scrollOffset)
        selection = TermRange(
            start: TermPoint(line: top, col: 0),
            end: TermPoint(line: top + Int32(screen.rows) - 1, col: gridSize.cols),
            block: false)
        needsDisplay = true
    }

    @objc func cancelSelection(_ sender: Any?) {
        selection = nil
        needsDisplay = true
    }

    @objc func clearScreen(_ sender: Any?) {
        selection = nil
        session?.clearScreen()
    }

    @objc func clearBuffer(_ sender: Any?) {
        selection = nil
        setMatches([], current: nil)
        session?.clearBuffer()
    }

    // MARK: - マウス

    /// マウスの位置 → セルの境目（選択の端）。
    private func boundary(_ event: NSEvent) -> TermPoint {
        let p = convert(event.locationInWindow, from: nil)
        let col = max(0, min(Int(gridSize.cols), Int(((p.x - pad) / cellW).rounded())))
        let row = max(0, min(screen.rows - 1, Int(((p.y - pad) / cellH).rounded(.down))))
        return TermPoint(line: Int32(row - screen.scrollOffset), col: UInt16(col))
    }

    /// マウスの位置 → その下のセル（単語や URL を調べる）。
    private func cell(_ event: NSEvent) -> TermPoint {
        let p = convert(event.locationInWindow, from: nil)
        let col = max(0, min(Int(gridSize.cols) - 1, Int(((p.x - pad) / cellW).rounded(.down))))
        let row = max(0, min(screen.rows - 1, Int(((p.y - pad) / cellH).rounded(.down))))
        return TermPoint(line: Int32(row - screen.scrollOffset), col: UInt16(col))
    }

    private func unitRange(at p: TermPoint) -> TermRange? {
        guard let session = textSession else { return nil }
        switch dragUnit {
        case .cell: return nil
        case .word: return session.wordAt(point: p, delimiters: copyPaste.delimiters)
        case .line: return session.lineAt(line: p.line)
        }
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        guard let session = textSession else { return }
        let here = cell(event)
        if event.modifierFlags.contains(.command), copyPaste.clickableUrl,
           let link = session.urlAt(point: here), let url = URL(string: link) {
            NSWorkspace.shared.open(url)
            return
        }
        switch event.clickCount {
        case 2:
            dragUnit = .word
        case 3...:
            dragUnit = .line
        default:
            dragUnit = .cell
        }
        if let range = unitRange(at: here) {
            dragAnchor = range
            selection = range
        } else if event.modifierFlags.contains(.shift), var sel = selection {
            sel.end = boundary(event)
            selection = sel
        } else {
            let p = boundary(event)
            dragAnchor = nil
            selection = TermRange(start: p, end: p, block: event.modifierFlags.contains(.option))
        }
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        guard selection != nil else { return }
        // 端より外へ引っぱったらスクロールする
        let y = convert(event.locationInWindow, from: nil).y
        if y < 0, screen.scroll(by: 1) { scrollChanged() }
        if y > bounds.height, screen.scroll(by: -1) { scrollChanged() }

        if let anchor = dragAnchor, let range = unitRange(at: cell(event)) {
            let (a0, a1) = anchor.ordered, (r0, r1) = range.ordered
            let before = (r0.line, r0.col) < (a0.line, a0.col)
            selection = before
                ? TermRange(start: a1, end: r0, block: false)
                : TermRange(start: a0, end: (r1.line, r1.col) > (a1.line, a1.col) ? r1 : a1, block: false)
        } else {
            selection?.end = boundary(event)
        }
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        guard let sel = selection else { return }
        if sel.isEmpty {
            selection = nil
            needsDisplay = true
        } else if copyPaste.autoCopy {
            copy(nil)
        }
    }

    override func rightMouseDown(with event: NSEvent) {
        if copyPaste.rightClickPaste {
            pasteClipboard()
        } else {
            super.rightMouseDown(with: event)
        }
    }

    override func otherMouseDown(with event: NSEvent) {
        if event.buttonNumber == 2 && copyPaste.middleClickPaste {
            pasteClipboard()
        } else {
            super.otherMouseDown(with: event)
        }
    }

    /// 右クリックのメニュー（右クリックで貼り付けるときは出さない）。
    override func menu(for event: NSEvent) -> NSMenu? {
        guard !copyPaste.rightClickPaste else { return nil }
        let menu = NSMenu()
        if copyPaste.clickableUrl, let link = textSession?.urlAt(point: cell(event)) {
            let open = NSMenuItem(title: L("URL を開く"), action: #selector(openLink(_:)), keyEquivalent: "")
            open.representedObject = link
            open.target = self
            menu.addItem(open)
            menu.addItem(.separator())
        }
        let items: [(String, Selector, Bool)] = [
            (TT.text("MENU_EDIT_COPY", "コピー"), #selector(copy(_:)), selectedText != nil),
            (TT.text("MENU_EDIT_COPYTABLE", "テーブルをコピー"), #selector(copyTable(_:)), selectedText != nil),
            (TT.text("MENU_EDIT_PASTE", "貼り付け"), #selector(paste(_:)), true),
            (TT.text("MENU_EDIT_PASTECR", "貼り付け<CR>"), #selector(pasteWithCR(_:)), true),
        ]
        for (title, action, enabled) in items {
            let item = NSMenuItem(title: title, action: enabled ? action : nil, keyEquivalent: "")
            item.target = self
            menu.addItem(item)
        }
        menu.addItem(.separator())
        let all = NSMenuItem(title: TT.text("MENU_EDIT_SELECTALL", "全て選択"), action: #selector(selectAll(_:)), keyEquivalent: "")
        all.target = self
        menu.addItem(all)
        return menu
    }

    @objc private func openLink(_ sender: NSMenuItem) {
        if let link = sender.representedObject as? String, let url = URL(string: link) {
            NSWorkspace.shared.open(url)
        }
    }

    override func resetCursorRects() {
        addCursorRect(NSRect(x: 0, y: 0, width: bounds.width - scrollerWidth, height: bounds.height), cursor: .iBeam)
    }

    // MARK: - スクロール

    override func scrollWheel(with event: NSEvent) {
        guard !screen.altScreen else { return }
        let lines = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY / cellH : event.scrollingDeltaY
        scrollRemainder += lines
        let whole = Int(scrollRemainder.rounded(.towardZero))
        scrollRemainder -= CGFloat(whole)
        if whole != 0 && screen.scroll(by: whole) { scrollChanged() }
    }

    private func scrollChanged() {
        needsDisplay = true
        updateScroller()
    }

    /// スクロールバー：つまみの大きさは画面の行数 / 全体、位置は下端が最新。
    private func updateScroller() {
        let history = screen.history
        let total = history + screen.rows
        scroller.isEnabled = history > 0 && !screen.altScreen
        scroller.knobProportion = total > 0 ? CGFloat(screen.rows) / CGFloat(total) : 1
        scroller.doubleValue = history > 0 ? 1 - Double(screen.scrollOffset) / Double(history) : 1
    }

    @objc private func scrollerMoved(_ sender: NSScroller) {
        let page = max(1, screen.rows - 1)
        var delta = 0
        switch sender.hitPart {
        case .knob, .knobSlot:
            let target = Int(((1 - sender.doubleValue) * Double(screen.history)).rounded())
            delta = target - screen.scrollOffset
        case .decrementPage: delta = page
        case .incrementPage: delta = -page
        case .decrementLine: delta = 1
        case .incrementLine: delta = -1
        default: break
        }
        if delta != 0 && screen.scroll(by: delta) { needsDisplay = true }
        updateScroller()
    }

    // MARK: - 検索

    /// 検索の一致を表示する。`current` の一致が見えるようにスクロールする。
    func setMatches(_ list: [TermRange], current: Int?) {
        matches = list
        currentMatch = current
        if let current, current < list.count {
            let line = Int(list[current].ordered.0.line)
            let top = -screen.scrollOffset, bottom = top + screen.rows - 1
            if line < top || line > bottom {
                let offset = max(0, min(screen.history, screen.rows / 2 - line))
                _ = screen.scroll(by: offset - screen.scrollOffset)
            }
        }
        scrollChanged()
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
