use serde::{Serialize, Deserialize};
use serde_json;

#[derive(Debug, Serialize, Deserialize, Clone)]
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct BinMessageEvent {
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
    pub message: Vec<BinMessageElement>,
    pub message_format: String,
    pub post_type: String,
    pub group_id: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub struct Sender {
    pub user_id: u64,
    pub nickname: String,
    pub card: String,
    pub role: String,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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
    #[serde(other)]
    Unknown
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum BinMessageElement {
    At   { qq: String, name: String },
    Text { text: String },
    #[serde(other)]
    Unknown,
}

impl From<BinMessageElement> for MessageElement {
    fn from(v: BinMessageElement) -> Self {
        match v {
            BinMessageElement::At   { qq, name }   => MessageElement::At   { qq, name },
            BinMessageElement::Text { text }       => MessageElement::Text { text },
            BinMessageElement::Unknown             => MessageElement::Unknown,
        }
    }
}

impl From<MessageElement> for BinMessageElement {
    fn from(v: MessageElement) -> Self {
        match v {
            MessageElement::At   { qq, name } => BinMessageElement::At   { qq, name },
            MessageElement::Text { text }     => BinMessageElement::Text { text },
            MessageElement::Unknown           => BinMessageElement::Unknown,
        }
    }
}

pub trait IntoBinMessageEvent {
    fn into_bin_message_event(self) -> BinMessageEvent;
}

impl IntoBinMessageEvent for MessageEvent {
    fn into_bin_message_event(self) -> BinMessageEvent {
        let message_elements: Vec<BinMessageElement> = self.message.into_iter().map(Into::into).collect();
        BinMessageEvent {
            self_id: self.self_id,
            user_id: self.user_id,
            time: self.time,
            message_id: self.message_id,
            message_seq: self.message_seq,
            message_type: self.message_type,
            sender: self.sender,
            raw_message: self.raw_message,
            font: self.font,
            sub_type: self.sub_type,
            message: message_elements,
            message_format: self.message_format,
            post_type: self.post_type,
            group_id: self.group_id,
        }
    }
}

pub trait FromBinMessageEvent {
    fn from_bin_message_event(event: BinMessageEvent) -> MessageEvent;
}

impl FromBinMessageEvent for BinMessageEvent {
    fn from_bin_message_event(event: BinMessageEvent) -> MessageEvent {
        let message_elements: Vec<MessageElement> = event.message.into_iter().map(Into::into).collect();
        MessageEvent {
            self_id: event.self_id,
            user_id: event.user_id,
            time: event.time,
            message_id: event.message_id,
            message_seq: event.message_seq,
            message_type: event.message_type,
            sender: event.sender,
            raw_message: event.raw_message,
            font: event.font,
            sub_type: event.sub_type,
            message: message_elements,
            message_format: event.message_format,
            post_type: event.post_type,
            group_id: event.group_id,
        }
    }
}

pub fn parse_message_event(json_str: &str) -> Result<MessageEvent, serde_json::Error> {
    serde_json::from_str(json_str)
}