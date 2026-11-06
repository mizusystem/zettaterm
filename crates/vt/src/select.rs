//! 選択範囲の文字列化、単語・行の選択、検索、URL の検出、表のコピー。
//!
//! UI は選択の両端（[`Point`]）だけを持ち、文字列や範囲の計算はここで行う。
//! 行は [`Row`] の列で受け取るので、画面とスクロールバックを区別しない。

use regex::{Regex, RegexBuilder};

use crate::types::{CellFlags, Row};

/// 端末上の位置。`line` は 0 が画面の最上行、負の値はスクロールバック。
/// `col` はセルの境目（0 なら行頭、`cols` なら行末）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub line: i32,
    pub col: u16,
}

impl Point {
    pub fn new(line: i32, col: u16) -> Self {
        Self { line, col }
    }
}

/// 選択範囲。`start` と `end` はどちらが先でもよい。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Selection {
    pub start: Point,
    pub end: Point,
    /// 矩形選択（Tera Term の Alt + ドラッグ）
    pub block: bool,
}

impl Selection {
    /// 読む順に並べた両端。
    pub fn ordered(&self) -> (Point, Point) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }

    /// 矩形選択の列の範囲（左端, 右端）。
    pub fn columns(&self) -> (u16, u16) {
        (
            self.start.col.min(self.end.col),
            self.start.col.max(self.end.col),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end || (self.block && self.start.col == self.end.col)
    }

    /// セル `(line, col)` が選択に含まれるか（描画用）。
    pub fn contains(&self, line: i32, col: u16) -> bool {
        let (a, b) = self.ordered();
        if line < a.line || line > b.line {
            return false;
        }
        if self.block {
            let (l, r) = self.columns();
            return col >= l && col < r;
        }
        let after_start = line > a.line || col >= a.col;
        let before_end = line < b.line || col < b.col;
        after_start && before_end
    }
}

/// Tera Term の DelimList に当たる、ダブルクリックで単語を区切る文字の既定値。
/// ネットワーク機器のアドレスやパスを一度で選べるよう、`. / - _ :` は区切りにしない。
pub const DEFAULT_DELIMITERS: &str = " \t!\"#$&'()*+,;<=>?@[\\]^`{|}~";

/// 行のセルの範囲 `[from, to)` を文字列にする。全角の右半分は飛ばす。
fn cells_text(row: &Row, from: usize, to: usize) -> String {
    row.cells
        .iter()
        .skip(from)
        .take(to.saturating_sub(from))
        .map(|c| c.text.as_str())
        .collect()
}

/// 全角文字の途中を指す境目を、文字の左端へ寄せる。
fn snap_left(row: &Row, col: u16) -> u16 {
    let c = usize::from(col);
    if c > 0 && c < row.cells.len() && row.cells[c].flags & CellFlags::WIDE_SPACER != 0 {
        col - 1
    } else {
        col
    }
}

/// 全角文字の途中を指す境目を、文字の右端へ寄せる。
fn snap_right(row: &Row, col: u16) -> u16 {
    let c = usize::from(col);
    if c > 0 && c < row.cells.len() && row.cells[c].flags & CellFlags::WIDE_SPACER != 0 {
        col + 1
    } else {
        col
    }
}

/// 選択範囲の文字列。行末の空白と末尾の空行は除き、折り返しで続く行は改行せずにつなぐ。
/// `rows` は選択にかかる行を含んでいればよい（順不同、余分があってもよい）。
pub fn selection_text(rows: &[Row], sel: &Selection) -> String {
    if sel.is_empty() {
        return String::new();
    }
    let (a, b) = sel.ordered();
    let mut out = String::new();
    for line in a.line..=b.line {
        let Some(row) = rows.iter().find(|r| r.line == line) else {
            continue;
        };
        let width = row.cells.len() as u16;
        let (from, to) = if sel.block {
            let (l, r) = sel.columns();
            (l, r)
        } else {
            (
                if line == a.line { a.col } else { 0 },
                if line == b.line { b.col } else { width },
            )
        };
        let from = snap_left(row, from.min(width));
        let to = snap_right(row, to.min(width));
        let text = cells_text(row, from.into(), to.into());
        let last = line == b.line;
        // 折り返しで続く行は、行末まで選んでいれば空白を残してつなぐ
        let joins = !sel.block && row.wrapped && !last;
        if joins {
            out.push_str(&text);
        } else {
            out.push_str(text.trim_end());
            if !last {
                out.push('\n');
            }
        }
    }
    // 画面下の空行まで選んだとき（全て選択など）の改行の並びは残さない
    let kept = out.trim_end_matches('\n').len();
    out.truncate(kept);
    out
}

/// 区切り文字か。
fn is_delim(text: &str, delims: &str) -> bool {
    text.is_empty() || text.chars().all(|c| delims.contains(c))
}

/// `p` の位置の単語の範囲（ダブルクリック）。区切り文字の上なら、同じ区切り文字の並びを選ぶ。
/// 折り返しで続く行にまたがる単語も 1 つとして扱う。
pub fn word_at(rows: &[Row], p: Point, delims: &str) -> Selection {
    let flat = Logical::around(rows, p.line);
    let Some(flat) = flat else {
        return Selection {
            start: p,
            end: p,
            block: false,
        };
    };
    let Some(i) = flat.index_of(p) else {
        return Selection {
            start: p,
            end: p,
            block: false,
        };
    };
    let class = |t: &str| -> Option<String> {
        if is_delim(t, delims) {
            // 同じ区切り文字（空白なら空白）だけをまとめる
            Some(t.to_owned())
        } else {
            None
        }
    };
    let me = class(&flat.cells[i].text);
    let same = |j: usize| class(&flat.cells[j].text) == me;
    let mut s = i;
    while s > 0 && same(s - 1) {
        s -= 1;
    }
    let mut e = i + 1;
    while e < flat.cells.len() && same(e) {
        e += 1;
    }
    Selection {
        start: flat.point(s),
        end: flat.point_end(e),
        block: false,
    }
}

/// `line` を含む論理行（折り返しでつながった行の全体）の範囲（トリプルクリック）。
pub fn line_at(rows: &[Row], line: i32) -> Selection {
    match Logical::around(rows, line) {
        Some(flat) => Selection {
            start: flat.point(0),
            end: flat.point_end(flat.cells.len()),
            block: false,
        },
        None => Selection {
            start: Point::new(line, 0),
            end: Point::new(line, 0),
            block: false,
        },
    }
}

/// 論理行。折り返しでつながった行のセルを 1 列に並べたもの。
struct Logical<'a> {
    cells: Vec<&'a crate::types::Cell>,
    /// 各セルの元の位置
    at: Vec<Point>,
}

impl<'a> Logical<'a> {
    /// `line` を含む論理行。
    fn around(rows: &'a [Row], line: i32) -> Option<Self> {
        let find = |l: i32| rows.iter().find(|r| r.line == l);
        find(line)?;
        let mut first = line;
        while let Some(prev) = find(first - 1) {
            if !prev.wrapped {
                break;
            }
            first -= 1;
        }
        let mut lines = vec![];
        let mut l = first;
        while let Some(r) = find(l) {
            lines.push(r);
            if !r.wrapped {
                break;
            }
            l += 1;
        }
        Some(Self::from_rows(&lines))
    }

    fn from_rows(rows: &[&'a Row]) -> Self {
        let mut cells = vec![];
        let mut at = vec![];
        for r in rows {
            for (c, cell) in r.cells.iter().enumerate() {
                cells.push(cell);
                at.push(Point::new(r.line, c as u16));
            }
        }
        Self { cells, at }
    }

    fn index_of(&self, p: Point) -> Option<usize> {
        let i = self
            .at
            .iter()
            .position(|q| q.line == p.line && q.col == p.col)
            .or_else(|| {
                // 行末より右なら、その行の最後のセル
                self.at.iter().rposition(|q| q.line == p.line)
            })?;
        // 全角の右半分なら左半分へ
        if i > 0 && self.cells[i].flags & CellFlags::WIDE_SPACER != 0 {
            Some(i - 1)
        } else {
            Some(i)
        }
    }

    /// セル `i` の左端。
    fn point(&self, i: usize) -> Point {
        self.at.get(i).copied().unwrap_or_default()
    }

    /// セル `i` の直前のセルの右端（`i` は排他的な終わり）。
    fn point_end(&self, i: usize) -> Point {
        if i == 0 {
            return self.point(0);
        }
        let last = self.at[i - 1];
        Point::new(last.line, last.col + 1)
    }

    /// 文字列と、各文字の先頭バイトがどのセルかの対応。
    fn text(&self) -> (String, Vec<(usize, usize)>) {
        let mut s = String::new();
        let mut map = vec![];
        for (i, c) in self.cells.iter().enumerate() {
            if c.text.is_empty() {
                continue;
            }
            map.push((s.len(), i));
            s.push_str(&c.text);
        }
        (s, map)
    }
}

/// バイト位置 → セルの番号（文字の途中なら、その文字のセル）。
fn cell_of(map: &[(usize, usize)], byte: usize) -> usize {
    match map.binary_search_by_key(&byte, |&(b, _)| b) {
        Ok(i) => map[i].1,
        Err(0) => 0,
        Err(i) => map[i - 1].1,
    }
}

/// 検索の条件。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub regex: bool,
}

impl SearchQuery {
    /// 正規表現に組み立てる。空、または正規表現が誤っていれば Err（メッセージ）。
    pub fn compile(&self) -> Result<Regex, String> {
        if self.text.is_empty() {
            return Err(String::new());
        }
        let pattern = if self.regex {
            self.text.clone()
        } else {
            regex::escape(&self.text)
        };
        RegexBuilder::new(&pattern)
            .case_insensitive(!self.case_sensitive)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| e.to_string())
    }
}

/// 一度に返す一致の上限
pub const MAX_MATCHES: usize = 10_000;

/// `rows`（行番号の昇順）から `re` に一致する範囲を探す。折り返しでつながった行は 1 行として探す。
/// 空文字列への一致は返さない。
pub fn search(rows: &[Row], re: &Regex) -> Vec<Selection> {
    let mut out = vec![];
    let mut i = 0;
    while i < rows.len() && out.len() < MAX_MATCHES {
        let mut j = i;
        while j + 1 < rows.len() && rows[j].wrapped && rows[j + 1].line == rows[j].line + 1 {
            j += 1;
        }
        let group: Vec<&Row> = rows[i..=j].iter().collect();
        let flat = Logical::from_rows(&group);
        let (text, map) = flat.text();
        for m in re.find_iter(&text) {
            if m.start() == m.end() {
                continue;
            }
            let s = cell_of(&map, m.start());
            let e = cell_of(&map, m.end() - 1);
            // 終わりは最後の文字のセル（全角なら右半分まで）の右端
            let mut end_cell = e + 1;
            if end_cell < flat.cells.len()
                && flat.cells[end_cell].flags & CellFlags::WIDE_SPACER != 0
            {
                end_cell += 1;
            }
            out.push(Selection {
                start: flat.point(s),
                end: flat.point_end(end_cell),
                block: false,
            });
            if out.len() >= MAX_MATCHES {
                break;
            }
        }
        i = j + 1;
    }
    out
}

fn url_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)\b(?:https?|ftp|sftp|ssh|telnet|file)://[^\s<>"'`{}|\\^]+"#)
            .expect("URL の正規表現")
    })
}

/// 末尾の句読点や、対応の取れない閉じ括弧を URL から外す。
fn trim_url(s: &str) -> &str {
    let mut s = s;
    loop {
        let Some(last) = s.chars().last() else {
            return s;
        };
        let strip = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' => true,
            ')' => s.matches('(').count() < s.matches(')').count(),
            ']' => s.matches('[').count() < s.matches(']').count(),
            _ => false,
        };
        if !strip {
            return s;
        }
        s = &s[..s.len() - last.len_utf8()];
    }
}

/// 端末上の URL 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlMatch {
    pub range: Selection,
    pub url: String,
}

/// `rows` の中の URL（クリッカブル URL の下線と、Cmd / Ctrl + クリック用）。
pub fn find_urls(rows: &[Row]) -> Vec<UrlMatch> {
    let mut out = vec![];
    let mut i = 0;
    while i < rows.len() {
        let mut j = i;
        while j + 1 < rows.len() && rows[j].wrapped && rows[j + 1].line == rows[j].line + 1 {
            j += 1;
        }
        let group: Vec<&Row> = rows[i..=j].iter().collect();
        let flat = Logical::from_rows(&group);
        let (text, map) = flat.text();
        for m in url_regex().find_iter(&text) {
            let url = trim_url(m.as_str());
            if url.len() <= "http://".len() {
                continue;
            }
            let s = cell_of(&map, m.start());
            let e = cell_of(&map, m.start() + url.len() - 1);
            out.push(UrlMatch {
                range: Selection {
                    start: flat.point(s),
                    end: flat.point_end(e + 1),
                    block: false,
                },
                url: url.to_owned(),
            });
        }
        i = j + 1;
    }
    out
}

/// `p` の位置にある URL。
pub fn url_at(rows: &[Row], p: Point) -> Option<String> {
    let flat = Logical::around(rows, p.line)?;
    let first = flat.at.first()?.line;
    let last = flat.at.last()?.line;
    let group: Vec<Row> = rows
        .iter()
        .filter(|r| r.line >= first && r.line <= last)
        .cloned()
        .collect();
    find_urls(&group)
        .into_iter()
        .find(|u| u.range.contains(p.line, p.col))
        .map(|u| u.url)
}

/// Tera Term の「テーブルをコピー」：連続する空白をタブ 1 つにし、行頭・行末の空白を除く。
pub fn table_to_tsv(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join("\t"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Cell;

    /// テスト用の行。全角は WIDE + WIDE_SPACER の 2 セルにする。
    fn row(line: i32, text: &str, width: usize, wrapped: bool) -> Row {
        let mut cells = vec![];
        for ch in text.chars() {
            let wide = !ch.is_ascii();
            cells.push(Cell {
                text: ch.to_string(),
                fg: 0,
                bg: 0,
                flags: if wide { CellFlags::WIDE } else { 0 },
            });
            if wide {
                cells.push(Cell {
                    text: String::new(),
                    fg: 0,
                    bg: 0,
                    flags: CellFlags::WIDE_SPACER,
                });
            }
        }
        while cells.len() < width {
            cells.push(Cell {
                text: " ".into(),
                fg: 0,
                bg: 0,
                flags: 0,
            });
        }
        Row {
            line,
            cells,
            wrapped,
        }
    }

    fn sel(a: (i32, u16), b: (i32, u16), block: bool) -> Selection {
        Selection {
            start: Point::new(a.0, a.1),
            end: Point::new(b.0, b.1),
            block,
        }
    }

    #[test]
    fn normal_selection_trims_and_joins_wrapped_lines() {
        let rows = [
            row(-1, "hello world", 12, false),
            row(0, "abcdefghijkl", 12, true),
            row(1, "mn", 12, false),
        ];
        // 逆向きに選んでもよい
        assert_eq!(
            selection_text(&rows, &sel((1, 2), (-1, 6), false)),
            "world\nabcdefghijklmn"
        );
        assert_eq!(
            selection_text(&rows, &sel((-1, 0), (-1, 5), false)),
            "hello"
        );
        assert_eq!(selection_text(&rows, &sel((0, 3), (0, 3), false)), "");
    }

    #[test]
    fn block_selection_takes_the_same_columns_from_each_line() {
        let rows = [
            row(0, "eth0  up    1G", 16, false),
            row(1, "eth1  down  10G", 16, false),
        ];
        assert_eq!(
            selection_text(&rows, &sel((1, 12), (0, 6), true)),
            "up\ndown"
        );
        assert!(sel((0, 3), (5, 3), true).is_empty());
        assert!(sel((0, 6), (1, 12), true).contains(1, 6));
        assert!(!sel((0, 6), (1, 12), true).contains(1, 12));
    }

    #[test]
    fn wide_characters_are_not_cut_in_half() {
        let rows = [row(0, "aあいb", 8, false)];
        // あ の右半分から い の左半分まで → あい
        assert_eq!(selection_text(&rows, &sel((0, 2), (0, 4), false)), "あい");
    }

    #[test]
    fn double_click_selects_addresses_and_paths_whole() {
        let rows = [row(0, "ping 192.168.1.10 /var/log", 30, false)];
        let w = word_at(&rows, Point::new(0, 8), DEFAULT_DELIMITERS);
        assert_eq!(selection_text(&rows, &w), "192.168.1.10");
        let w = word_at(&rows, Point::new(0, 20), DEFAULT_DELIMITERS);
        assert_eq!(selection_text(&rows, &w), "/var/log");
        // 空白の上なら空白の並び
        let w = word_at(&rows, Point::new(0, 4), DEFAULT_DELIMITERS);
        assert_eq!((w.start.col, w.end.col), (4, 5));
    }

    #[test]
    fn double_click_word_spans_a_wrapped_line() {
        let rows = [row(0, "xx abcd", 7, true), row(1, "efg yy", 7, false)];
        let w = word_at(&rows, Point::new(1, 1), DEFAULT_DELIMITERS);
        assert_eq!(selection_text(&rows, &w), "abcdefg");
        let l = line_at(&rows, 1);
        assert_eq!(selection_text(&rows, &l), "xx abcdefg yy");
    }

    #[test]
    fn search_finds_matches_across_wrapped_lines_and_wide_chars() {
        let rows = [
            row(-2, "Error: link down", 20, false),
            row(-1, "あいうERR", 8, true),
            row(0, "OR end", 8, false),
        ];
        let q = SearchQuery {
            text: "error".into(),
            ..Default::default()
        };
        let m = search(&rows, &q.compile().unwrap());
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].start, m[0].end),
            (Point::new(-2, 0), Point::new(-2, 5))
        );
        assert_eq!(
            (m[1].start, m[1].end),
            (Point::new(-1, 6), Point::new(0, 2))
        );

        let q = SearchQuery {
            text: "error".into(),
            case_sensitive: true,
            ..Default::default()
        };
        assert!(search(&rows, &q.compile().unwrap()).is_empty());

        let q = SearchQuery {
            text: "い.".into(),
            regex: true,
            ..Default::default()
        };
        let m = search(&rows, &q.compile().unwrap());
        assert_eq!(
            (m[0].start, m[0].end),
            (Point::new(-1, 2), Point::new(-1, 6))
        );

        // 正規表現でない検索では記号もそのまま
        let q = SearchQuery {
            text: "(".into(),
            ..Default::default()
        };
        assert!(q.compile().is_ok());
        let q = SearchQuery {
            text: "(".into(),
            regex: true,
            ..Default::default()
        };
        assert!(q.compile().is_err());
    }

    #[test]
    fn urls_are_found_and_trailing_punctuation_dropped() {
        let rows = [
            row(0, "see https://example.com/a_(b). ok", 40, false),
            row(1, "(http://x.test/p)", 40, false),
        ];
        let urls = find_urls(&rows);
        let list: Vec<_> = urls.iter().map(|u| u.url.as_str()).collect();
        assert_eq!(list, ["https://example.com/a_(b)", "http://x.test/p"]);
        assert_eq!(urls[0].range.start, Point::new(0, 4));
        assert_eq!(
            url_at(&rows, Point::new(0, 10)).as_deref(),
            Some("https://example.com/a_(b)")
        );
        assert_eq!(url_at(&rows, Point::new(0, 1)), None);
    }

    #[test]
    fn url_spanning_a_wrapped_line() {
        let rows = [
            row(0, "go http://ab", 12, true),
            row(1, "c.test/x", 12, false),
        ];
        assert_eq!(
            url_at(&rows, Point::new(1, 2)).as_deref(),
            Some("http://abc.test/x")
        );
    }

    #[test]
    fn table_copy_turns_runs_of_spaces_into_tabs() {
        let text = "  Port   Status  Speed\nGi0/1  up      1000 ";
        assert_eq!(table_to_tsv(text), "Port\tStatus\tSpeed\nGi0/1\tup\t1000");
    }
}
