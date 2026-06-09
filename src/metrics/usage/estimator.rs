use std::collections::HashMap;

/// 模型厂商
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Provider {
    OpenAI,
    Claude,
    Gemini,
    Unknown,
}

/// 厂商权重配置
#[derive(Debug, Clone)]
pub struct TokenWeights {
    /// 英文单词（每个单词）
    pub word: f64,
    /// 数字（每个连续数字串）
    pub number: f64,
    /// CJK 字符（每个字符）
    pub cjk: f64,
    /// 普通标点符号
    pub symbol: f64,
    /// 数学符号
    pub math_symbol: f64,
    /// URL 分隔符
    pub url_delim: f64,
    /// @ 符号
    pub at_sign: f64,
    /// Emoji
    pub emoji: f64,
    /// 换行符/制表符
    pub newline: f64,
    /// 空格
    pub space: f64,
    /// 基础 padding
    pub base_pad: i32,
}

impl Default for TokenWeights {
    fn default() -> Self {
        Self::openai()
    }
}

impl TokenWeights {
    pub fn openai() -> Self {
        Self {
            word: 1.02,
            number: 1.55,
            cjk: 0.85,
            symbol: 0.4,
            math_symbol: 2.68,
            url_delim: 1.0,
            at_sign: 2.0,
            emoji: 2.12,
            newline: 0.5,
            space: 0.42,
            base_pad: 0,
        }
    }

    pub fn claude() -> Self {
        Self {
            word: 1.13,
            number: 1.63,
            cjk: 1.21,
            symbol: 0.4,
            math_symbol: 4.52,
            url_delim: 1.26,
            at_sign: 2.82,
            emoji: 2.6,
            newline: 0.89,
            space: 0.39,
            base_pad: 0,
        }
    }

    pub fn gemini() -> Self {
        Self {
            word: 1.15,
            number: 2.8,
            cjk: 0.68,
            symbol: 0.38,
            math_symbol: 1.05,
            url_delim: 1.2,
            at_sign: 2.5,
            emoji: 1.08,
            newline: 1.15,
            space: 0.2,
            base_pad: 0,
        }
    }
}

/// Token 估算器
pub struct TokenEstimator {
    weights: HashMap<Provider, TokenWeights>,
}

impl Default for TokenEstimator {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenEstimator {
    pub fn new() -> Self {
        let mut weights = HashMap::new();
        weights.insert(Provider::OpenAI, TokenWeights::openai());
        weights.insert(Provider::Claude, TokenWeights::claude());
        weights.insert(Provider::Gemini, TokenWeights::gemini());
        Self { weights }
    }

    /// 根据模型名称推断厂商
    pub fn detect_provider(model: &str) -> Provider {
        let lower = model.to_lowercase();
        if lower.contains("gpt")
            || lower.contains("o1")
            || lower.contains("o3")
            || lower.contains("o4")
        {
            Provider::OpenAI
        } else if lower.contains("claude") {
            Provider::Claude
        } else if lower.contains("gemini") {
            Provider::Gemini
        } else {
            Provider::Unknown
        }
    }

    /// 获取权重配置
    fn get_weights(&self, provider: &Provider) -> &TokenWeights {
        self.weights
            .get(provider)
            .unwrap_or(self.weights.get(&Provider::OpenAI).unwrap())
    }

    /// 估算 token 数量
    pub fn estimate(&self, text: &str, provider: &Provider) -> i32 {
        if text.is_empty() {
            return 0;
        }

        let w = self.get_weights(provider);
        let mut count: f64 = 0.0;

        // 状态机：当前单词类型
        enum WordType {
            None,
            Latin,
            Number,
        }
        let mut current_type = WordType::None;

        for ch in text.chars() {
            // 1. 空格和换行符
            if ch.is_whitespace() {
                current_type = WordType::None;
                if ch == '\n' || ch == '\t' {
                    count += w.newline;
                } else {
                    count += w.space;
                }
                continue;
            }

            // 2. CJK 字符
            if is_cjk(ch) {
                current_type = WordType::None;
                count += w.cjk;
                continue;
            }

            // 3. Emoji
            if is_emoji(ch) {
                current_type = WordType::None;
                count += w.emoji;
                continue;
            }

            // 4. 拉丁字母和数字
            if ch.is_alphanumeric() {
                let is_num = ch.is_numeric();
                let new_type = if is_num {
                    WordType::Number
                } else {
                    WordType::Latin
                };

                // 单词边界检测
                match current_type {
                    WordType::None => {
                        if is_num {
                            count += w.number;
                        } else {
                            count += w.word;
                        }
                    }
                    WordType::Latin => {
                        if is_num {
                            // 字母 -> 数字，新 token
                            count += w.number;
                        }
                    }
                    WordType::Number => {
                        if !is_num {
                            // 数字 -> 字母，新 token
                            count += w.word;
                        }
                    }
                }
                current_type = new_type;
                continue;
            }

            // 5. 标点符号
            current_type = WordType::None;
            if is_math_symbol(ch) {
                count += w.math_symbol;
            } else if ch == '@' {
                count += w.at_sign;
            } else if is_url_delim(ch) {
                count += w.url_delim;
            } else {
                count += w.symbol;
            }
        }

        (count.ceil() as i32) + w.base_pad
    }

    /// 估算请求 token（兼容旧接口）
    pub fn estimate_request_tokens(&self, text: &str, model: &str) -> i32 {
        let provider = Self::detect_provider(model);
        self.estimate(text, &provider)
    }
}

/// 判断是否为 CJK 字符
fn is_cjk(ch: char) -> bool {
    let cp = ch as u32;
    // CJK 统一汉字
    (0x4E00..=0x9FFF).contains(&cp) ||
    // CJK 扩展 A
    (0x3400..=0x4DBF).contains(&cp) ||
    // 日文平假名
    (0x3040..=0x309F).contains(&cp) ||
    // 日文片假名
    (0x30A0..=0x30FF).contains(&cp) ||
    // 韩文
    (0xAC00..=0xD7A3).contains(&cp)
}

/// 判断是否为 Emoji
fn is_emoji(ch: char) -> bool {
    let cp = ch as u32;
    // Emoticons
    (0x1F600..=0x1F64F).contains(&cp) ||
    // Misc Symbols and Pictographs
    (0x1F300..=0x1F5FF).contains(&cp) ||
    // Transport and Map Symbols
    (0x1F680..=0x1F6FF).contains(&cp) ||
    // Supplemental Symbols and Pictographs
    (0x1F900..=0x1F9FF).contains(&cp) ||
    // Symbols and Pictographs Extended-A
    (0x1FA00..=0x1FAFF).contains(&cp) ||
    // Misc Symbols
    (0x2600..=0x26FF).contains(&cp) ||
    // Dingbats
    (0x2700..=0x27BF).contains(&cp)
}

/// 判断是否为数学符号
fn is_math_symbol(ch: char) -> bool {
    let cp = ch as u32;
    // Mathematical Operators
    (0x2200..=0x22FF).contains(&cp) ||
    // Supplemental Mathematical Operators
    (0x2A00..=0x2AFF).contains(&cp) ||
    // Mathematical Alphanumeric Symbols
    (0x1D400..=0x1D7FF).contains(&cp)
}

/// 判断是否为 URL 分隔符
fn is_url_delim(ch: char) -> bool {
    matches!(ch, '/' | ':' | '?' | '&' | '=' | ';' | '#' | '%')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_provider() {
        assert_eq!(TokenEstimator::detect_provider("gpt-4"), Provider::OpenAI);
        assert_eq!(
            TokenEstimator::detect_provider("gpt-4o-mini"),
            Provider::OpenAI
        );
        assert_eq!(
            TokenEstimator::detect_provider("o1-preview"),
            Provider::OpenAI
        );
        assert_eq!(
            TokenEstimator::detect_provider("claude-3-opus"),
            Provider::Claude
        );
        assert_eq!(
            TokenEstimator::detect_provider("gemini-1.5-pro"),
            Provider::Gemini
        );
        assert_eq!(
            TokenEstimator::detect_provider("unknown-model"),
            Provider::Unknown
        );
    }

    #[test]
    fn test_estimate_english() {
        let estimator = TokenEstimator::new();
        // "Hello World" 大约 2-3 个 token
        let tokens = estimator.estimate("Hello World", &Provider::OpenAI);
        assert!((2..=4).contains(&tokens));
    }

    #[test]
    fn test_estimate_chinese() {
        let estimator = TokenEstimator::new();
        // "你好世界" 大约 2-4 个 token
        let tokens = estimator.estimate("你好世界", &Provider::OpenAI);
        assert!((2..=5).contains(&tokens));
    }

    #[test]
    fn test_estimate_mixed() {
        let estimator = TokenEstimator::new();
        let text = "Hello 你好 World 世界";
        let tokens = estimator.estimate(text, &Provider::OpenAI);
        assert!((4..=8).contains(&tokens));
    }

    #[test]
    fn test_estimate_empty() {
        let estimator = TokenEstimator::new();
        assert_eq!(estimator.estimate("", &Provider::OpenAI), 0);
    }

    #[test]
    fn test_estimate_emoji() {
        let estimator = TokenEstimator::new();
        let tokens = estimator.estimate("Hello 😀 World", &Provider::OpenAI);
        assert!(tokens >= 3);
    }

    #[test]
    fn test_provider_weights_differ() {
        let estimator = TokenEstimator::new();
        let text = "你好世界 Hello World";

        let openai_tokens = estimator.estimate(text, &Provider::OpenAI);
        let claude_tokens = estimator.estimate(text, &Provider::Claude);
        let gemini_tokens = estimator.estimate(text, &Provider::Gemini);

        // 不同厂商的估算结果应该不同
        assert_ne!(openai_tokens, claude_tokens);
        assert_ne!(openai_tokens, gemini_tokens);
    }

    #[test]
    fn test_estimate_request_tokens() {
        let estimator = TokenEstimator::new();
        let tokens = estimator.estimate_request_tokens("Hello World", "gpt-4");
        assert!((2..=4).contains(&tokens));
    }

    #[test]
    fn test_is_cjk() {
        assert!(is_cjk('你'));
        assert!(is_cjk('好'));
        assert!(is_cjk('あ')); // 日文平假名
        assert!(is_cjk('ア')); // 日文片假名
        assert!(is_cjk('한')); // 韩文
        assert!(!is_cjk('A'));
        assert!(!is_cjk('1'));
    }

    #[test]
    fn test_is_emoji() {
        assert!(is_emoji('😀'));
        assert!(is_emoji('🎉'));
        assert!(!is_emoji('A'));
        assert!(!is_emoji('你'));
    }

    #[test]
    fn test_is_math_symbol() {
        assert!(is_math_symbol('∑'));
        assert!(is_math_symbol('∫'));
        assert!(is_math_symbol('∞'));
        assert!(!is_math_symbol('+'));
        assert!(!is_math_symbol('='));
    }

    #[test]
    fn test_is_url_delim() {
        assert!(is_url_delim('/'));
        assert!(is_url_delim(':'));
        assert!(is_url_delim('?'));
        assert!(is_url_delim('&'));
        assert!(!is_url_delim('a'));
    }

    #[test]
    fn test_estimate_numbers() {
        let estimator = TokenEstimator::new();
        let tokens = estimator.estimate("12345", &Provider::OpenAI);
        // 数字串应该有较高的权重
        assert!(tokens >= 2);
    }

    #[test]
    fn test_estimate_long_text() {
        let estimator = TokenEstimator::new();
        let text =
            "This is a longer text with multiple words and sentences. It should have more tokens.";
        let tokens = estimator.estimate(text, &Provider::OpenAI);
        assert!(tokens >= 15);
    }
}
