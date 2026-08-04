use base64::{engine::general_purpose::STANDARD, Engine as _};
use minisign_verify::{PublicKey, Signature};
use serde_json::Value;
use std::{env, fs, path::Path};

fn main() {
    let mut arguments = env::args().skip(1);
    let payload_path = arguments
        .next()
        .expect("usage: verify-updater-signature <payload> <signature>");
    let signature_path = arguments
        .next()
        .expect("usage: verify-updater-signature <payload> <signature>");
    assert!(arguments.next().is_none(), "only payload and signature are accepted");

    // 使用与发布客户端相同的内嵌公钥，而不是从环境变量或网络获取任何数据。
    let config: Value = serde_json::from_str(include_str!("../../../src-tauri/tauri.conf.json"))
        .expect("tauri.conf.json must remain valid JSON");
    let encoded_public_key = config["plugins"]["updater"]["pubkey"]
        .as_str()
        .expect("updater public key must be configured");
    let public_key_text = decode_outer_base64(encoded_public_key, "updater public key");
    let signature_encoded = fs::read_to_string(&signature_path).expect("cannot read updater signature");
    let signature_text = decode_outer_base64(signature_encoded.trim(), "updater signature");

    let public_key = PublicKey::decode(&public_key_text).expect("invalid updater public key");
    let signature = Signature::decode(&signature_text).expect("invalid updater signature");
    let payload = fs::read(Path::new(&payload_path)).expect("cannot read updater payload");
    public_key
        .verify(&payload, &signature, false)
        .expect("updater payload signature does not match the embedded public key");

    // 只输出通过结果，不记录 URL、签名、公钥或任何认证资料。
    println!("Updater signature verification passed.");
}

fn decode_outer_base64(value: &str, label: &str) -> String {
    String::from_utf8(
        STANDARD
            .decode(value)
            .unwrap_or_else(|_| panic!("{label} must be base64")),
    )
    .unwrap_or_else(|_| panic!("{label} must be UTF-8"))
}
