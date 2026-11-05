#!/usr/bin/env python3
"""Tera Term のソースから UI 文言・設定キー・選択肢を JSON に抜き出す。

    python3 -I tools/extract-tt-ui/extract.py <teraterm のチェックアウト> spec/

出力（すべて UTF-8、キー順は安定）
    spec/SOURCE.json            元にしたコミットと日付
    spec/lang/<locale>.json     .lng の全セクション（{"Tera Term": {"MENU_FILE": "ファイル(&F)", ...}, ...}）
    spec/teraterm-ini.json      TERATERM.INI のセクション・キー・既定値
    spec/serial-options.json    シリアル設定の選択肢（serial_pp.cpp の *List[]）
    spec/terminal-options.json  端末設定の選択肢（term_pp.cpp の *List[]。改行コードなど）

手で書き写さない。入力が変わったらこのスクリプトを回し直す。
標準ライブラリだけで動く。
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path


def read_text(path: Path) -> str:
    """BOM 付き UTF-8 / Shift_JIS の混在を吸収して読む。"""
    data = path.read_bytes()
    for enc in ("utf-8-sig", "cp932"):
        try:
            return data.decode(enc)
        except UnicodeDecodeError:
            continue
    return data.decode("utf-8", errors="replace")


def parse_ini(text: str, comment_prefixes=(";",)) -> dict[str, dict[str, str]]:
    """重複キーは後勝ち。値の前後の空白は落とす。"""
    sections: dict[str, dict[str, str]] = {}
    current: dict[str, str] | None = None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith(comment_prefixes):
            continue
        if line.startswith("[") and line.endswith("]"):
            current = sections.setdefault(line[1:-1].strip(), {})
            continue
        if current is None or "=" not in line:
            continue
        key, value = line.split("=", 1)
        current[key.strip()] = value.strip()
    return sections


def extract_lang(root: Path) -> dict[str, dict]:
    lang_dir = root / "installer/release/lang_utf8"
    out = {}
    for path in sorted(lang_dir.glob("*.lng")):
        if path.stem == "Default":
            continue
        out[path.stem] = parse_ini(read_text(path))
    return out


def extract_ini(root: Path) -> dict[str, dict[str, str]]:
    return parse_ini(read_text(root / "installer/release/TERATERM.INI"))


LIST_RE = re.compile(r"static\s+const\s+char\s*\*\s*(\w*List\w*)\[\]\s*=\s*\{(.*?)\};", re.S)
STR_RE = re.compile(r'"((?:[^"\\]|\\.)*)"')


def extract_options(root: Path, source: str) -> dict[str, list[str]]:
    src = read_text(root / "teraterm/teraterm" / source)
    out = {}
    for name, body in LIST_RE.findall(src):
        out[option_key(name)] = STR_RE.findall(body)
    return out


def option_key(name: str) -> str:
    """BaudList -> baud、NLList -> nl、NLListRcv -> nlRcv。"""
    key = name.replace("List", "")
    run = len(key) - len(key.lstrip("ABCDEFGHIJKLMNOPQRSTUVWXYZ"))
    if run > 1 and run < len(key):
        run -= 1  # 次の単語の頭文字は大文字のまま残す
    return key[:run].lower() + key[run:]


def source_info(root: Path) -> dict[str, str]:
    try:
        line = subprocess.run(
            ["git", "-C", str(root), "log", "-1", "--format=%H %cs"],
            check=True, capture_output=True, text=True,
        ).stdout.strip()
        commit, date = line.split(" ", 1)
    except (subprocess.CalledProcessError, FileNotFoundError, ValueError):
        commit, date = "unknown", "unknown"
    return {
        "repository": "https://github.com/TeraTermProject/teraterm",
        "commit": commit,
        "date": date,
        "license": "BSD-3-Clause (see THIRD_PARTY.md)",
    }


def write_json(path: Path, data) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(data, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    path.write_text(text, encoding="utf-8")


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    root, out = Path(argv[1]), Path(argv[2])

    langs = extract_lang(root)
    for locale, sections in langs.items():
        write_json(out / "lang" / f"{locale}.json", sections)
    write_json(out / "teraterm-ini.json", extract_ini(root))
    write_json(out / "serial-options.json", extract_options(root, "serial_pp.cpp"))
    write_json(out / "terminal-options.json", extract_options(root, "term_pp.cpp"))
    write_json(out / "SOURCE.json", source_info(root))

    print(f"wrote {len(langs)} languages, TERATERM.INI and option lists to {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
