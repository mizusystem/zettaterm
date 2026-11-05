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
        XCTAssertEqual(TerminalController.localized("authentication canceled"), "認証を中止しました。")
        XCTAssertTrue(TerminalController.localized("10.0.0.1: connection timed out").hasPrefix("接続がタイムアウトしました。"))
        XCTAssertEqual(TerminalController.localized("something else"), "something else")
    }

    func testLngNewlinesBecomeRealNewlines() {
        XCTAssertEqual(TT.clean("a\\nb"), "a\nb")
    }
}
