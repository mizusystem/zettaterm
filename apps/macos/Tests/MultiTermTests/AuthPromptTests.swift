import MTCore
import XCTest

@testable import MultiTerm

@MainActor
final class AuthPromptTests: XCTestCase {
    func testCancelAnswersWithTheSafeDefault() {
        var got: AuthPrompt.Answer?
        let info = HostKeyInfo(host: "h", port: 22, algorithm: "ssh-ed25519", fingerprint: "SHA256:x", status: .unknown)
        let p = AuthPrompt(.hostKey(info)) { got = $0 }
        XCTAssertFalse(p.isAnswered)
        p.cancel()
        XCTAssertTrue(p.isAnswered)
        guard case .hostKey(.reject) = got else { return XCTFail("\(String(describing: got))") }
        // 2 回目の答えは無視する
        p.answer(.hostKey(.acceptAndSave))
        guard case .hostKey(.reject) = got else { return XCTFail("answered twice") }
    }

    func testCancelForEachKind() {
        let request = AuthRequest(host: "h", port: 22, user: "u", methods: [.password], defaultKey: nil,
                                  failed: false, further: false, banner: nil)
        var answers: [AuthPrompt.Answer] = []
        AuthPrompt(.credentials(request)) { answers.append($0) }.cancel()
        AuthPrompt(.keyboard(name: "", instructions: "", prompts: [])) { answers.append($0) }.cancel()
        AuthPrompt(.passphrase(keyPath: "/k", failed: false)) { answers.append($0) }.cancel()
        guard case .credentials(.cancel) = answers[0],
              case .keyboard(nil) = answers[1],
              case .passphrase(nil) = answers[2]
        else { return XCTFail("\(answers)") }
    }

    func testPasswordMemoryIsKeyedByUserHostAndPort() {
        let k = PasswordMemory.key(user: "kt", host: "web", port: 2222)
        XCTAssertEqual(k, "kt@web:2222")
        PasswordMemory.set(k, "pw")
        XCTAssertEqual(PasswordMemory.get(k), "pw")
        PasswordMemory.set(k, nil)
        XCTAssertNil(PasswordMemory.get(k))
    }

    func testCoreMessagesAreLocalized() {
        // 表示言語（テストの実行環境では英語になることがある）によらず、対応表の文に置き換わる
        XCTAssertTrue(TerminalController.localized("authentication canceled").hasPrefix(L("認証を中止しました。")))
        XCTAssertTrue(TerminalController.localized("10.0.0.1: connection timed out")
            .hasPrefix(TT.text("MSG_COMM_TIMEOUT_ERROR", "接続がタイムアウトしました")))
        // OS の文は、Tera Term の文言に対象（ポート）を入れて出し、元の文を添える
        let busy = TerminalController.localized("Resource busy", subject: "/dev/cu.usbserial-1")
        XCTAssertTrue(busy.hasPrefix(TT.text("MSG_CANTOPEN_ERROR_DENIED", "").replacingOccurrences(of: "%s", with: "/dev/cu.usbserial-1")))
        XCTAssertTrue(busy.hasSuffix("（Resource busy）"))
        XCTAssertTrue(TerminalController.localized("Connection refused (os error 61)")
            .hasPrefix(TT.text("MSG_COMM_REFUSE_ERROR", "")))
        XCTAssertNotEqual(L("認証を中止しました。"), "authentication canceled")
        XCTAssertEqual(TerminalController.localized("something else"), "something else")
    }

    func testLngNewlinesBecomeRealNewlines() {
        XCTAssertEqual(TT.clean("a\\nb"), "a\nb")
    }
}
