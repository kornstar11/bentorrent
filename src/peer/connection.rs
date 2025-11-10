

pub struct PeerState {
    am_choking: bool,
    am_interested: bool,
    peer_choking: bool,
    peer_interested: bool,
}

impl PeerState {

    pub fn can_upload_to(&self) -> bool {
        self.peer_interested && !self.am_choking
    }

    pub fn can_download_from(&self) -> bool {
        self.am_interested && !self.peer_choking
    }
    
}