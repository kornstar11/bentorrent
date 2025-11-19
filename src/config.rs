pub struct Config {
    pub use_file_writer: bool,
}

impl Config {
    pub fn new() -> Self {
        Self {
            use_file_writer: true
        }
    }
    
}