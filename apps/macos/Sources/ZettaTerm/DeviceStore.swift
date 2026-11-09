import Foundation
import ZettaCore

/// シリアルデバイスの一覧。抜き差しに追従する（コアの DeviceWatcher）。アプリ全体で 1 つ。
@MainActor
final class DeviceStore: ObservableObject {
    static let shared = DeviceStore()

    @Published private(set) var devices: [SerialDevice] = []
    private var watcher: DeviceWatcher?

    private init() {
        devices = listSerialDevices()
        let bridge = DeviceBridge { [weak self] list in
            Task { @MainActor in self?.devices = list }
        }
        watcher = try? DeviceWatcher(listener: bridge)
    }

    func contains(_ path: String) -> Bool {
        devices.contains { $0.path == path }
    }
}

/// コアのスレッドから呼ばれる。
private final class DeviceBridge: DeviceListener, @unchecked Sendable {
    private let handler: @Sendable ([SerialDevice]) -> Void

    init(_ handler: @escaping @Sendable ([SerialDevice]) -> Void) {
        self.handler = handler
    }

    func onDevicesChanged(devices: [SerialDevice]) {
        handler(devices)
    }
}
