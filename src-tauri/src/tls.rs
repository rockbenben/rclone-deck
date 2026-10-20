//! 装一个 rustls 的算法提供者。
//!
//! **为什么需要它**：`tauri-plugin-updater` 带着 `reqwest/rustls-no-provider` 进来
//! （它要 https 去问发布页）。cargo 的特征是**并集**，所以同一个 reqwest 也跟着带上了
//! rustls 后端 —— 而 rustls 0.23 不会自己挑一个提供者，不装一个的话，**每一个**
//! reqwest 客户端在建的那一刻就会 panic（面板自己那个只走 http 的也一样）。
//!
//! 这一条不是假想：`reqwest` 在建客户端时先问"有没有人装过默认提供者"
//! （`Client::builder().build()` 里那句 `CryptoProvider::get_default()`），
//! 没有才用自己那份，而 `rustls-no-provider` 那一档**没有**自己那份 —— 于是直接 panic。
//! 症状是面板一起来就崩（`expect("reqwest client")` 那一句），或者测试里
//! `RcClient::from_base_url` 直接炸在那儿。
//!
//! 装的是 **ring**：插件自己也是用它那一份（`tauri-plugin-updater` 的 Cargo.toml 里
//! `features = ["ring"]`），两份是同一个提供者，不会打架。
//!
//! 装几次都无所谓：第二个提供者装不上（已经有了一个），那不是事，吞掉即可。

use std::sync::Once;

static ONCE: Once = Once::new();

/// 装一次提供者。**在每一个 reqwest 客户端建之前都要过一遍这一句。**
pub fn ensure_provider() {
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
