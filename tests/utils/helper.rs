pub fn clear_output() {
    let _ = std::fs::remove_dir_all("test_output");
}

pub fn generate_random_string() -> String {
    let random_string = uuid::Uuid::new_v4().to_string();
    random_string
}
