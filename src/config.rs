use clap::Args;

#[derive(Args, Debug, Clone)]
#[group(required = false, multiple = false)]
pub struct Config {
    #[arg(default_value_t = true)]
    #[arg(long)]
    pub use_file_writer: bool,
    #[arg(default_value_t = 4)]
    #[arg(long)]
    pub max_conns: usize,
    #[arg(default_value_t = 128)]
    #[arg(long)]
    pub peer_tx_size: usize,
    #[arg(default_value_t = 128)]
    #[arg(long)]
    pub peer_rx_size: usize,
    #[arg(default_value_t = 2)]
    #[arg(long)]
    pub file_handler_channel_size: usize,
    #[arg(default_value_t = 128)]
    #[arg(long)]
    pub max_outstanding_requests: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            use_file_writer: true,
            max_conns: 4,
            peer_tx_size: 128,
            peer_rx_size: 128,
            file_handler_channel_size: 2,
            max_outstanding_requests: 128,
        }
    }
}
