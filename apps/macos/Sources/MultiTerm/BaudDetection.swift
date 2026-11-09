import Foundation
import MTCore

/// 通信速度の自動検出（Tera Term には無い追加）。「新しい接続」のシリアルで、接続する前に使う。
/// 速度を変えながら受信し、文字として読める速度を探す（コアの `BaudDetector`）。
@MainActor
final class BaudDetection: ObservableObject {
    @Published private(set) var running = false
    /// 進み具合と結果の表示
    @Published private(set) var status: String?
    /// 見つかった速度（見つかるたびに変わる）
    @Published private(set) var found: UInt32?
    private var detector: BaudDetector?
    private var timer: Timer?
    private var canceled = false

    /// `poke` は各速度の初めに CR を送るか（既定はオフ。1 文字のコマンドを解釈する機器があるため）。
    func start(port: String, poke: Bool, waitSeconds: Int) {
        guard !running, !port.isEmpty else { return }
        let detector = BaudDetector()
        self.detector = detector
        running = true
        canceled = false
        found = nil
        status = L("調べています…")
        timer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            let baud = detector.trying()
            MainActor.assumeIsolated {
                if baud > 0, self?.running == true { self?.status = L("%@ bps を調べています…", String(baud)) }
            }
        }
        let settings = SerialConfig.saved.settings
        Task {
            let result: Result<DetectedBaud?, Error> = await Task.detached {
                Result { try detector.run(port: port, settings: settings, waitMs: UInt32(max(1, waitSeconds)) * 1000, poke: poke) }
            }.value
            timer?.invalidate()
            timer = nil
            self.detector = nil
            running = false
            switch result {
            case let .success(d?):
                found = d.baud
                status = L("%@ bps で読めました：%@", String(d.baud), Self.preview(d.sample))
            case .success(nil):
                status = canceled
                    ? L("中止しました。")
                    : L("読める速度が見つかりませんでした。相手が何か送るとき（起動の直後など）にもう一度試すか、「Enter を送って調べる」をオンにしてください。")
            case let .failure(e):
                status = TerminalController.describe(e, subject: port)
            }
        }
    }

    func cancel() {
        canceled = true
        detector?.cancel()
    }

    /// 見本の最後の読める行（プロンプトが来ることが多い。長ければ切る）
    private static func preview(_ sample: String) -> String {
        let line = sample.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .last { $0.contains(where: \.isLetter) } ?? ""
        return line.count > 40 ? String(line.prefix(40)) + "…" : line
    }
}
