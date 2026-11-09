import Foundation
import ZettaCore

/// SSH の認証・ホスト鍵確認の問い合わせ 1 件。答えると `finish` が呼ばれ、接続処理が先へ進む。
@MainActor
final class AuthPrompt: Identifiable {
    enum Kind {
        case hostKey(HostKeyInfo)
        case credentials(AuthRequest)
        case keyboard(name: String, instructions: String, prompts: [KeyboardPrompt])
        case passphrase(keyPath: String, failed: Bool)
        case passwordChange(PasswordChangeRequest)
    }

    enum Answer {
        case hostKey(HostKeyDecision)
        case credentials(AuthResponse)
        case keyboard([String]?)
        case passphrase(String?)
        case passwordChange(PasswordChange?)
    }

    let id = UUID()
    let kind: Kind
    private var finish: ((Answer) -> Void)?

    init(_ kind: Kind, finish: @escaping (Answer) -> Void) {
        self.kind = kind
        self.finish = finish
    }

    var isAnswered: Bool { finish == nil }

    func answer(_ a: Answer) {
        finish?(a)
        finish = nil
    }

    /// 閉じられた・ウィンドウが消えたなど、答えずに終わるとき。
    func cancel() {
        switch kind {
        case .hostKey: answer(.hostKey(.reject))
        case .credentials: answer(.credentials(.cancel))
        case .keyboard: answer(.keyboard(nil))
        case .passphrase: answer(.passphrase(nil))
        case .passwordChange: answer(.passwordChange(nil))
        }
    }
}

/// 「パスワードをメモリ上に記憶する」。アプリを終えるまで覚えておく（ディスクには書かない）。
@MainActor
enum PasswordMemory {
    private static var passwords: [String: String] = [:]

    static func key(user: String, host: String, port: UInt16) -> String { "\(user)@\(host):\(port)" }
    static func get(_ key: String) -> String? { passwords[key] }
    static func set(_ key: String, _ password: String?) { passwords[key] = password }
}

/// コアからの問い合わせを受けて、ウィンドウにダイアログを出し、答えが出るまで待つ。
/// コアの接続処理のスレッドから呼ばれる（メインスレッドから接続を開いてはいけない）。
final class SshPrompterBridge: SshPrompter, @unchecked Sendable {
    /// 問い合わせを出す先。メインスレッドでだけ触る
    @MainActor weak var controller: TerminalController?

    private final class Box<T>: @unchecked Sendable {
        var value: T
        init(_ v: T) { value = v }
    }

    /// メインスレッドで `make` を呼んで問い合わせを出し、答えを待つ。出せなければ `fallback`。
    private func ask<T: Sendable>(
        _ fallback: T,
        _ make: @escaping @MainActor @Sendable (@escaping @Sendable (T) -> Void) -> AuthPrompt.Kind?,
        _ extract: @escaping @Sendable (AuthPrompt.Answer) -> T?
    ) -> T {
        let done = DispatchSemaphore(value: 0)
        let result = Box(fallback)
        let lock = NSLock()
        let deliver: @Sendable (T) -> Void = { v in
            lock.lock()
            result.value = v
            lock.unlock()
            done.signal()
        }
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                guard let controller = self.controller, let kind = make(deliver) else {
                    done.signal()
                    return
                }
                controller.present(AuthPrompt(kind) { answer in
                    deliver(extract(answer) ?? fallback)
                })
            }
        }
        done.wait()
        lock.lock()
        defer { lock.unlock() }
        return result.value
    }

    func hostKey(info: HostKeyInfo) -> HostKeyDecision {
        ask(.reject, { _ in .hostKey(info) }) {
            if case let .hostKey(d) = $0 { return d }
            return nil
        }
    }

    /// マクロ・起動引数の `/auth=` `/keyfile=` `/passwd=`。次の接続の最初の認証で 1 回だけ使い、
    /// 接続が終わったら消す（TTSSH の自動ログイン）
    private let oneTimeLock = NSLock()
    private var oneTimeLogin: LoginPreset?

    func setOneTimeLogin(_ preset: LoginPreset?) {
        oneTimeLock.lock()
        oneTimeLogin = preset
        oneTimeLock.unlock()
    }

    /// 決めてあるログインの仕方で答えられれば、その答え。答えられなければ鍵ファイルを入れた問い合わせ
    private func answerOneTimeLogin(_ request: AuthRequest) -> (AuthResponse?, AuthRequest) {
        oneTimeLock.lock()
        defer { oneTimeLock.unlock() }
        guard let preset = oneTimeLogin else { return (nil, request) }
        let a = answerLoginPreset(preset: preset, request: request)
        oneTimeLogin = a.preset
        return (a.response, a.request)
    }

    func credentials(request: AuthRequest) -> AuthResponse {
        let (preset, request) = answerOneTimeLogin(request)
        if let preset { return preset }
        return ask(.cancel, { deliver in
            // 記憶したパスワードがあれば、最初の 1 回はダイアログを出さずに使う
            let key = PasswordMemory.key(user: request.user, host: request.host, port: request.port)
            if !request.failed, !request.further, request.methods.contains(.password),
               let saved = PasswordMemory.get(key) {
                deliver(.password(user: request.user, password: saved))
                return nil
            }
            return .credentials(request)
        }) {
            if case let .credentials(r) = $0 { return r }
            return nil
        }
    }

    func keyboardInteractive(name: String, instructions: String, prompts: [KeyboardPrompt]) -> [String]? {
        ask(nil, { _ in .keyboard(name: name, instructions: instructions, prompts: prompts) }) {
            if case let .keyboard(a) = $0 { return a }
            return nil
        }
    }

    func passphrase(keyPath: String, failed: Bool) -> String? {
        ask(nil, { _ in .passphrase(keyPath: keyPath, failed: failed) }) {
            if case let .passphrase(p) = $0 { return p }
            return nil
        }
    }

    func passwordChange(request: PasswordChangeRequest) -> PasswordChange? {
        let change: PasswordChange? = ask(nil, { _ in .passwordChange(request) }) {
            if case let .passwordChange(c) = $0 { return c }
            return nil
        }
        // 「パスワードをメモリ上に記憶する」で覚えていたなら、新しいパスワードに置き換える
        if let change {
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    let key = PasswordMemory.key(user: request.user, host: request.host, port: request.port)
                    if PasswordMemory.get(key) != nil { PasswordMemory.set(key, change.newPassword) }
                }
            }
        }
        return change
    }
}
