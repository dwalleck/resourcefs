use resourcefs_core::Secret;

fn main() {
    let secret = Secret::new("fixture-secret".to_owned()).unwrap();
    let _serialized = serde_json::to_string(&secret).unwrap();
}
