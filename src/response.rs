use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ApiResponse<T> {
    pub success: bool,
    /// Carries the data if the operation was successful, could be a response message
    pub data: Option<T>,
    /// Carries an error message if the operation failed
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

    #[allow(dead_code)]
    pub fn new(success: bool, data: T, message: impl Into<String>) -> Self {
        Self {
            success,
            data: Some(data),
            message: Some(message.into()),
        }
    }

    pub fn empty() -> Self {
        Self {
            success: false,
            data: None,
            message: None,
        }
    }
}

#[allow(dead_code)]
pub fn json_response<T: Serialize>(
    success: bool,
    message: Option<String>,
    data: Option<T>
) -> axum::Json<ApiResponse<T>> {
    axum::Json(ApiResponse {
        success,
        message: message.into(),
        data,
    })
}