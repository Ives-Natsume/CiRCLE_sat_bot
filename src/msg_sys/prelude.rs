use serde::{Serialize, Deserialize};
use serde_json::{
    Value,
};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MessageEvent {
    pub self_id: u64,
    pub user_id: u64,
    pub time: i64,
    pub message_id: u64,
    pub message_seq: u64,
    pub message_type: String,
    pub sender: Sender,
    pub raw_message: String,
    pub font: u32,
    pub sub_type: String,
    pub message: Vec<MessageElement>,
    pub message_format: String,
    pub post_type: String,
    pub group_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Sender {
    pub user_id: u64,
    pub nickname: String,
    pub card: String,
    pub role: String,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type", content = "data")]
pub enum MessageElement {
    #[serde(rename = "at")]
    At {
        qq: String,
        name: String,
    },
    #[serde(rename = "text")]
    Text {
        text: String,
    },
    #[serde(rename = "other")]
    Unknown
}

pub fn parse_message_event(json_str: &str) -> Result<MessageEvent, serde_json::Error> {
    serde_json::from_str(json_str)
}