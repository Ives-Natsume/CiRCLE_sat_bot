use reqwest;
use serde_json;
use crate::{
    query,
    response::ApiResponse,
    config
};
use url;
use crate::msg_sys::prelude::*;
use crate::config::CONFIG;

const ENDPOINT_URL: &str = "http://localhost:3300/send_group_msg";

/// Sends a group message to the specified group ID using the provided API response.
/// Send `ApiResponse.message` if no valid data is provided.
pub async fn send_group_msg(
    response: ApiResponse<Vec<String>>,
    group_id: u64,
) {
    let message_text: String = response
        .data
        .map(|data| data.join("\n"))
        .unwrap_or_else(|| response.message.unwrap_or_else(|| "No data available".to_string()));

    if message_text == "数据已更新喵~" {
        //tracing::info!("No need to send message, data is already updated.");
        return;
    }

    let msg_body = serde_json::json!({
        "group_id": group_id,
        "message": [
            {
                "type": "text",
                "data": {
                    "text": message_text
                }
            }
        ]
    });

    let client = reqwest::Client::new();
    let response = client
        .post(ENDPOINT_URL)
        .json(&msg_body)
        .send()
        .await;

    match response {
        Ok(res) => {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<Failed to read body>".to_string());
            tracing::info!("Group message sent. Status: {}, Response: {}", status, body);
        }
        Err(err) => {
            tracing::error!("Failed to send group message: {}", err);
        }
    }
}

#[allow(dead_code)]
async fn parse_file_path(
    file_path: &str,
) -> Result<String, String> {
    let trimmed_path = file_path.trim_start_matches(r"\\?\");
    let path = std::path::PathBuf::from(trimmed_path);
    match url::Url::from_file_path(&path) {
        Ok(url) => {
            let url_path = url.to_string();
            
            Ok(url_path)
        }
        Err(_) => Err(format!("Failed to convert path to URL: {:?}", path)),
    }
}

async fn send_group_msg_with_photo(
    group_id: u64,
) {
    // let latest_img_path = match crate::solar_image::get_image::get_latest_image().await {
    //     Ok(path) => path,
    //     Err(e) => {
    //         tracing::error!("Failed to get latest solar image: {}", e);
    //         let response = ApiResponse {
    //             success: false,
    //             data: None,
    //             message: Some("出错了喵...".to_string()),
    //         };
    //         send_group_msg(response, group_id).await;
    //         return;
    //     }
    // };

    // let url_path = match parse_file_path(&latest_img_path).await {
    //     Ok(path) => path,
    //     Err(e) => {
    //         tracing::error!("Failed to parse file path: {}", e);
    //         let response = ApiResponse {
    //             success: false,
    //             data: None,
    //             message: Some("图片路径解析失败喵...".to_string()),
    //         };
    //         send_group_msg(response, group_id).await;
    //         return;
    //     }
    // };

    // tracing::info!("Sending group message with photo: {}", url_path);

    let msg_body = serde_json::json!({
        "group_id": group_id,
        "message": [
            {
                "type": "image",
                "data": {
                    "file": "https://www.hamqsl.com/solarn0nbh.php?image=random".to_string(),
                }
            }
        ]
    });

    let client = reqwest::Client::new();
    let response = client
        .post(ENDPOINT_URL)
        .json(&msg_body)
        .send()
        .await;

    match response {
        Ok(res) => {
            let status = res.status();
            let body = res.text().await.unwrap_or_else(|_| "<Failed to read body>".to_string());
            tracing::info!("Group message with photo sent. Status: {}, Response: {}", status, body);
        }
        Err(err) => {
            tracing::error!("Failed to send group message with photo: {}", err);
            let response = ApiResponse {
                success: false,
                data: None,
                message: Some("发送图片失败喵...".to_string()),
            };
            send_group_msg(response, group_id).await;
        }
    }
}

async fn command_router(
    payload: &MessageEvent,
    config: &config::Config,
) {
    let mut message_text: String = String::new();
    let mut response: ApiResponse<Vec<String>> = ApiResponse {
        success: false,
        data: None,
        message: None,
    };

    for elem in &payload.message {
        match elem {
            MessageElement::Text { text } => {
                message_text.push_str(text);
            }
            _ => {}
        }
    }

    let re = regex::Regex::new(r"^\s*/(\w+)(?:\s+([\s\S]*))?$").unwrap();

    if let Some(caps) = re.captures(message_text.as_str()) {
        let command = caps.get(1).unwrap().as_str().to_string();
        let args = caps.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();

        match command.as_str() {
            "query" | "q" => {
                if args.is_empty() {
                    response.message = Some("告诉我卫星名称喵！".to_string());
                }
                else {
                    response = query_handler(&args).await;
                }
            },
            "pass" | "p" => {
                if !config.backend_config.special_group_id.as_ref().map_or(false, |ids| ids.contains(&payload.group_id)) {
                    response.message = Some("这是只有CiRCLE成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }
                if args.is_empty() {
                    response.message = Some("告诉我卫星名称喵！".to_string());
                } else {
                    let query_response = crate::pass_query::sat_pass_predict::query_satellite(Some(args));
                    if query_response.is_empty() {
                        response.message = Some("找不到这个卫星喵...".to_string());
                    } else {
                        response.success = true;
                        response.data = Some(query_response);
                    }
                }
            },
            "all" | "a" => {
                if !config.backend_config.special_group_id.as_ref().map_or(false, |ids| ids.contains(&payload.group_id)) {
                    response.message = Some("这是只有CiRCLE成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }
                let query_response = crate::pass_query::all_pass_notify::get_all_sats_pass().await;
                if query_response.is_empty() {
                    response.message = Some("没有找到卫星经过的信息呢...".to_string());
                } else {
                    response.success = true;
                    response.data = Some(query_response);
                }
            },
            "sun" | "s" => {
                send_group_msg_with_photo(payload.group_id).await;
                return;
            },
            // 热重载函数，搭建中...
            "add" => {
                if !config.backend_config.special_group_id.as_ref().map_or(false, |ids| ids.contains(&payload.group_id)) {
                    response.message = Some("这是只有CiRCLE成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }

                if !config.bot_config.admin_id.contains(&payload.user_id) {
                    response.message = Some("这是只有Roselia成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }

                if args.is_empty() {
                    response.message = Some("告诉我卫星编号喵！".to_string());
                } else {
                    match args.parse::<u32>() {
                        Ok(sat_id) => {
                            let query_response = crate::pass_query::sat_hotload::add_to_temp_list(sat_id, &CONFIG).await;

                            if query_response.is_empty() {
                                response.message = Some("找不到这个卫星喵...".to_string());
                            } else {
                                response.success = true;
                                response.data = Some(query_response);
                            }
                        },
                        Err(_) => {
                            response.message = Some("告诉我卫星编号的数字喵！怎么这么笨喵！".to_string());
                        }
                    }
                }
            },
            "del" => {
                if !config.backend_config.special_group_id.as_ref().map_or(false, |ids| ids.contains(&payload.group_id)) {
                    response.message = Some("这是只有CiRCLE成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }

                if !config.bot_config.admin_id.contains(&payload.user_id) {
                    response.message = Some("这是只有Roselia成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }

                if args.is_empty() {
                    response.message = Some("告诉我卫星编号喵！".to_string());
                } else {
                    let query_response = crate::pass_query::sat_hotload::remove_from_temp_list(&args, &CONFIG).await;

                    if query_response.is_empty() {
                        response.message = Some("找不到这个卫星喵...".to_string());
                    } else {
                        response.success = true;
                        response.data = Some(query_response);
                    }
                }
            },
            "permission" | "chmod" => {
                if !config.backend_config.special_group_id.as_ref().map_or(false, |ids| ids.contains(&payload.group_id)) {
                    response.message = Some("这是只有CiRCLE成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }

                if !config.bot_config.admin_id.contains(&payload.user_id) {
                    response.message = Some("这是只有Roselia成员才能使用的魔法喵~".to_string());
                    send_group_msg(response, payload.group_id).await;
                    return;
                }

                let args_vec: Vec<&str> = args.split_whitespace().collect();

                if args_vec.len() != 3 {
                    response.message = Some("格式是permission <卫星ID> <权限> <开关> 喵！".to_string());
                } else {
                    let name_or_id = args_vec[0];
                    let field = args_vec[1];
                    let value = match args_vec[2].parse::<u8>() {
                        Ok(v) => v,
                        Err(_) => {
                            response.message = Some("小开关没有反应呢...".to_string());
                            send_group_msg(response, payload.group_id).await;
                            return;
                        }
                    };

                    let query_response = crate::pass_query::sat_hotload::set_temp_sat_permission(name_or_id, field, value, &CONFIG).await;

                    if query_response.is_empty() {
                        response.message = Some("没有找到这个卫星喵...".to_string());
                    } else {
                        response.success = true;
                        response.data = Some(query_response);
                    }
                }
            },
            "help" | "h" => {
                response.success = true;
                response.data = config.backend_config.help.clone();
            },
            "about" => {
                response.success = true;
                response.data = config.backend_config.about.clone();
            }
            _ => {
                response.message = Some(format!("说了这些难懂的话，你也有责任吧？"));
            }
        }
    } else {
        response.message = Some("干什么！哈！".to_string());
    }

    let group_id = payload.group_id;
    send_group_msg(response, group_id).await;
}

async fn joke(payload: &MessageEvent, _config: &config::Config) {
    let group_id = payload.group_id;
    for elem in &payload.message {
        if let MessageElement::Text { text } = elem {
            if text.starts_with("/") {
                let text = query::sat_query::sat_name_normalize(text);
                if text.contains("咕咕嘎嘎") || text.contains("gugugaga") {
                    let response = ApiResponse {
                        success: true,
                        data: Some(vec!["咕咕嘎嘎！".to_string()]),
                        message: None,
                    };
                    send_group_msg(response, group_id).await;
                }
                if text.contains("css") {
                    let response = ApiResponse {
                        success: true,
                        data: Some(vec!["又想诈骗，才不会信的说！".to_string()]),
                        message: None,
                    };
                    send_group_msg(response, group_id).await;
                }
                if text.contains("ciallo") {
                    let response = ApiResponse {
                        success: true,
                        data: Some(vec!["Ciallo~(∠・ω< )⌒★".to_string()]),
                        message: None,
                    };
                    send_group_msg(response, group_id).await;
                }
            }
            else {
                let text = query::sat_query::sat_name_normalize(text);
                if text.contains("rinko") || text.contains("rinrin") {
                    let response = ApiResponse {
                        success: true,
                        data: Some(vec!["Rinko在这里喵~".to_string()]),
                        message: None,
                    };
                    send_group_msg(response, group_id).await;
                }
                if text.contains("circle") {
                    let response = ApiResponse {
                        success: true,
                        data: Some(vec!["最喜欢大家了~".to_string()]),
                        message: None,
                    };
                    send_group_msg(response, group_id).await;
                }
                if text.contains("roselia") {
                    let response = ApiResponse {
                        success: true,
                        data: Some(vec!["Rinrin Bloom".to_string()]),
                        message: None,
                    };
                    send_group_msg(response, group_id).await;
                }
                if text == query::sat_query::sat_name_normalize("Rinko在这里喵~") || text == query::sat_query::sat_name_normalize("Rinrin Bloom") {
                    let response = ApiResponse {
                        success: true,
                        data: Some(vec!["不许复读😡".to_string()]),
                        message: None,
                    };
                    send_group_msg(response, group_id).await;
                }
            }
        }
    }
}

async fn query_handler(
    args: &str,
) -> ApiResponse<Vec<String>> {
    let mut response_data: Vec<String> = Vec::new();
    let mut response_msg: String = String::new();
    let mut success: bool = true;

    // query key words:
    // `/query <sat_name>`: look up for AMSAT data by satellite name
    let json_file_path = "amsat_status.json";
    let toml_file_path = "satellites.toml";

    let query_response = query::sat_query::look_up_sat_status_from_json(
        json_file_path,
        toml_file_path,
        args,
    );

    match query_response {
        ApiResponse { success: true, data: Some(results), message: None } => {
            response_data = results;
            if response_data.is_empty() {
                response_msg = format!("Rinko宕机了喵...重新试试吧");
                success = false;
            }
        }
        ApiResponse { success: false, data: None, message: Some(msg) } => {
            response_msg = msg;
            success = false;
        }
        _ => {
            response_msg = "Ako酱...总感觉...有什么不好的事情发生了".to_string();
            success = false;
        }
    }

    let response = ApiResponse {
        success,
        data: if response_data.is_empty() {
            None
        } else {
            Some(response_data)
        },
        message: if response_msg.is_empty() {
            None
        } else {
            Some(response_msg)
        },
    };

    response
}

pub async fn message_handler(
    message_raw_text: String,
    config: &config::Config,
) {
    if let Ok(payload) = parse_message_event(&message_raw_text) {
        for elem in &payload.message {
            match elem {
                MessageElement::Text { text } => {
                    text_router(text, &payload, &config).await;
                }
                MessageElement::At { qq, .. } => {
                    if *qq == config.bot_config.qq_id {
                        command_router(&payload, &config).await;
                    }
                }
                _ => {}
            }
        }
    }
}

async fn text_router(text: &String, payload: &MessageEvent, config: &config::Config) {
    if text.starts_with("/") {
        if text.contains("ciallo") ||
            text.contains("gugugaga") ||
            text.contains("咕咕嘎嘎") ||
            text.contains("css") {
            joke(&payload, config).await;
            return;
        }
    }

    if text.contains("circle") ||
        text.contains("rinrin") ||
        text.contains("rinko") ||
        text.contains("roselia") {
        joke(&payload, config).await;
        return;
    }

    if text.starts_with("/q") || text.starts_with("/h") || text.starts_with("/p") || text.starts_with("/a") || text.starts_with("/s") {
        command_router(&payload, config).await;
        return;
    }
}