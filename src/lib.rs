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
