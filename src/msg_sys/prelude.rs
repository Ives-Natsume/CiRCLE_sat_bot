use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Sender {
    pub user_id: u64,
}

#[derive(Deserialize, Debug)]
pub struct IncomingRequest {
    pub sender: Sender,
    pub message: String,
    pub group_id: u64,
}