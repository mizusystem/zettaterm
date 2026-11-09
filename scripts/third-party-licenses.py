#!/usr/bin/env python3
"""配るバイナリに組み込まれる依存クレートのライセンス表示を 1 つのテキストにまとめる。

使い方: python3 scripts/third-party-licenses.py <パッケージ名> [ターゲット] > THIRD_PARTY_LICENSES.txt

`cargo metadata` で、そのパッケージから通常の依存（build / dev は除く）でたどれるクレートを集め、
各クレートのソースにある LICENSE* / COPYING* / NOTICE* をそのまま並べる。ファイルが無いクレートは
Cargo.toml の license（SPDX）だけを書く。ワークスペース内のクレートと vendor/ の写しは
このリポジトリの LICENSE と THIRD_PARTY.md が受け持つので、ここでは vendor の元の表示だけ載せる。
"""

import json
import os
import subprocess
import sys

NAMES = ("license", "licence", "copying", "notice", "unlicense", "copyright")


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    root_name = sys.argv[1]
    cmd = ["cargo", "metadata", "--format-version", "1", "--locked"]
    if len(sys.argv) > 2:
        cmd += ["--filter-platform", sys.argv[2]]
    meta = json.loads(subprocess.check_output(cmd))
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    workspace = set(meta["workspace_members"])
    root = next(p["id"] for p in meta["packages"] if p["name"] == root_name and p["id"] in workspace)

    # 通常の依存だけをたどる
    seen, stack = set(), [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] in (None, "normal") for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])

    out = sys.stdout
    out.write("Third-party software included in this program\n")
    out.write("=" * 46 + "\n\n")
    out.write(
        "The program is built from the ZettaTerm sources (MIT OR Apache-2.0, see LICENSE-MIT and\n"
        "LICENSE-APACHE) and the crates listed below, each under its own license. Crates under\n"
        "MPL-2.0 are used unmodified; their source code is available from https://crates.io/.\n\n"
    )
    missing = False
    for pid in sorted(seen, key=lambda i: (packages[i]["name"], packages[i]["version"])):
        p = packages[pid]
        if pid in workspace and not p["manifest_path"].replace("\\", "/").count("/vendor/"):
            continue
        out.write("-" * 78 + "\n")
        out.write(f"{p['name']} {p['version']}  ({p.get('license') or 'see below'})\n")
        if p.get("repository"):
            out.write(f"{p['repository']}\n")
        out.write("-" * 78 + "\n")
        folder = os.path.dirname(p["manifest_path"])
        files = sorted(
            f for f in os.listdir(folder)
            if f.lower().startswith(NAMES) and os.path.isfile(os.path.join(folder, f))
        )
        if not files:
            out.write(
                f"(No license file in the package. License: {p.get('license') or 'unknown'};"
                " the full text is at the end of this file.)\n\n"
            )
            missing = True
            continue
        for f in files:
            with open(os.path.join(folder, f), encoding="utf-8", errors="replace") as fh:
                out.write(f"[{f}]\n{fh.read().rstrip()}\n\n")
    if missing:
        here = os.path.dirname(os.path.abspath(__file__))
        apache = os.path.join(here, "..", "vendor", "russh", "LICENSE-APACHE")
        out.write("=" * 78 + "\nApache License 2.0 (full text)\n" + "=" * 78 + "\n")
        with open(apache, encoding="utf-8") as fh:
            out.write(fh.read().rstrip() + "\n\n")
        out.write("=" * 78 + "\nMIT License (full text; the copyright line is the crate's own)\n" + "=" * 78 + "\n")
        out.write(MIT)
    return 0


MIT = """Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
"""


if __name__ == "__main__":
    sys.exit(main())
