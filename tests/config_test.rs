#[test]
fn test_config_parsing_storage_bucket() {
    std::env::set_var("LKN_STORAGE__BUCKET", "lekton-e2e");
    
    let conf = config::Config::builder()
        .add_source(
            config::Environment::with_prefix("LKN")
                .separator("__")
                .try_parsing(true),
        )
        .build()
        .unwrap();
    
    let bucket: Result<String, _> = conf.get_string("storage.bucket");
    println!("DEBUG BUCKET RESULT: {:?}", bucket);
    
    assert_eq!(bucket.unwrap(), "lekton-e2e");
}
