#[derive(Debug, Clone)]
pub struct Config {
    pub use_file_writer: bool,
    pub max_conns: usize,
    pub peer_tx_size: usize,
    pub peer_rx_size: usize,
    pub file_handler_channel_size: usize,
}

impl Config {
    pub fn new() -> Self {
        Self {
            use_file_writer: true,
            max_conns: 4,
            peer_tx_size: 128,
            peer_rx_size: 128,
            file_handler_channel_size: 2,
        }
    }
}
