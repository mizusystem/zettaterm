import MTCore
import XCTest

@testable import MultiTerm

final class ConnectTargetTests: XCTestCase {
    func testRoundTripsThroughCoreProfile() {
        var serial = SerialConfig()
        serial.baud = 115_200
        serial.parity = .even
        serial.delayPerLineMs = 10
        let targets: [ConnectTarget] = [
            .localShell,
            .ssh(host: "web", port: nil, user: nil),
            .ssh(host: "db", port: 2222, user: "kt"),
            .ssh(host: "legacy", port: nil, user: nil, systemSSH: true),
            .telnet(host: "bbs", port: 2323, ip: .v4),
            .tcp(host: "10.0.0.2", port: 7, ip: .v6),
            .serial(port: "/dev/cu.usbmodem1101", config: serial),
        ]
        for t in targets {
            XCTAssertEqual(ConnectTarget(t.profile), t)
        }
    }

    func testCodableForWindowRequests() throws {
        let r = ConnectRequest(target: .connect(.ssh(host: "web", port: 22, user: "u")), asTab: true)
        let decoded = try JSONDecoder().decode(ConnectRequest.self, from: JSONEncoder().encode(r))
        XCTAssertEqual(decoded, r)
    }

    func testLabels() {
        XCTAssertEqual(ConnectTarget.ssh(host: "h", port: 2222, user: "u").label, "u@h:2222 (SSH)")
        XCTAssertEqual(ConnectTarget.telnet(host: "bbs", port: nil, ip: .auto).label, "bbs (Telnet)")
        XCTAssertEqual(ConnectTarget.tcp(host: "h", port: 7, ip: .auto).label, "h:7")
        XCTAssertEqual(ConnectTarget.ssh(host: "h", port: nil, user: nil).host, "h")
        XCTAssertNil(ConnectTarget.localShell.host)
    }

    func testTTSSHStrings() {
        XCTAssertNotEqual(TT.text("DLG_HOST_TCPIPSERVICE", "__x__", section: "TTSSH"), "__x__")
        // セクションに無いキーは Tera Term セクションから引く
        XCTAssertEqual(TT.text("BTN_OK", "__x__", section: "TTSSH"), "OK")
    }
}
