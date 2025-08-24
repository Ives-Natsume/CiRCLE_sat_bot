use std::{
    fs,
    io::{self, Write},
    path::Path,
};
use indexmap::IndexMap;
use serde_json::{Value, Map};

fn main() -> io::Result<()> {
    // Configure available languages and their file paths
    let languages = vec![
        ("en", "locales/en.json"),
        ("zh", "locales/zh.json"),
    ];

    println!("=== I18n Translation Adder ===");
    println!("Press Enter to exit at any time.\n");

    loop {
        print!("Enter new key: ");
        io::stdout().flush()?;
        let mut key = String::new();
        io::stdin().read_line(&mut key)?;
        let key = key.trim();

        if key.is_empty() {
            println!("Exiting...");
            break;
        }

        // Verify key doesn't already exist
        for (lang, path) in &languages {
            if let Ok(content) = fs::read_to_string(path) {
                if let Ok(value) = serde_json::from_str::<Value>(&content) {
                    if value.as_object().and_then(|obj| obj.get(key)).is_some() {
                        println!("[WARN] Key '{}' already exists in {}!", key, lang);
                        return Ok(());
                    }
                }
            }
        }

        // Collect translations for each language
        let mut translations = IndexMap::new();
        for (lang, _path) in &languages {
            print!("Enter '{}' translation for '{}': ", lang, key);
            io::stdout().flush()?;
            let mut value = String::new();
            io::stdin().read_line(&mut value)?;
            translations.insert(*lang, value.trim().to_string());
        }

        // Update files while preserving key order
        for (lang, path) in &languages {
            update_file_ordered(path, key, &translations[lang])?;
            println!("Added to {}: {}", lang, path);
        }

        println!("\nTranslation added successfully!\n");
    }
    Ok(())
}

#[allow(unused_mut)]
fn update_file_ordered(path: &str, key: &str, value: &str) -> io::Result<()> {
    let mut map: Map<String, Value> = if Path::new(path).exists() {
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Map::new()
    };

    let mut ordered_map = IndexMap::new();
    for (k, v) in map.iter() {
        ordered_map.insert(k.clone(), v.clone());
    }
    
    // add new key-value pair
    ordered_map.insert(key.to_string(), Value::String(value.to_string()));
    
    // sort keys in alphabetical order
    let mut sorted_keys: Vec<_> = ordered_map.keys().collect();
    sorted_keys.sort();
    
    // create ordered map with sorted keys
    let mut sorted_map = Map::new();
    for k in sorted_keys {
        if let Some(v) = ordered_map.get(k) {
            sorted_map.insert(k.clone(), v.clone());
        }
    }

    let json = serde_json::to_string_pretty(&sorted_map)?;
    fs::write(path, json)?;
    Ok(())
}