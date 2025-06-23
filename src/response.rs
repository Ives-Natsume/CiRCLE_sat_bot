use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub message: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: None,
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            message: Some(msg.into()),
        }
    }

    pub fn new(success: bool, data: T, message: impl Into<String>) -> Self {
        Self {
            success,
            data: Some(data),
            message: Some(message.into()),
        }
    }
}
