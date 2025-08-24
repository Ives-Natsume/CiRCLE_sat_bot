use std::{collections::HashMap, fs};
use std::sync::{
    Arc,
    RwLock,
};

type LocaleMap = HashMap<String, String>;

#[derive(Debug)]
pub struct I18n {
    current_lang: RwLock<String>,
    locales: RwLock<HashMap<String, LocaleMap>>,
}

impl I18n {
    pub fn new() -> Self {
        Self {
            current_lang: RwLock::new("en".to_string()), // 默认语言
            locales: RwLock::new(HashMap::new()),
        }
    }

    /// 加载语言包
    pub fn load_locale(&self, lang: &str, path: &str) {
        let file_str = fs::read_to_string(path).expect(format!("Failed to read file: {}", path).as_str());
        let map: LocaleMap = serde_json::from_str(&file_str).expect(format!("Failed to parse file: {}", path).as_str());

        let mut locales_guard = self.locales.write().unwrap();
        locales_guard.insert(lang.to_string(), map);
    }

    /// 切换当前语言
    pub fn set_lang(&self, lang: &str) {
        let mut lang_guard = self.current_lang.write().unwrap();
        *lang_guard = lang.to_string();
    }

    /// 获取当前语言
    pub fn get_lang(&self) -> String {
        self.current_lang.read().unwrap().clone()
    }

    /// 查找文本
    pub fn text(&self, key: &str) -> String {
        let lang = self.get_lang();
        let locales_guard = self.locales.read().unwrap();

        locales_guard
            .get(&lang)
            .and_then(|map| map.get(key))
            .cloned()
            .or_else(|| {
                // 缺失时返回默认语言
                locales_guard
                    .get("en")
                    .and_then(|map| map.get(key))
                    .cloned()
            })
            .unwrap_or_else(|| format!(">_< missing:{}", key)) // 全部缺失时返回缺失提示
    }
}

lazy_static::lazy_static! {
    pub static ref I18N: Arc<I18n> = {
        let i18n = Arc::new(I18n::new());

        // 预加载语言包
        i18n.load_locale("en", "locales/en.json");
        i18n.load_locale("zh", "locales/zh.json");

        i18n
    };
}

pub fn text(key: &str) -> String {
    I18N.text(key)
}
