//! パスワードハッシュを作る。初回アカウント作成と、パスワード再設定に使う。
//!
//! ```text
//! cargo run -p app --example hash_password -- 'correct horse battery staple'
//! ```
//!
//! 出力は `core.app_user.password_hash` にそのまま入る。
//! **平文はシェル履歴に残る。** 共有端末では使わないこと。

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHasher};

fn main() {
    let Some(password) = std::env::args().nth(1) else {
        eprintln!("usage: hash_password <password>");
        std::process::exit(2);
    };

    if password.len() < 12 {
        eprintln!("warning: 12 文字未満のパスワードは推奨しない");
    }

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .expect("argon2 のハッシュ生成に失敗");

    println!("{hash}");
}
