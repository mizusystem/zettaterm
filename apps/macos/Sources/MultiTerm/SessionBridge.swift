import Foundation
import MTCore

/// コアからの通知を受ける。コアのスレッドから呼ばれる。
///
/// `onDamage` はフラグを立てるだけ。端末ビューが displayLink のたびにフラグを見て
/// snapshot を取りに行くので、メインスレッドへの dispatch は発生しない。
final class SessionBridge: SessionListener, @unchecked Sendable {
    private let lock = NSLock()
    private var dirty = true

    private let handleState: @Sendable (SessionState) -> Void
    private let handleTitle: @Sendable (String) -> Void
    private let handleBell: @Sendable () -> Void

    init(
        onState: @escaping @Sendable (SessionState) -> Void,
        onTitle: @escaping @Sendable (String) -> Void,
        onBell: @escaping @Sendable () -> Void
    ) {
        handleState = onState
        handleTitle = onTitle
        handleBell = onBell
    }

    /// 前回から再描画が必要になったかを返し、フラグを下ろす。
    func takeDirty() -> Bool {
        lock.lock()
        defer { lock.unlock() }
        let d = dirty
        dirty = false
        return d
    }

    func onDamage() {
        lock.lock()
        dirty = true
        lock.unlock()
    }

    func onState(state: SessionState) {
        let handler = handleState
        DispatchQueue.main.async { handler(state) }
    }

    func onBell() {
        let handler = handleBell
        DispatchQueue.main.async { handler() }
    }

    func onTitle(title: String) {
        let handler = handleTitle
        DispatchQueue.main.async { handler(title) }
    }
}
