import MTCore
import XCTest

@testable import MultiTerm

final class ScreenBufferTests: XCTestCase {
    private func row(_ line: Int32, _ text: String) -> Row {
        Row(line: line, cells: text.map { Cell(text: String($0), fg: 0xFFFFFF, bg: 0, flags: 0) }, wrapped: false)
    }

    private func snapshot(gen: UInt64, rows: UInt16, full: Bool, lines: [Row], history: UInt32 = 0,
                          cursor: Cursor = Cursor(row: 0, col: 0, shape: .block)) -> GridSnapshot {
        GridSnapshot(generation: gen, cols: 10, rows: rows, full: full, lines: lines, cursor: cursor,
                     history: history, altScreen: false, defaultFg: 0xFFFFFF, defaultBg: 0, cursorColor: 0xCCCCCC)
    }

    func testFullSnapshotReplacesAllLines() {
        var s = ScreenBuffer()
        let changed = s.apply(snapshot(gen: 1, rows: 3, full: true, lines: [row(0, "a"), row(1, "b"), row(2, "c")]))
        XCTAssertNil(changed)
        XCTAssertEqual(s.rows, 3)
        XCTAssertEqual(s.lines[1].cells.first?.text, "b")
        XCTAssertEqual(s.generation, 1)
    }

    func testPartialSnapshotReportsChangedRowsAndCursor() {
        var s = ScreenBuffer()
        s.apply(snapshot(gen: 1, rows: 3, full: true, lines: [row(0, "a"), row(1, "b"), row(2, "c")]))
        let changed = s.apply(snapshot(gen: 2, rows: 3, full: false, lines: [row(2, "z")],
                                       cursor: Cursor(row: 1, col: 0, shape: .block)))
        XCTAssertEqual(changed, [0, 1, 2])
        XCTAssertEqual(s.lines[2].cells.first?.text, "z")
        XCTAssertEqual(s.lines[0].cells.first?.text, "a")
    }

    func testScrollIsClampedAndKeepsPositionWhenHistoryGrows() {
        var s = ScreenBuffer()
        s.apply(snapshot(gen: 1, rows: 2, full: true, lines: [row(0, "a"), row(1, "b")], history: 5))
        XCTAssertTrue(s.scroll(by: 3))
        XCTAssertEqual(s.scrollOffset, 3)
        XCTAssertTrue(s.scroll(by: 100))
        XCTAssertEqual(s.scrollOffset, 5)
        XCTAssertFalse(s.scroll(by: 1))
        s.apply(snapshot(gen: 2, rows: 2, full: false, lines: [], history: 8))
        XCTAssertEqual(s.scrollOffset, 8)
        XCTAssertTrue(s.scrollToBottom())
        XCTAssertEqual(s.scrollOffset, 0)
    }

    func testGridGeometry() {
        let g = GridGeometry(cellWidth: 8, cellHeight: 16, padding: 4)
        XCTAssertEqual(g.size(width: 8 * 80 + 8, height: 16 * 24 + 8).cols, 80)
        XCTAssertEqual(g.size(width: 8 * 80 + 8, height: 16 * 24 + 8).rows, 24)
        XCTAssertEqual(g.size(width: 0, height: 0).cols, 1)
    }

    func testKeyMapping() {
        XCTAssertEqual(KeyMapping.specialKey(keyCode: 126), .up)
        XCTAssertEqual(KeyMapping.specialKey(keyCode: 122), .function(number: 1))
        XCTAssertNil(KeyMapping.specialKey(keyCode: 0))
        XCTAssertEqual(KeyMapping.key(forCommand: #selector(NSResponder.insertNewline(_:))), .enter)
    }
}
