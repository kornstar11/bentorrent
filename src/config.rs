pub struct Config {
    pub use_file_writer: bool,
    pub max_conns: usize,
}

impl Config {
    pub fn new() -> Self {
        Self {
            use_file_writer: true,
            max_conns: 4
        }
    }
    
}