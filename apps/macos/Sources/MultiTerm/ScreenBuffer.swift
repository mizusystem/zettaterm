import MTCore

/// コアから取った画面の写し。snapshot の差分を当て、スクロール位置を管理する。
/// 描画やイベントとは独立させてテストできるようにしている。
struct ScreenBuffer {
    private(set) var lines: [Row] = []
    private(set) var generation: UInt64 = 0
    private(set) var cursor = Cursor(row: 0, col: 0, shape: .block, blinking: false)
    private(set) var history: Int = 0
    private(set) var altScreen = false
    private(set) var defaultFg: UInt32 = 0xE5E5E5
    private(set) var defaultBg: UInt32 = 0x000000
    private(set) var cursorColor: UInt32 = 0xC0C0C0
    /// 画面の下端から何行さかのぼって表示しているか。0 なら最新。
    private(set) var scrollOffset: Int = 0

    var rows: Int { lines.count }

    /// snapshot を当てる。戻り値は書き換わった画面上の行番号（全行なら nil）。
    @discardableResult
    mutating func apply(_ s: GridSnapshot) -> Set<Int>? {
        let rowCount = Int(s.rows)
        var changed: Set<Int>? = []
        if s.full || lines.count != rowCount {
            lines = (0..<rowCount).map { Row(line: Int32($0), cells: [], wrapped: false) }
            changed = nil
        }
        for row in s.lines {
            let i = Int(row.line)
            if i >= 0 && i < rowCount {
                lines[i] = row
                changed?.insert(i)
            }
        }

        // さかのぼって見ている間に出力が増えても、見ている内容がずれないようにする
        let newHistory = Int(s.history)
        if scrollOffset > 0 && newHistory > history {
            scrollOffset += newHistory - history
        }
        history = newHistory
        scrollOffset = min(scrollOffset, history)

        if let c = changed, c.isEmpty == false || cursor != s.cursor {
            var withCursor = c
            withCursor.insert(Int(cursor.row))
            withCursor.insert(Int(s.cursor.row))
            changed = withCursor
        }
        cursor = s.cursor
        generation = s.generation
        altScreen = s.altScreen
        defaultFg = s.defaultFg
        defaultBg = s.defaultBg
        cursorColor = s.cursorColor
        return changed
    }

    /// スクロール位置を `delta` 行動かす（正でさかのぼる）。動いたら true。
    mutating func scroll(by delta: Int) -> Bool {
        let next = max(0, min(history, scrollOffset + delta))
        defer { scrollOffset = next }
        return next != scrollOffset
    }

    mutating func scrollToBottom() -> Bool {
        defer { scrollOffset = 0 }
        return scrollOffset != 0
    }
}

/// フォントのセル寸法から、ビューの大きさに入る行数・列数を求める。
struct GridGeometry: Equatable {
    var cellWidth: Double
    var cellHeight: Double
    var padding: Double

    func size(width: Double, height: Double) -> (cols: UInt16, rows: UInt16) {
        let cols = Int(((width - padding * 2) / cellWidth).rounded(.down))
        let rows = Int(((height - padding * 2) / cellHeight).rounded(.down))
        return (UInt16(clamping: max(cols, 1)), UInt16(clamping: max(rows, 1)))
    }
}
