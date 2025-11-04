use core::error;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Missing torrent values {0}")]
    Missing(String),
}