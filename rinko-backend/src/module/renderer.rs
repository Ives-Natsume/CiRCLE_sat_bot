///! Satellite status renderer - Multi-transponder support
///!
///! Generate images from satellite data with transponder information

// use super::sat::manager::{AmsatSearchResult, SatelliteManager};
// use super::sat::types::{AmsatReport, ReportStatus, SatelliteDataBlock};
// use super::sat::amsat_types::AmsatEntry;
use super::sat_rev::types::{AmsatEntry, ReportStatus, AmsatReport, SatelliteDataBlock};
use super::lotw::types::{LotwQueueSnapshot, LotwQueueRow};
use super::qo100::types::{Qo100Snapshot, Qo100Spot};
use anyhow::{Context, Result};
use chrono::{DateTime, Timelike, Utc};
use std::path::{Path, PathBuf};
use resvg::usvg::{fontdb, Options, Tree};
use resvg::tiny_skia;
use fontdb::Database;

const SAT_SVG_TEMPLATE: &str = "resources/sat_template.svg";
const LOTW_SVG_TEMPLATE: &str = "resources/lotw_template.svg";
const QO100_SVG_TEMPLATE: &str = "resources/qo100_template.svg";

/// Map time difference to color gradient
fn map_time_to_color(target_time: &str, now_utc: &DateTime<Utc>, min_hours: f64, max_hours: f64) -> Result<String> {
    let target = DateTime::parse_from_rfc3339(target_time)
        .context("Failed to parse target time")?;
    let target_utc = target.with_timezone(&Utc);

    let delta_hours = (now_utc.signed_duration_since(target_utc)).num_seconds().abs() as f64 / 3600.0;

    let green = (125u8, 227u8, 61u8);
    let yellow = (255u8, 255u8, 0u8);
    let red = (255u8, 0u8, 0u8);

    let (r, g, b) = if delta_hours <= min_hours {
        green
    } else if delta_hours >= max_hours {
        red
    } else {
        let mid = (min_hours + max_hours) / 2.0;
        if delta_hours <= mid {
            let t = (delta_hours - min_hours) / (mid - min_hours);
            lerp_color(green, yellow, t)
        } else {
            let t = (delta_hours - mid) / (max_hours - mid);
            lerp_color(yellow, red, t)
        }
    };

    Ok(format!("#{:02X}{:02X}{:02X}", r, g, b))
}

fn lerp_color(c1: (u8, u8, u8), c2: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let r = (c1.0 as f64 + (c2.0 as f64 - c1.0 as f64) * t).round() as u8;
    let g = (c1.1 as f64 + (c2.1 as f64 - c1.1 as f64) * t).round() as u8;
    let b = (c1.2 as f64 + (c2.2 as f64 - c1.2 as f64) * t).round() as u8;
    (r, g, b)
}

/// Satellite renderer
pub struct SatelliteRenderer {
    output_dir: PathBuf,
}

impl SatelliteRenderer {
    const BLOCK_TITLE_HEIGHT: f32 = 45.0;
    const HEADER_HEIGHT: f32 = 40.0;
    const ROW_HEIGHT: f32 = 38.0;
    const BLOCK_SPACING: f32 = 30.0;
    const TOP_PADDING: f32 = 20.0;
    const FOOTER_HEIGHT: f32 = 32.0;
    const MAX_REPORTS_PER_SATELLITE: usize = 5;

    const X_CALLSIGN: f32 = 20.0;
    const X_GRIDS: f32 = 170.0;
    const X_REPORT: f32 = 280.0;
    const X_TIME: f32 = 540.0;
    const COLOR_BLOCK_WIDTH: f32 = 12.0;
    const COLOR_BLOCK_HEIGHT: f32 = 18.0;
    const COLOR_BLOCK_TEXT_SPACING: f32 = 8.0;


    /// Create a new renderer
    pub fn new(output_dir: impl AsRef<Path>) -> Self {
        Self {
            output_dir: output_dir.as_ref().to_path_buf(),
        }
    }

    /// Render AMSAT search results to image (new dual-store API)
    pub async fn render_amsat_results(
        &self,
        results: Vec<&AmsatEntry>,
    ) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.output_dir)
            .await
            .context("Failed to create output directory")?;

        let filename = self.generate_amsat_filename(&results);
        let output_path = self.output_dir.join(&filename);

        if output_path.exists() {
            tracing::debug!("Using cached image: {:?}", output_path);
            return Ok(output_path);
        }

        let svg_content = self.generate_amsat_svg(results).await?;
        render_svg_to_png(&svg_content, &output_path).await?;

        tracing::info!("Generated satellite status image: {:?}", output_path);
        Ok(output_path)
    }

    /// Generate filename for AMSAT results
    fn generate_amsat_filename(&self, results: &Vec<&AmsatEntry>) -> String {
        let now = chrono::Utc::now();
        let minute = (now.minute() / 15) * 15;
        let floored = now
            .with_minute(minute)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap();

        let time_str = floored.format("%Y%m%d_%H%M").to_string();

        let names: Vec<String> = results
            .iter()
            .take(5)
            .map(|r| Self::normalize_sat_name(&r.api_name))
            .collect();

        let name_part = if names.is_empty() {
            "empty".to_string()
        } else if names.len() > 3 {
            format!("{}_and_{}_more", names[..2].join("_"), names.len() - 2)
        } else {
            names.join("_")
        };

        format!("sat_{}_{}.png", time_str, name_part)
    }

    /// Generate SVG for AMSAT results
    async fn generate_amsat_svg(
        &self,
        results: Vec<&AmsatEntry>,
    ) -> Result<String> {
        let mut current_y = Self::TOP_PADDING;
        let mut content = String::new();
        let now_utc = Utc::now();

        if results.is_empty() {
            content.push_str(
                r#"<text x="410" y="100" text-anchor="middle" class="table-text">No satellite data available.</text>"#,
            );
            current_y = 120.0;
        } else {
            for result in results {
                let block = self.generate_amsat_block(&result, &mut current_y, &now_utc)?;
                content.push_str(&block);
            }
        }

        let footer = self.generate_footer(current_y);
        let total_height = current_y + Self::FOOTER_HEIGHT;

        let svg = tokio::fs::read_to_string(SAT_SVG_TEMPLATE)
            .await
            .context("Failed to read SVG template")?
            .replace("{{SVG_HEIGHT}}", &total_height.to_string())
            .replace("{{CONTENT}}", &content)
            .replace("{{FOOTER}}", &footer);

        Ok(svg)
    }

    /// Generate a block for a single AMSAT entry
    fn generate_amsat_block(
        &self,
        entry: &AmsatEntry,
        current_y: &mut f32,
        now_utc: &DateTime<Utc>,
    ) -> Result<String> {
        let mut block = String::new();

        // Title: API name
        let title = &entry.api_name;
        block.push_str(&format!(
            r#"<text x="{}" y="{}" class="satellite-title">{}</text>"#,
            Self::X_CALLSIGN,
            *current_y + Self::BLOCK_TITLE_HEIGHT / 2.0,
            Self::escape_xml(title)
        ));
        block.push('\n');
        *current_y += 10.0 + Self::BLOCK_TITLE_HEIGHT / 2.0;

        // AMSAT update status
        let (status_class, status_text) = if entry.update_success {
            ("amsat-update-success", "AMSAT Update: Success")
        } else {
            ("amsat-update-failure", "AMSAT Update: Failed")
        };

        let logo_size = Self::ROW_HEIGHT * 0.6;
        let logo_x = Self::X_TIME;
        let logo_y = *current_y + (Self::ROW_HEIGHT - logo_size) / 2.0;

        block.push_str(&format!(
            r#"<image x="{}" y="{}" width="{}" height="{}" href="resources/amsat.png"/><text x="{}" y="{}" class="{}">{}</text>
"#,
            logo_x, logo_y, logo_size, logo_size,
            logo_x + logo_size + 10.0,
            *current_y + Self::ROW_HEIGHT / 2.0,
            status_class,
            status_text
        ));

        // Last update time
        let last_update_str = entry.last_updated.format("%Y-%m-%d %H:%M:%S UTC").to_string();
        let hours_ago = (now_utc.signed_duration_since(entry.last_updated)).num_hours();

        block.push_str(&format!(
            r#"<text x="{}" y="{}" class="table-text">Last update: {} ({}h ago)</text>"#,
            Self::X_CALLSIGN,
            *current_y + Self::ROW_HEIGHT / 2.0,
            last_update_str,
            hours_ago
        ));
        block.push('\n');
        *current_y += Self::ROW_HEIGHT;

        // Reports
        let total_reports = entry.total_reports();
        if total_reports == 0 {
            block.push_str(&format!(
                r#"<text x="410" y="{}" text-anchor="middle" class="table-text">No reports available.</text>"#,
                *current_y + 40.0
            ));
            block.push('\n');
            *current_y += 80.0;
            *current_y += Self::BLOCK_SPACING;
            return Ok(block);
        }

        block.push_str(&self.generate_reports_section(
            &entry.api_name,
            &entry.reports,
            current_y,
            now_utc,
        )?);

        *current_y += Self::BLOCK_SPACING;
        Ok(block)
    }
    
    
    fn normalize_sat_name(name: &str) -> String {
        name.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    }
    
    /// Generate reports section
    fn generate_reports_section(
        &self,
        _api_name: &str,
        data_blocks: &[SatelliteDataBlock],
        current_y: &mut f32,
        now_utc: &DateTime<Utc>,
    ) -> Result<String> {
        let mut section = String::new();

        // Table header
        section.push_str(&format!(
            r##"<g class="header">
<rect x="0" y="{}" width="100%" height="{}" fill="#f0f2f5" />
<text x="{}" y="{}" class="table-text header-text">Callsign</text>
<text x="{}" y="{}" class="table-text header-text">Grids</text>
<text x="{}" y="{}" class="table-text header-text">Report</text>
<text x="{}" y="{}" class="table-text header-text">Time</text>
</g>
"##,
            *current_y, Self::HEADER_HEIGHT,
            Self::X_CALLSIGN, *current_y + Self::HEADER_HEIGHT / 2.0,
            Self::X_GRIDS, *current_y + Self::HEADER_HEIGHT / 2.0,
            Self::X_REPORT, *current_y + Self::HEADER_HEIGHT / 2.0,
            Self::X_TIME, *current_y + Self::HEADER_HEIGHT / 2.0,
        ));
        *current_y += Self::HEADER_HEIGHT;

        let selected_reports = Self::select_reports_for_render(data_blocks);
        
        // Data rows
        for report in selected_reports {
                let y_pos = *current_y + Self::ROW_HEIGHT / 2.0;
                let report_color = ReportStatus::string_to_color_hex(&report.report);
                let report_text = ReportStatus::from_string(&report.report).to_string();
                
                let report_time = DateTime::parse_from_rfc3339(&report.reported_time)
                    .unwrap_or_else(|_| Utc::now().into());
                let hours_ago = (now_utc.signed_duration_since(report_time)).num_hours();
                
                let time_color = map_time_to_color(&report.reported_time, now_utc, 0.0, 12.0)
                    .unwrap_or_else(|_| "#808080".to_string());
                
                section.push_str(&format!(
                    r##"<g class="data-row">
   <text x="{}" y="{}" class="table-text">{}</text>
   <text x="{}" y="{}" class="table-text">{}</text>
   <rect x="{}" y="{}" width="{}" height="{}" fill="{}" rx="1" />
   <text x="{}" y="{}" class="table-text">{}</text>
   <rect x="{}" y="{}" width="{}" height="{}" fill="{}" rx="1" />
   <text x="{}" y="{}" class="table-text">{} ({}h ago)</text>
</g>
"##,
                    Self::X_CALLSIGN, y_pos, Self::escape_xml(&report.callsign),
                    Self::X_GRIDS, y_pos, Self::escape_xml(&report.grid_square),
                    Self::X_REPORT, y_pos - Self::COLOR_BLOCK_HEIGHT / 2.0, Self::COLOR_BLOCK_WIDTH, Self::COLOR_BLOCK_HEIGHT, report_color,
                    Self::X_REPORT + Self::COLOR_BLOCK_WIDTH + Self::COLOR_BLOCK_TEXT_SPACING, y_pos, report_text,
                    Self::X_TIME, y_pos - Self::COLOR_BLOCK_HEIGHT / 2.0, Self::COLOR_BLOCK_WIDTH, Self::COLOR_BLOCK_HEIGHT, time_color,
                    Self::X_TIME + Self::COLOR_BLOCK_WIDTH + Self::COLOR_BLOCK_TEXT_SPACING, y_pos, report.reported_time, hours_ago
                ));
                
                *current_y += Self::ROW_HEIGHT;
        }
        
        Ok(section)
    }

    fn select_reports_for_render<'a>(data_blocks: &'a [SatelliteDataBlock]) -> Vec<&'a AmsatReport> {
        let all_reports: Vec<&AmsatReport> = data_blocks
            .iter()
            .flat_map(|block| block.reports.iter())
            .collect();

        if all_reports.len() <= Self::MAX_REPORTS_PER_SATELLITE {
            return all_reports;
        }

        all_reports
            .into_iter()
            .take(Self::MAX_REPORTS_PER_SATELLITE)
            .collect()
    }
    
    /// Generate footer
    fn generate_footer(&self, footer_y: f32) -> String {
        let render_time = chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        
        format!(
            r##"    <g id="footer">
<rect x="0" y="{}" width="100%" height="{}" fill="#f0f2f5" />
<text x="50%" y="{}" class="table-text footer-text" text-anchor="middle">
    Powered by Rinko, rendered at {} BJT
</text>
</g>
"##,
            footer_y,
            Self::FOOTER_HEIGHT,
            footer_y + Self::FOOTER_HEIGHT / 2.0,
            render_time
        )
    }
    
    fn escape_xml(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}

pub struct LotwRenderer {
    output_dir: PathBuf,
}

impl LotwRenderer {
    /// Starting Y of the first data row (header band ends at y=86)
    const DATA_START_Y: f32 = 86.0;
    /// Height of each data row
    const ROW_HEIGHT: f32 = 38.0;
    /// Footer height
    const FOOTER_HEIGHT: f32 = 30.0;

    // Column X positions (match the template header positions)
    const X_EPOCH:      f32 = 20.0;
    const X_LOGS:       f32 = 230.0;
    const X_QSOS:       f32 = 300.0;
    const X_BYTES:      f32 = 420.0;
    const X_PROCESSING: f32 = 590.0;

    pub fn new(output_dir: impl AsRef<Path>) -> Self {
        Self {
            output_dir: output_dir.as_ref().to_path_buf(),
        }
    }

    pub async fn render(&self, snapshot: &LotwQueueSnapshot) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.output_dir)
            .await
            .context("Failed to create output directory")?;

        let filename = Self::generate_lotw_filename(snapshot);
        let output_path = self.output_dir.join(&filename);

        // Stable symlink / latest convenience path
        let latest_path = self.output_dir.join("lotw_latest.png");

        if output_path.exists() {
            tracing::debug!("Using cached LoTW image: {:?}", output_path);
            return Ok(latest_path);
        }

        let svg = Self::build_svg(snapshot).await?;
        render_svg_to_png(&svg, &output_path).await?;

        // Overwrite the "latest" symlink / copy
        if latest_path.exists() {
            tokio::fs::remove_file(&latest_path).await.ok();
        }
        tokio::fs::copy(&output_path, &latest_path)
            .await
            .context("Failed to copy to lotw_latest.png")?;

        tracing::info!("Generated LoTW status image: {:?}", output_path);
        Ok(latest_path)
    }

    // ── private helpers ──────────────────────────────────────────────────────

    fn generate_lotw_filename(snapshot: &LotwQueueSnapshot) -> String {
        let ts = snapshot.fetched_at.format("%Y%m%d_%H%M").to_string();
        format!("lotw_{}.png", ts)
    }

    async fn build_svg(snapshot: &LotwQueueSnapshot) -> Result<String> {
        let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M BJT").to_string();

        let rows_svg = Self::build_rows_svg(&snapshot.rows);

        let total_rows_height = snapshot.rows.len() as f32 * Self::ROW_HEIGHT;
        let footer_y     = Self::DATA_START_Y + total_rows_height;
        let footer_text_y = footer_y + Self::FOOTER_HEIGHT / 2.0;
        let svg_height   = footer_y + Self::FOOTER_HEIGHT;

        let svg = tokio::fs::read_to_string(LOTW_SVG_TEMPLATE)
            .await
            .context("Failed to read SVG template")?
            .replace("{{SVG_HEIGHT}}",     &format!("{:.0}", svg_height))
            .replace("{{GENERATED_AT}}",   &Self::escape_xml(&generated_at))
            .replace("{{ROWS}}",           &rows_svg)
            .replace("{{FOOTER_Y}}",       &format!("{:.0}", footer_y))
            .replace("{{FOOTER_TEXT_Y}}",  &format!("{:.0}", footer_text_y));

        Ok(svg)
    }

    fn build_rows_svg(rows: &[LotwQueueRow]) -> String {
        let mut out = String::new();

        for (i, row) in rows.iter().enumerate() {
            let y        = Self::DATA_START_Y + i as f32 * Self::ROW_HEIGHT;
            let mid_y    = y + Self::ROW_HEIGHT / 2.0;
            let bg_color = if i % 2 == 0 { "#ffffff" } else { "#f8f9fa" };

            // Background stripe
            out.push_str(&format!(
                r#"<rect x="0" y="{y:.0}" width="820" height="{row_height:.0}" fill="{bg}"/>"#,
                y          = y,
                row_height = Self::ROW_HEIGHT,
                bg         = bg_color,
            ));

            // Divider line
            out.push_str(&format!(
                r##"<line x1="0" y1="{:.0}" x2="820" y2="{:.0}" stroke="#eeeeee" stroke-width="1"/>"##,
                y + Self::ROW_HEIGHT,
                y + Self::ROW_HEIGHT,
            ));

            // Epoch
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="cell-text time-text">{v}</text>"#,
                x  = Self::X_EPOCH,
                my = mid_y,
                v  = Self::escape_xml(&row.epoch),
            ));

            // Logs
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="cell-text">{v}</text>"#,
                x  = Self::X_LOGS,
                my = mid_y,
                v  = row.logs,
            ));

            // QSOs – highlight background when large
            let qso_bg = if row.qsos > 10_000 { "#ffe3e3" } else { "#e7f5ff" };
            let qso_bold = if row.qsos > 10_000 { " font-weight=\"bold\"" } else { "" };
            out.push_str(&format!(
                r#"<rect x="{x}" y="{ry:.1}" width="110" height="22" fill="{bg}" rx="2"/>
<text x="{x}" y="{my:.1}" class="cell-text"{bold}>{v}</text>"#,
                x    = Self::X_QSOS,
                ry   = mid_y - 11.0,
                bg   = qso_bg,
                my   = mid_y,
                bold = qso_bold,
                v    = Self::format_number(row.qsos),
            ));

            // Bytes
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="cell-text">{v}</text>"#,
                x  = Self::X_BYTES,
                my = mid_y,
                v  = Self::format_number(row.bytes),
            ));

            // Currently-processing timestamp (line 1) + latency (line 2)
            let latency_class = if row.latency_bad { "latency-bad" } else { "latency-ok" };
            let latency_suffix = if row.latency_bad { " ●" } else { "" };
            out.push_str(&format!(
                r#"<text x="{x}" y="{ts_y:.1}" class="cell-text time-text" font-size="12">{ts}</text>
<text x="{x}" y="{lat_y:.1}" class="cell-text {cls}" font-size="11">{lat}{sfx}</text>"#,
                x      = Self::X_PROCESSING,
                ts_y   = mid_y - 8.0,
                ts     = Self::escape_xml(&row.currently_processing),
                lat_y  = mid_y + 9.0,
                cls    = latency_class,
                lat    = Self::escape_xml(&row.latency_text),
                sfx    = latency_suffix,
            ));
        }

        out
    }

    fn escape_xml(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    /// Format large numbers with thousands separators (e.g. 50_669_803 → "50,669,803")
    fn format_number(n: u64) -> String {
        let s = n.to_string();
        let mut result = String::new();
        for (i, c) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                result.push(',');
            }
            result.push(c);
        }
        result.chars().rev().collect()
    }
}

pub struct Qo100Renderer {
    output_dir: PathBuf,
}

impl Qo100Renderer {
    /// Starting Y of the first data row (header band ends at y=86)
    const DATA_START_Y: f32 = 86.0;
    /// Height of each data row
    const ROW_HEIGHT: f32 = 32.0;
    /// Footer height
    const FOOTER_HEIGHT: f32 = 30.0;
    /// Max spots to render
    const MAX_SPOTS: usize = 50;

    // Column X positions (match the template header positions)
    const X_DATETIME: f32 = 20.0;
    const X_DX:       f32 = 180.0;
    const X_FREQ:     f32 = 340.0;
    const X_COMMENTS: f32 = 420.0;
    const X_SPOTTER:  f32 = 700.0;
    const X_SOURCE:   f32 = 840.0;

    pub fn new(output_dir: impl AsRef<Path>) -> Self {
        Self {
            output_dir: output_dir.as_ref().to_path_buf(),
        }
    }

    pub async fn render(&self, snapshot: &Qo100Snapshot) -> Result<PathBuf> {
        tokio::fs::create_dir_all(&self.output_dir)
            .await
            .context("Failed to create output directory")?;

        let filename = Self::generate_qo100_filename(snapshot);
        let output_path = self.output_dir.join(&filename);

        let latest_path = self.output_dir.join("qo100_latest.png");

        if output_path.exists() {
            tracing::debug!("Using cached QO-100 image: {:?}", output_path);
            return Ok(latest_path);
        }

        let svg = Self::build_svg(snapshot).await?;
        render_svg_to_png(&svg, &output_path).await?;

        if latest_path.exists() {
            tokio::fs::remove_file(&latest_path).await.ok();
        }
        tokio::fs::copy(&output_path, &latest_path)
            .await
            .context("Failed to copy to qo100_latest.png")?;

        tracing::info!("Generated QO-100 cluster image: {:?}", output_path);
        Ok(latest_path)
    }

    fn generate_qo100_filename(snapshot: &Qo100Snapshot) -> String {
        let ts = snapshot.fetched_at.format("%Y%m%d_%H%M").to_string();
        format!("qo100_{}.png", ts)
    }

    async fn build_svg(snapshot: &Qo100Snapshot) -> Result<String> {
        let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M BJT").to_string();

        let spots = if snapshot.spots.len() > Self::MAX_SPOTS {
            &snapshot.spots[..Self::MAX_SPOTS]
        } else {
            &snapshot.spots
        };

        let rows_svg = Self::build_rows_svg(spots);

        let total_rows_height = spots.len() as f32 * Self::ROW_HEIGHT;
        let footer_y      = Self::DATA_START_Y + total_rows_height;
        let footer_text_y  = footer_y + Self::FOOTER_HEIGHT / 2.0;
        let svg_height    = footer_y + Self::FOOTER_HEIGHT;

        let svg = tokio::fs::read_to_string(QO100_SVG_TEMPLATE)
            .await
            .context("Failed to read QO-100 SVG template")?
            .replace("{{SVG_HEIGHT}}",    &format!("{:.0}", svg_height))
            .replace("{{GENERATED_AT}}",  &Self::escape_xml(&generated_at))
            .replace("{{ROWS}}",          &rows_svg)
            .replace("{{FOOTER_Y}}",      &format!("{:.0}", footer_y))
            .replace("{{FOOTER_TEXT_Y}}", &format!("{:.0}", footer_text_y));

        Ok(svg)
    }

    fn build_rows_svg(spots: &[Qo100Spot]) -> String {
        let mut out = String::new();

        for (i, spot) in spots.iter().enumerate() {
            let y     = Self::DATA_START_Y + i as f32 * Self::ROW_HEIGHT;
            let mid_y = y + Self::ROW_HEIGHT / 2.0;
            let bg    = if i % 2 == 0 { "#ffffff" } else { "#f8f9fa" };

            // Background stripe
            out.push_str(&format!(
                r#"<rect x="0" y="{y:.0}" width="920" height="{h:.0}" fill="{bg}"/>"#,
                y = y, h = Self::ROW_HEIGHT, bg = bg,
            ));

            // Divider
            out.push_str(&format!(
                r##"<line x1="0" y1="{:.0}" x2="920" y2="{:.0}" stroke="#eeeeee" stroke-width="1"/>"##,
                y + Self::ROW_HEIGHT, y + Self::ROW_HEIGHT,
            ));

            // Date/Time
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="cell-text time-text">{v}</text>"#,
                x = Self::X_DATETIME, my = mid_y, v = Self::escape_xml(&spot.datetime),
            ));

            // DX (bold + blue)
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="dx-text">{v}</text>"#,
                x = Self::X_DX, my = mid_y, v = Self::escape_xml(&spot.dx),
            ));

            // Freq
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="cell-text">{v}</text>"#,
                x = Self::X_FREQ, my = mid_y, v = Self::escape_xml(&spot.freq),
            ));

            // Comments (truncate if too long)
            let comments = if spot.comments.len() > 40 {
                format!("{}...", &spot.comments[..37])
            } else {
                spot.comments.clone()
            };
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="cell-text">{v}</text>"#,
                x = Self::X_COMMENTS, my = mid_y, v = Self::escape_xml(&comments),
            ));

            // Spotter
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="cell-text">{v}</text>"#,
                x = Self::X_SPOTTER, my = mid_y, v = Self::escape_xml(&spot.spotter),
            ));

            // Source
            out.push_str(&format!(
                r#"<text x="{x}" y="{my:.1}" class="cell-text">{v}</text>"#,
                x = Self::X_SOURCE, my = mid_y, v = Self::escape_xml(&spot.source),
            ));
        }

        out
    }

    fn escape_xml(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }
}

/// Render SVG to PNG
async fn render_svg_to_png(svg_content: &str, output_path: &Path) -> Result<()> {
    // Load fonts
    let mut fontdb = Database::new();
    fontdb.load_fonts_dir("fonts");
    tracing::debug!("Loaded {} font faces from fonts directory", fontdb.len());

    // Parse SVG with font database
    let mut options = Options::default();
    options.font_family = "Consolas".to_string();
    options.fontdb = std::sync::Arc::new(fontdb);

    let tree = Tree::from_str(svg_content, &options)
        .context("Failed to parse SVG")?;
    
    let size = tree.size();
    let width = size.width() as u32;
    let height = size.height() as u32;
    
    // Render to PNG
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .context("Failed to create pixmap")?;
    
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    
    pixmap.save_png(output_path)
        .context("Failed to save PNG")?;
    
    Ok(())
}
