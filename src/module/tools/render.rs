use resvg::render;
use usvg::{Transform, Tree, Options};
use tiny_skia::Pixmap;
use fontdb::Database;
use std::{path::Path, fs, fmt::Write};
use crate::{
    app_status::AppStatus,
    fs::handler::{FileData, FileFormat, FileRequest},
    module::{
        amsat::prelude::{
            ReportStatus, SatStatus, SatelliteFileElement, SatelliteFileFormat
        },
        tools::roaming::{RoamingData, UserRoamingData}
    }, msg::prelude::MessageEvent, response::ApiResponse
};

const SVG_ROAMING_TEMPLATE_PATH: &str = "data/svg_roaming_template.svg";
const SVG_SATSTATUS_TEMPLATE_PATH: &str = "data/svg_satstatus_template.svg";
pub const SATSTATUS_PIC_PATH_PREFIX: &str = "data/pic/satstatus_pics/";

pub async fn render_roaming_data(
    roaming_data: &Vec<UserRoamingData>,
) -> anyhow::Result<()> {
    const HEADER_HEIGHT: f32 = 42.0;
    const BASE_ROW_HEIGHT: f32 = 38.0; // 基础行高
    const LINE_HEIGHT: f32 = 23.0; // 文本行高
    const MAX_REMARK_WIDTH: f32 = 260.0; // 备注最大宽度
    const MAX_LINES: usize = 3; // 最大显示行数
    const X_PADDING_CALLSIGN: f32 = 20.0;
    const X_PADDING_GRIDS: f32 = 150.0;
    const X_PADDING_REMARK: f32 = 340.0;
    const X_PADDING_UPDATE_TIME: f32 = 630.0;
    const X_PADDING_USER_ID: f32 = 880.0;
    const FOOTER_HEIGHT: f32 = 32.0;

    const EVEN_ROW_COLOR: &str = "#f6f8fa";
    const ODD_ROW_COLOR: &str = "#ffffff";
    const FOOTER_COLOR: &str = "#f0f2f5";

    let template_content = match read_svg_template_file(SVG_ROAMING_TEMPLATE_PATH).await {
        Ok(content) => content,
        Err(e) => {
            tracing::error!("Failed to read SVG template file: {}", e);
            return Err(anyhow::anyhow!("Failed to read SVG template file: {}", e));
        }
    };

    let mut rows_svg = String::new();
    let mut total_height = HEADER_HEIGHT;

    for (index, item) in roaming_data.iter().enumerate() {
        // 处理备注文本换行
        let (remark_lines, line_count) = if let Some(remark) = &item.roaming_data.remark {
            wrap_text(remark, MAX_REMARK_WIDTH, MAX_LINES)
        } else {
            (vec!["".to_string()], 1)
        };
        
        // 计算当前行高度（基础高度 + 额外行高）
        let row_height = BASE_ROW_HEIGHT + (line_count.saturating_sub(1) as f32 * LINE_HEIGHT);
        let y_pos = total_height - HEADER_HEIGHT; // 相对于rows-container的y位置
        
        // 计算文本垂直位置（相对于每行的位置）
        let text_y = y_pos + (BASE_ROW_HEIGHT / 2.0);

        // 计算行的背景颜色
        let row_color = if index % 2 == 0 { EVEN_ROW_COLOR } else { ODD_ROW_COLOR };
        
        // 生成行SVG
        writeln!(
            rows_svg,
            r##"      <g class="data-row">
        <rect x="0" y="{y_pos}" width="100%" height="{row_height}" fill="{row_color}" />
        <text x="{x_callsign}" y="{text_y}" class="table-text row-text monospace">{callsign}</text>
        <text x="{x_grids}" y="{text_y}" class="table-text row-text monospace">{grids}</text>
        <text x="{x_update_time}" y="{text_y}" class="table-text row-text monospace">{update_time}</text>
        <text x="{x_user_id}" y="{text_y}" class="table-text row-text monospace">{user_id}</text>
        <g transform="translate({x_remark}, 0)">
          {remark_lines}
        </g>
      </g>"##,
            y_pos = y_pos,
            row_height = row_height,
            text_y = text_y,
            x_callsign = X_PADDING_CALLSIGN,
            callsign = &item.roaming_data.callsign,
            x_grids = X_PADDING_GRIDS,
            grids = &item.roaming_data.grid,
            x_remark = X_PADDING_REMARK,
            x_update_time = X_PADDING_UPDATE_TIME,
            update_time = &item.submit_time,
            x_user_id = X_PADDING_USER_ID,
            user_id = &item.user_id,
            remark_lines = remark_lines.iter().enumerate().map(|(i, line)| {
                let line_y = y_pos + (BASE_ROW_HEIGHT / 2.0) + (i as f32 * LINE_HEIGHT);
                format!(r#"<text x="0" y="{line_y}" class="table-text row-text">{line}</text>"#)
            }).collect::<Vec<_>>().join("\n          ")
        )?;
        
        total_height += row_height;
    }

    let footer_y = total_height;
    total_height += FOOTER_HEIGHT;

    let render_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let footer_svg = format!(
        r##"    <g id="footer">
        <rect x="0" y="{footer_y}" width="100%" height="{FOOTER_HEIGHT}" fill="{FOOTER_COLOR}" />
        <text x="50%" y="{footer_text_y}" class="table-text footer-text" text-anchor="middle">
            Rinko Bot v0.1.1, rendered at {time_str} BJT, 测试中
        </text>
        </g>"##,
        footer_y = footer_y,
        FOOTER_HEIGHT = FOOTER_HEIGHT,
        FOOTER_COLOR = FOOTER_COLOR,
        footer_text_y = footer_y + (FOOTER_HEIGHT / 2.0),
        time_str = render_time
    );

    let final_svg = template_content
        .replace("{{SVG_HEIGHT}}", &total_height.to_string())
        .replace("{{ROWS}}", &rows_svg)
        .replace("{{FOOTER}}", &footer_svg);

    // let output_path = "data/output.svg";
    // tokio::fs::write(output_path, final_svg).await?;

    let png_output_path: &Path = Path::new("data/pic/roaming_list.png");

    match render_svg_to_png(&final_svg, png_output_path).await {
        Ok(_) => {
            tracing::info!("Successfully rendered PNG to {:?}", png_output_path);
            Ok(())
        },
        Err(e) => {
            tracing::error!("Failed to render SVG to PNG: {}", e);
            Err(anyhow::anyhow!("Failed to render SVG to PNG: {}", e))
        }
    }
}

pub async fn render_satstatus_data(
    report_data: &Vec<SatelliteFileFormat>,
    payload: &MessageEvent,
) -> ApiResponse<Vec<String>> {
    let mut response = ApiResponse::<Vec<String>>::empty();
    tracing::debug!("Rendering satellite status data for {} blocks", report_data.len());
    const BLOCK_TITLE_HEIGHT: f32 = 45.0;
    const HEADER_HEIGHT: f32 = 40.0;
    const ROW_HEIGHT: f32 = 38.0;
    const BLOCK_SPACING: f32 = 30.0;
    const LEFT_PADDING: f32 = 20.0;
    const X_CALLSIGN: f32 = 20.0;
    const X_GRIDS: f32 = 170.0;
    const X_REPORT: f32 = 280.0;
    const X_TIME: f32 = 540.0;
    const COLOR_BLOCK_WIDTH: f32 = 12.0;
    const COLOR_BLOCK_HEIGHT: f32 = 18.0;
    const COLOR_BLOCK_TEXT_SPACING: f32 = 8.0;
    const FOOTER_HEIGHT: f32 = 32.0;
    const FOOTER_COLOR: &str = "#f0f2f5";

    let mut all_blocks_svg = String::new();
    let mut current_y_offset = 20.0;
    let now_utc = chrono::Utc::now();

    if report_data.is_empty() {
        match writeln!(
            all_blocks_svg,
            r##"<text x="50%" y="100" text-anchor="middle" class="table-text" fill="#6e7781">No satellite data available.</text>"##
        ) {
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Failed to write SVG: {}", e);
                response.message = Some(format!("Failed to write SVG: {}", e));
                return response;
            }
        };
        current_y_offset += 120.0;
    } else {
        for block in report_data {
            // draw title
            match writeln!(
                all_blocks_svg,
                r##"<text x="{padding}" y="{y_pos}" class="satellite-title">{name}</text>"##,
                padding = LEFT_PADDING,
                y_pos = current_y_offset + BLOCK_TITLE_HEIGHT / 2.0,
                name = &block.name
            ) {
                Ok(_) => {
                    tracing::debug!("Successfully wrote SVG title for {}", &block.name);
                }
                Err(e) => {
                    tracing::error!("Failed to write SVG: {}", e);
                    response.message = Some(format!("Failed to write SVG: {}", e));
                    return response;
                }
            };
            current_y_offset += BLOCK_TITLE_HEIGHT;

            // draw last_update_time
            let element_time = chrono::DateTime::parse_from_rfc3339(&block.last_update_time)
                    .unwrap()
                    .with_timezone(&chrono::Utc);
            let delta_t = now_utc.signed_duration_since(element_time).num_hours();
            match writeln!(
                all_blocks_svg,
                r##"<text x="{x_time}" y="{y_pos}" class="table-text">Last update: {time} ({delta_t}h ago)</text>"##,
                x_time = X_CALLSIGN,
                y_pos = current_y_offset + ROW_HEIGHT / 2.0,
                time = &block.last_update_time.replace("T", " ").replace("Z", " UTC"),
                delta_t = delta_t
            ) {
                Ok(_) => {
                    tracing::debug!("Successfully wrote SVG last update time for {}", &block.name);
                }
                Err(e) => {
                    tracing::error!("Failed to write SVG: {}", e);
                    response.message = Some(format!("Failed to write SVG: {}", e));
                    return response;
                }
            };
            current_y_offset += ROW_HEIGHT;

            // draw table head
            match writeln!(
                all_blocks_svg,
                r##"<g class="header">
<rect x="0" y="{y_pos}" width="100%" height="{height}" fill="#f0f2f5" />
<text x="{x_call}" y="{text_y}" class="table-text header-text">Callsign</text>
<text x="{x_grid}" y="{text_y}" class="table-text header-text">Grids</text>
<text x="{x_repo}" y="{text_y}" class="table-text header-text">Report</text>
<text x="{x_time}" y="{text_y}" class="table-text header-text">Time</text>
</g>"##,
                y_pos = current_y_offset,
                height = HEADER_HEIGHT,
                text_y = current_y_offset + HEADER_HEIGHT / 2.0,
                x_call = X_CALLSIGN, x_grid = X_GRIDS, x_repo = X_REPORT, x_time = X_TIME
            ) {
                Ok(_) => {
                    tracing::debug!("Successfully wrote SVG table header");
                }
                Err(e) => {
                    tracing::error!("Failed to write SVG: {}", e);
                    response.message = Some(format!("Failed to write SVG: {}", e));
                    return response;
                }
            };
            current_y_offset += HEADER_HEIGHT;

            let mut drawn_rows = 0;
            for element in &block.data {
                if drawn_rows >= 5 {
                    break;
                }

                // draw data rows
                for report in &element.report {
                    let y_pos = current_y_offset + ROW_HEIGHT / 2.0;
                    let report_text_x = X_REPORT + COLOR_BLOCK_WIDTH + COLOR_BLOCK_TEXT_SPACING;
                    let report_time = chrono::DateTime::parse_from_rfc3339(&report.reported_time)
                        .unwrap()
                        .with_timezone(&chrono::Utc);
                    let delta_t = now_utc.signed_duration_since(report_time).num_hours();
                    match writeln!(
                        all_blocks_svg,
                        r##"<g class="data-row">
   <text x="{x_callsign}" y="{y_pos}" class="table-text">{callsign}</text>
   <text x="{x_grid}" y="{y_pos}" class="table-text">{grid}</text>
   <rect x="{x_report}" y="{rect_y}" width="{rect_w}" height="{rect_h}" fill="{color}" rx="1" />
   <text x="{report_text_x}" y="{y_pos}" class="table-text">{report}</text>
   <text x="{x_time}" y="{y_pos}" class="table-text">{time} ({delta_t}h ago)</text>
</g>"##,
                        x_callsign = X_CALLSIGN,
                        x_grid = X_GRIDS,
                        x_report = X_REPORT,
                        report_text_x = report_text_x,
                        x_time = X_TIME,
                        y_pos = y_pos,
                        rect_y = y_pos - COLOR_BLOCK_HEIGHT / 2.0,
                        rect_w = COLOR_BLOCK_WIDTH,
                        rect_h = COLOR_BLOCK_HEIGHT,
                        callsign = &report.callsign,
                        grid = &report.grid_square,
                        report = ReportStatus::from_string(&report.report).to_string(),
                        color = ReportStatus::string_to_color_hex(&report.report),
                        time = &report.reported_time,
                        delta_t = delta_t
                    ) {
                        Ok(_) => {
                            tracing::debug!("Successfully wrote SVG data row for {}", &report.callsign);
                        }
                        Err(e) => {
                            tracing::error!("Failed to write SVG: {}", e);
                            response.message = Some(format!("Failed to write SVG: {}", e));
                            return response;
                        }
                    };
                    current_y_offset += ROW_HEIGHT;
                    drawn_rows += 1;
                }
            }

            current_y_offset += BLOCK_SPACING;
        }
    }

    let footer_y = current_y_offset;
    current_y_offset += FOOTER_HEIGHT;

    let render_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let footer_svg = format!(
        r##"    <g id="footer">
        <rect x="0" y="{footer_y}" width="100%" height="{FOOTER_HEIGHT}" fill="{FOOTER_COLOR}" />
        <text x="50%" y="{footer_text_y}" class="table-text footer-text" text-anchor="middle">
            Rinko Bot v0.1.1, rendered at {time_str} BJT, 测试中
        </text>
        </g>"##,
        footer_y = footer_y,
        FOOTER_HEIGHT = FOOTER_HEIGHT,
        FOOTER_COLOR = FOOTER_COLOR,
        footer_text_y = footer_y + (FOOTER_HEIGHT / 2.0),
        time_str = render_time
    );

    let total_height = current_y_offset;
    let template_content = tokio::fs::read_to_string(SVG_SATSTATUS_TEMPLATE_PATH).await;

    let template_content = match template_content {
        Ok(content) => {
            content
        },
        Err(e) => {
            tracing::error!("Failed to read SVG template file: {}", e);
            response.message = Some(format!("Failed to read SVG template file: {}", e));
            return response;
        }
    };

    let final_svg = template_content
        .replace("{{SVG_HEIGHT}}", &total_height.to_string())
        .replace("{{CONTENT}}", &all_blocks_svg)
        .replace("{{FOOTER}}", &footer_svg);

    // let output_path = "data/output.svg";
    // tokio::fs::write(output_path, final_svg).await?;

    let group_id = payload.group_id.clone();
    let message_id = payload.message_id.clone();
    let time = now_utc.to_rfc3339();

    let output_path_string = format!("{}{}-{}-{}.png", SATSTATUS_PIC_PATH_PREFIX, time, group_id, message_id);
    let png_output_path = Path::new(&output_path_string);

    match render_svg_to_png(&final_svg, png_output_path).await {
        Ok(_) => {
            tracing::info!("Successfully rendered PNG to {:?}", png_output_path);
            response.message = Some("image".to_string());
            response.data = Some(vec![format!("file:///server_{}", output_path_string)]);
            response.success = true;
            response
        },
        Err(e) => {
            tracing::error!("Failed to render SVG to PNG: {}", e);
            response.message = Some(format!("Failed to render SVG to PNG: {}", e));
            response
        }
    }
}

fn wrap_text(text: &str, max_width: f32, max_lines: usize) -> (Vec<String>, usize) {
    // 简单换行算法：按字符分割
    let avg_char_width = 8.0; // 平均字符宽度估计值
    
    let mut lines = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0.0;
    
    for word in text.split_whitespace() {
        let word_width = word.len() as f32 * avg_char_width;
        
        // 检查当前行是否能容纳这个词
        if current_width + word_width > max_width && !current_line.is_empty() {
            lines.push(current_line.trim().to_string());
            current_line.clear();
            current_width = 0.0;
            
            if lines.len() >= max_lines {
                break;
            }
        }
        
        current_line.push_str(word);
        current_line.push(' ');
        current_width += word_width + avg_char_width; // 单词宽度+空格
    }
    
    // 添加最后一行
    if !current_line.is_empty() && lines.len() < max_lines {
        lines.push(current_line.trim().to_string());
    }
    
    // 如果超过最大行数，截断最后一行
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            if last.len() > 3 {
                last.truncate(last.len() - 3);
                last.push_str("...");
            }
        }
    }
    
    let line_count = lines.len();
    if lines.is_empty() {
        lines.push("".to_string());
    }
    
    (lines, line_count)
}

async fn read_svg_template_file(
    path: &str
) -> Result<String, String> {
    let svg_data = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read SVG template file: {}", e))?;
    Ok(svg_data)
}

async fn render_svg_to_png(svg_data: &str, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut fontdb = Database::new();
    fontdb.load_fonts_dir("fonts");

    let mut options = Options::default();
    options.fontdb = fontdb.into();
    let rtree = Tree::from_str(svg_data, &options)?;

    let size = rtree.size().to_int_size();
    let width = size.width();
    let height = size.height();

    let mut pixmap = Pixmap::new(width, height).ok_or("Failed to create Pixmap")?;

    render(&rtree, Transform::default(), &mut pixmap.as_mut());

    let png_data = pixmap.encode_png()?;
    tokio::fs::write(output_path, png_data).await?;
    
    Ok(())
}