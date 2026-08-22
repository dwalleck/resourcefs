use resourcefs_core::Secret;

fn main() {
    let secret = Secret::new("fixture-secret".to_owned()).unwrap();
    println!("{secret:?}");
}
