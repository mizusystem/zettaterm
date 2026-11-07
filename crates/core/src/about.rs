//! バージョン情報（Tera Term の「ヘルプ > Tera Term について」）。

/// アプリの版
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// ビルドしたコミット（短い形）。分からなければ空
pub const COMMIT: &str = env!("MT_GIT_COMMIT");

/// 参照したソフトウェアの表示（THIRD_PARTY.md。Tera Term の BSD-3-Clause の表示を含む）
pub const NOTICES: &str = include_str!("../../../THIRD_PARTY.md");

/// 版とコミットを 1 行で（例：`0.0.0 (616d2d9)`）。
pub fn version_line() -> String {
    if COMMIT.is_empty() {
        VERSION.to_owned()
    } else {
        format!("{VERSION} ({COMMIT})")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn notices_include_the_tera_term_license() {
        assert!(super::NOTICES.contains("TeraTerm Project"));
        assert!(super::version_line().starts_with(super::VERSION));
    }
}
