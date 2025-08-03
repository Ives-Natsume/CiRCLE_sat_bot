use crate::{
    msg::prelude::{
        FromBinMessageEvent,
        BinMessageEvent,
        MessageEvent,
    },
    response::ApiResponse,
    socket::{BotMessage, MsgContent}
};

pub async fn bot_message_handler(
    msg: MsgContent,
) -> ApiResponse<Vec<String>> {
    let mut response: ApiResponse<Vec<String>> = ApiResponse {
        success: false,
        data: None,
        message: None,
    };
    if let Some(message) = msg.message {
        // core端确保包含message的消息不会携带payload和command
        // 直接退出
        return response;
    }

    let Some(payload) = msg.payload else {
      return response;
    };
    let Some(command) = msg.command else {
      return response;
    };

    let payload = BinMessageEvent::from_bin_message_event(payload);
    router(command, payload).await
}

async fn router(
    command: String,
    payload: MessageEvent,
) -> ApiResponse<Vec<String>> {
    let mut response: ApiResponse<Vec<String>> = ApiResponse {
        success: false,
        data: None,
        message: None,
    };
    match command.as_str() {
        "q" | "query" => {
            // unfinished
            response.message = Some("当前为Rinko重构版测试阶段，Rinko的服务器收到了喵，但是功能暂时未完成呢~".to_string());
            response.success = true;
        }
        _ => {}
    }
    response
}