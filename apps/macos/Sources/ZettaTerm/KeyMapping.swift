import AppKit
import ZettaCore

enum KeyMapping {
    /// IME を通さずに直接送るキー（矢印、ファンクションキーなど）。該当しなければ nil。
    static func specialKey(keyCode: UInt16) -> Key? {
        switch keyCode {
        case 36, 76: return .enter          // Return, テンキー Enter
        case 48: return .tab
        case 51: return .backspace          // delete（⌫）
        case 53: return .escape
        case 117: return .delete            // forward delete（⌦）
        case 114: return .insert            // Help / Insert
        case 123: return .left
        case 124: return .right
        case 125: return .down
        case 126: return .up
        case 115: return .home
        case 119: return .end
        case 116: return .pageUp
        case 121: return .pageDown
        case 122: return .function(number: 1)
        case 120: return .function(number: 2)
        case 99: return .function(number: 3)
        case 118: return .function(number: 4)
        case 96: return .function(number: 5)
        case 97: return .function(number: 6)
        case 98: return .function(number: 7)
        case 100: return .function(number: 8)
        case 101: return .function(number: 9)
        case 109: return .function(number: 10)
        case 103: return .function(number: 11)
        case 111: return .function(number: 12)
        default: return nil
        }
    }

    /// `meta` は「Option キーをメタキーとして使う」。オフなら Option は Mac の特殊文字入力に使う
    static func modifiers(_ flags: NSEvent.ModifierFlags, meta: Bool = false) -> Modifiers {
        Modifiers(shift: flags.contains(.shift), ctrl: flags.contains(.control),
                  alt: meta && flags.contains(.option))
    }

    /// `interpretKeyEvents` から来るコマンドを端末のキーに変換する。
    static func key(forCommand selector: Selector) -> Key? {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)): return .enter
        case #selector(NSResponder.insertTab(_:)): return .tab
        case #selector(NSResponder.insertBacktab(_:)): return .tab
        case #selector(NSResponder.deleteBackward(_:)): return .backspace
        case #selector(NSResponder.deleteForward(_:)): return .delete
        case #selector(NSResponder.cancelOperation(_:)): return .escape
        case #selector(NSResponder.moveUp(_:)): return .up
        case #selector(NSResponder.moveDown(_:)): return .down
        case #selector(NSResponder.moveLeft(_:)): return .left
        case #selector(NSResponder.moveRight(_:)): return .right
        default: return nil
        }
    }
}
