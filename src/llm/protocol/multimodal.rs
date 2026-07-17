//! 多模态内容转换工具：data URL 与 base64 互转。
//!
//! IR 的 `ImageUrl.url` 统一承载 http(s) URL 或
//! `data:{media_type};base64,{data}` 形式的 data URL（media_type 编码在前缀，
//! 跨协议往返不丢）。本模块负责 data URL 的拆解与构造，供 Anthropic image
//! block（`source`）与 Responses `input_image` / OpenAI `image_url` 互转使用。

/// 解析 data URL `data:image/png;base64,xxxx` → `(media_type, data)`。
/// 非 data URL（如 http(s) 链接）返回 None。
pub fn parse_data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let semi = rest.find(';')?;
    let (media_type, after) = rest.split_at(semi);
    let data = after.strip_prefix(";base64,")?;
    Some((media_type, data))
}

/// 构造 data URL：`data:{media_type};base64,{data}`。
pub fn build_data_url(media_type: &str, data: &str) -> String {
    format!("data:{};base64,{}", media_type, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_build_roundtrip() {
        let url = "data:image/png;base64,aGVsbG8=";
        let (mt, data) = parse_data_url(url).unwrap();
        assert_eq!(mt, "image/png");
        assert_eq!(data, "aGVsbG8=");
        assert_eq!(build_data_url(mt, data), url);
    }

    #[test]
    fn parse_returns_none_for_http_url() {
        assert_eq!(parse_data_url("https://example.com/x.png"), None);
        assert_eq!(parse_data_url("data:image/png;aGVsbG8="), None); // 缺 base64,
    }
}
