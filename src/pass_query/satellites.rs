use std::collections::HashMap;
use lazy_static::lazy_static;

#[allow(unused)]
pub struct SatelliteInfo {
    pub id: u32,
    pub aliases: Vec<String>,
}

lazy_static! {
    pub static ref SATELLITE_LIST: HashMap<String, SatelliteInfo> = {
        let mut map = HashMap::new();

        // FM sats
        map.insert(
            "AO-123".to_string(),
            SatelliteInfo {
                id: 61781,
                aliases: vec!["AO123".to_string(), "ao123".to_string(), "asrtu".to_string()],
            },
        );

        map.insert(
            "AO-91".to_string(),
            SatelliteInfo {
                id: 43017,
                aliases: vec!["AO91".to_string(), "ao91".to_string(), "Fox-1B".to_string(), "RadFxSat".to_string()],
            },
        );

        map.insert(
            "ISS".to_string(),
            SatelliteInfo {
                id: 25544,
                aliases: vec![
                    "iss".to_string(), "ISSFM".to_string(), "iss voice".to_string(), "iss-fm".to_string(),
                    "iss fm".to_string(), "iss".to_string(), "ariss".to_string(), "ariss fm".to_string(),
                    "ariss voice".to_string(), "zarya".to_string(), "iss zarya".to_string()
                ],
            },
        );

        map.insert(
            "SO-124".to_string(),
            SatelliteInfo {
                id: 62690,
                aliases: vec!["so124".to_string(), "SO124".to_string(), "HADES-R".to_string(), "hadesr".to_string(), "124".to_string()],
            },
        );

        map.insert(
            "SO-125".to_string(),
            SatelliteInfo {
                id: 63492,
                aliases: vec!["so125".to_string(), "SO125".to_string(), "HADES-ICM".to_string(), "hadesicm".to_string(), "icm".to_string(), "125".to_string()],
            },
        );

        map.insert(
            "SO-50".to_string(),
            SatelliteInfo {
                id: 27607,
                aliases: vec!["so50".to_string(), "SO50".to_string()],
            },
        );

        map
    };
}

lazy_static! {
    pub static ref SATELLITE_ALIASES: HashMap<String, Vec<String>> = {
        SATELLITE_LIST.iter()
            .map(|(name, info)| (name.clone(), info.aliases.clone()))
            .collect()
    };
}
