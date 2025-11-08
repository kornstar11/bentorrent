#[derive(Debug, Clone)]
pub struct PeerContext {
    pub info_hash: Vec<u8>,
    pub peer_id: Vec<u8>,
}
