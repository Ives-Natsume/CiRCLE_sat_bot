use reqwest;
use scraper::{Html, Selector};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StatusFlag {
    pub report_nums: u8,
    pub description: String,
}

impl StatusFlag {
    pub fn match_status_with_color(color: &str, nums: u8) -> Option<StatusFlag> {
        match color {
            "#4169E1" => Some(StatusFlag { report_nums: nums, description: "Transponder/Repeater Active".to_string() }),
            "yellow" => Some(StatusFlag { report_nums: nums, description: "Telemetry/Beacon Only".to_string() }),
            "red" => Some(StatusFlag { report_nums: nums, description: "No Signal".to_string() }),
            "orange" => Some(StatusFlag { report_nums: nums, description: "Conflictng Reports".to_string() }),
            "#9900FF" => Some(StatusFlag { report_nums: nums, description: "ISS Crew(Voice) Active".to_string() }),
            _ => Some(StatusFlag { report_nums: 0, description: "Unknown Status".to_string() }),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SatelliteStatus {
    pub name: String,
    pub status: Vec<Vec<StatusFlag>>,
}

impl SatelliteStatus {
    pub fn new(name: String, status: Vec<Vec<StatusFlag>>) -> Self {
        SatelliteStatus { name, status }
    }
}

// run the amsat_module
pub fn run_amsat_module() {
    // download the target HTML document
    let response = reqwest::blocking::get("https://www.amsat.org/status/");
    // get the HTML content from the request response
    let html_content = response.unwrap().text().unwrap();

    // parse the HTML content to extract satellite status
    let satellite_status = get_satellite_status(&html_content);

    // save the satellite status to a json file
    let json_content = serde_json::to_string_pretty(&satellite_status).expect("Unable to serialize satellite status to JSON");
    std::fs::write("amsat_status.json", json_content).expect("Unable to write file");
    println!("Satellite status saved to amsat_status.json");

    // save the HTML content to a file
    std::fs::write("amsat_status.html", html_content).expect("Unable to write file");
    println!("HTML content saved to amsat_status.html");
}

// Get all Satellite name
pub fn get_satellite_names(html: &str) -> Vec<String> {
    let mut names = Vec::new();
    let document = Html::parse_document(&html);

    // 选择器：找到 <select name="SatName"> 中的所有 <option>
    let select_selector = Selector::parse(r#"select[name="SatName"] option"#).unwrap();

    // 提取所有卫星名称
    for option in document.select(&select_selector) {
        if let Some(value) = option.value().attr("value") {
            if !value.is_empty() {
                //println!("{}", value);
                names.push(value.to_string());
            }
        }
    }

    names
}

// get all Satellite status
pub fn get_satellite_status(html: &str) -> Vec<SatelliteStatus> {
    let document = Html::parse_document(&html);

    let tr_sel = Selector::parse("tr").unwrap();
    let td_sel = Selector::parse("td").unwrap();
    let sat_sel = Selector::parse(r#"td[align="right"] > a"#).unwrap();

    // all satellite names
    let all_sat_list = get_satellite_names(html);

    let mut current_sat = String::new();
    let mut groups: Vec<SatelliteStatus> = Vec::new();

    for tr in document.select(&tr_sel) {
        // get all <td> elements in the current <tr>
        let tds: Vec<_> = tr.select(&td_sel).collect();

        // get the first <td> element as the satellite name
        if let Some(sat_name_elem) = tds[0].select(&sat_sel).next() {
            let sat_name = sat_name_elem.text().collect::<Vec<_>>().join(" ");
            //check if the satellite name is valid
            if all_sat_list.contains(&sat_name) {
                current_sat = sat_name;
                // add the satellite if it not exists
                if !groups.iter().any(|g| g.name == current_sat) {
                    groups.push(SatelliteStatus::new(current_sat.clone(), Vec::new()));
                }
            }
        }
        else {
            // skip if the first <td> is not a satellite name
            continue;
        }
        // get the rest of the <td> elements as the status
        // extract the status colors to match with the status flags
        let status_colors: Vec<String> = tds.iter()
            .skip(9) // skip the first <td> which is the satellite name
            .filter_map(|td| td.value().attr("bgcolor").map(|s| s.to_string()))
            .collect();
        // get the report numbers
        let report_nums: Vec<String> = tds.iter()
            .skip(9) // skip the first <td> which is the satellite name
            .filter_map(|td| td.text().next().map(|s| s.to_string()))
            .collect();
        // map the status colors and report numbers to StatusFlag
        let status_flags: Vec<Vec<StatusFlag>> = status_colors.iter()
            .zip(report_nums.iter())
            .map(|(color, nums)| {
                if let Some(flag) = StatusFlag::match_status_with_color(color, nums.parse().unwrap_or(0)) {
                    vec![flag]
                } else {
                    vec![StatusFlag { report_nums: 0, description: "Unknown Status".to_string() }]
                }
            })
            .collect();
        // if the current satellite is not empty, add the status flags to the current group
        if !current_sat.is_empty() {
            // find the group with the current satellite name
            if let Some(group) = groups.iter_mut().find(|g| g.name == current_sat) {
                group.status = status_flags;
            }
        }
    }

    // return the groups
    groups
}
