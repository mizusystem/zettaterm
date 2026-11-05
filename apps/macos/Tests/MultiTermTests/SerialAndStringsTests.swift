import MTCore
import XCTest

@testable import MultiTerm

final class SerialAndStringsTests: XCTestCase {
    func testCleanRemovesWindowsAccelerators() {
        XCTAssertEqual(TT.clean("新しい接続(&N)...\\tAlt+N"), "新しい接続...")
        XCTAssertEqual(TT.clean("T&CP/IP..."), "TCP/IP...")
        XCTAssertEqual(TT.clean("ポート(&P):"), "ポート:")
        XCTAssertEqual(TT.clean("Tera Term: 新しい接続"), "新しい接続")
        XCTAssertEqual(TT.clean("A && B"), "A & B")
    }

    func testStringsComeFromTeraTermSpec() {
        // spec/lang から読めていること（キーが無ければ fallback が返る）
        XCTAssertNotEqual(TT.text("DLG_SERIAL_OK_RESET", "__missing__"), "__missing__")
        XCTAssertEqual(TT.text("NO_SUCH_KEY", "fallback"), "fallback")
        XCTAssertEqual(TT.options("parity"), ["none", "odd", "even", "mark", "space"])
        XCTAssertTrue(SerialConfig.baudPresets.contains(115_200))
    }

    func testOptionLabelsFollowTeraTermOrder() {
        XCTAssertEqual(SerialConfig.label(.odd), "odd")
        XCTAssertEqual(SerialConfig.label(.rtsCts), "RTS/CTS")
        XCTAssertEqual(SerialConfig.label(.none as SerialConfig.FlowChoice), "NONE")
        XCTAssertEqual(SerialConfig.label(.two), "2 bit")
        XCTAssertEqual(SerialConfig.dataLabel(7), "7 bit")
    }

    func testConfigConvertsToCoreSettings() throws {
        var c = SerialConfig()
        c.baud = 250_000
        c.dataBits = 7
        c.parity = .even
        c.stopBits = .two
        c.flow = .rtsCts
        c.delayPerCharMs = 5
        let s = c.settings
        XCTAssertEqual(s.baud, 250_000)
        XCTAssertEqual(s.dataBits, 7)
        XCTAssertEqual(s.parity, .even)
        XCTAssertEqual(s.stopBits, .two)
        XCTAssertEqual(s.flow, .rtsCts)
        XCTAssertEqual(s.delayPerCharMs, 5)
        XCTAssertEqual(c.summary, "250000bps 7E2")

        let decoded = try JSONDecoder().decode(SerialConfig.self, from: JSONEncoder().encode(c))
        XCTAssertEqual(decoded, c)
    }

    func testDefaultMatchesCore() {
        XCTAssertEqual(SerialConfig().settings, defaultSerialSettings())
    }
}

final class TerminalConfigTests: XCTestCase {
    func testNewlineLabelsComeFromTeraTerm() {
        XCTAssertEqual(TerminalConfig.Receive.allCases.map(TerminalConfig.label), ["CR", "CR+LF", "LF", "AUTO"])
        XCTAssertEqual(TerminalConfig.Send.allCases.map(TerminalConfig.label), ["CR", "CR+LF", "LF"])
    }

    func testConvertsToCoreSettings() {
        let c = TerminalConfig(receive: .auto, send: .crLf, localEcho: true)
        XCTAssertEqual(c.settings, TerminalSettings(newlineReceive: .auto, newlineSend: .crLf, localEcho: true,
                                                    charsetReceive: .utf8, charsetSend: .utf8))
        let sjis = TerminalConfig(charsetReceive: .shiftJis, charsetSend: .eucJp)
        XCTAssertEqual(sjis.settings.charsetReceive, .shiftJis)
        XCTAssertEqual(sjis.settings.charsetSend, .eucJp)
        XCTAssertEqual(TerminalConfig().settings, defaultTerminalSettings())
    }

    func testOldSavedSettingsWithoutCharsetStillDecode() throws {
        let old = #"{"receive":"auto","send":"cr","localEcho":false}"#.data(using: .utf8)!
        let c = try JSONDecoder().decode(TerminalConfig.self, from: old)
        XCTAssertEqual(c.receive, .auto)
        XCTAssertEqual(c.charsetReceive, .utf8)
    }

    func testScopeSeparatesSerialFromOthers() {
        XCTAssertEqual(TerminalConfig.scope(for: .serial(port: "/dev/cu.x", config: SerialConfig())), .serial)
        XCTAssertEqual(TerminalConfig.scope(for: .ssh(host: "h", port: nil, user: nil)), .other)
        XCTAssertEqual(TerminalConfig.scope(for: nil), .other)
    }
}
