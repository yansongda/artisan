//! Artisan workspace facade，通过 feature 控制 re-export。
//!
//! # Features
//!
//! - `http`（默认启用）- re-export [`artisan_http`] 作为 [`http`] 模块
//!
//! # 使用方式
//!
//! ```rust
//! use artisan::http::{Artful, Plugin, Rocket, flow_ctrl::Next};
//! ```

#[cfg(feature = "http")]
pub use artisan_http as http;

#[cfg(test)]
mod tests {
    #[test]
    fn http_facade_re_exports_artful() {
        // facade re-export 冒烟：经 http 模块可正常构造 Artful
        let artful = crate::http::Artful::new().unwrap();
        assert!(artful.config().extra.is_empty());
    }
}
