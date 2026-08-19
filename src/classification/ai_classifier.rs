//! AI 分类器
//!
//! 通过调用 LLM API 对桌面图标进行智能分类。
//! 内置火山引擎豆包模型（每日免费 200 万 token），用户可自定义覆盖。

use crate::config::app_config::AiConfig;
use crate::platform::DesktopIcon;
use serde::Deserialize;

/// 默认分类列表（同时作为 AI 提示词候选和分类框模板）
const DEFAULT_CATEGORIES: &[(&str, &str, &str)] = &[
    ("browser", "浏览器", "🌐"),
    ("social", "社交聊天", "💬"),
    ("media", "影音娱乐", "🎬"),
    ("office", "办公学习", "📚"),
    ("dev", "开发工具", "💻"),
    ("games", "游戏", "🎮"),
    ("ai", "AI 工具", "🤖"),
    ("tools", "系统工具", "⚙️"),
    ("other", "其他", "📁"),
];

/// 系统提示词
const SYSTEM_PROMPT: &str = "你是桌面图标分类助手。你的任务是将桌面图标准确分类到指定类别中。\n只返回 JSON，不要包含任何其他文字。";

/// AI 返回结构
#[derive(Deserialize, Debug)]
struct AiClassifyResponse {
    results: Vec<AiClassifyItem>,
    #[serde(default)]
    new_categories: Option<Vec<String>>,
}

/// 单个图标分类结果
#[derive(Deserialize, Debug)]
struct AiClassifyItem {
    /// 图标显示名称
    name: String,
    /// 分类名称（中文）
    category: String,
}

/// AI 分类器
pub struct AiClassifier {
    client: reqwest::Client,
    config: AiConfig,
}

impl AiClassifier {
    /// 创建 AI 分类器
    pub fn new(config: AiConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_seconds as u64))
            .build()?;
        Ok(Self { client, config })
    }

    /// 分类桌面图标，返回 Vec<(图标显示名, 分类名)>
    pub async fn classify(
        &self,
        icons: &[DesktopIcon],
    ) -> anyhow::Result<Vec<(String, String)>> {
        if icons.is_empty() {
            return Ok(Vec::new());
        }

        let prompt = Self::build_prompt(icons);
        log::info!(
            "AI 分类请求: base_url={}, model={}, 图标数={}",
            &self.config.base_url,
            &self.config.model,
            icons.len()
        );

        let response_text = self.call_llm(SYSTEM_PROMPT, &prompt).await?;
        let parsed = Self::parse_response(&response_text)?;

        // 构建结果映射，未分类的图标自动归入「其他」
        let mut results: Vec<(String, String)> = Vec::new();
        for icon in icons {
            let category = parsed
                .results
                .iter()
                .find(|item| item.name == icon.display_name)
                .map(|item| item.category.clone())
                .unwrap_or_else(|| "其他".to_string());
            results.push((icon.display_name.clone(), category));
        }

        // 处理新分类（如果 AI 建议新增类别）
        if let Some(new_cats) = &parsed.new_categories {
            if !new_cats.is_empty() {
                log::info!("AI 建议新增分类: {:?}", new_cats);
            }
        }

        Ok(results)
    }

    /// 构建用户提示词
    fn build_prompt(icons: &[DesktopIcon]) -> String {
        let category_names: Vec<&str> = DEFAULT_CATEGORIES
            .iter()
            .map(|(_, name, _)| *name)
            .collect();

        let mut prompt = format!(
            "请将以下桌面图标分类到这些类别中：\n{}\n\n图标列表：\n",
            category_names.join(", ")
        );

        for (i, icon) in icons.iter().enumerate() {
            let target = icon
                .target_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            prompt.push_str(&format!(
                "{}. {} (目标: {})\n",
                i + 1,
                icon.display_name,
                target
            ));
        }

        prompt.push_str(
            "\n返回格式（严格 JSON）：\n{\"results\": [{\"name\": \"Google Chrome\", \"category\": \"浏览器\"}, ...], \"new_categories\": []}\n\n规则：\n- category 必须是上述类别之一\n- 无法归类的放入\"其他\"\n- 如果你认为需要新增类别，放入 new_categories 数组",
        );

        prompt
    }

    /// 调用 LLM API（带重试机制）
    async fn call_llm(&self, system: &str, user: &str) -> anyhow::Result<String> {
        let url = format!("{}/chat/completions", &self.config.base_url);
        let model = &self.config.model;

        let request_body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user}
            ],
            "temperature": 0.1,
            "max_tokens": 1024,
            "stream": false,
            "top_p": 0.7
        });
        
        log::info!("[AI] 请求 URL: {}", url);
        log::info!("[AI] 请求体: {}", serde_json::to_string_pretty(&request_body).unwrap_or_default());

        let mut last_error = None;
        let mut delay = std::time::Duration::from_secs(1);

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                log::warn!(
                    "AI 请求失败，{}ms 后重试 ({}/{})",
                    delay.as_millis(),
                    attempt,
                    self.config.max_retries
                );
                tokio::time::sleep(delay).await;
                delay *= 2;
            }

            match self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.config.api_key))
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await
            {
                Ok(response) => {
                    let status = response.status();
                    log::info!("[AI] HTTP 响应状态码: {}", status);
                    
                    if !status.is_success() {
                        let body = response.text().await.unwrap_or_default();
                        log::error!("API 请求失败: HTTP {}, body: {}", status, body);
                        last_error = Some(anyhow::anyhow!(
                            "API 请求失败: HTTP {} - {}",
                            status,
                            body
                        ));
                        continue;
                    }

                    let response_text = response.text().await?;
                    log::info!("[AI] 响应内容长度: {} 字节", response_text.len());
                    
                    if response_text.is_empty() {
                        log::error!("API 返回空内容，URL: {}", url);
                        last_error = Some(anyhow::anyhow!(
                            "API 返回空内容 (HTTP {})",
                            status
                        ));
                        continue;
                    }
                    
                    let body: serde_json::Value = serde_json::from_str(&response_text)
                        .map_err(|e| anyhow::anyhow!("API 返回非法 JSON: {}, 内容: {}", e, &response_text[..response_text.len().min(500)]))?;
                    
                    let content = body["choices"][0]["message"]["content"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("API 返回缺少 choices/message/content: {}", &response_text[..response_text.len().min(500)]))?;

                    return Ok(content.to_string());
                }
                Err(error) => {
                    last_error = Some(anyhow::anyhow!("网络请求失败: {}", error));
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("AI 请求失败")))
    }

    /// 解析 AI 返回 JSON
    fn parse_response(json_str: &str) -> anyhow::Result<AiClassifyResponse> {
        // 清理 markdown 代码块标记（三个反引号）
        let cleaned = json_str
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        
        serde_json::from_str(cleaned).map_err(|error| {
            log::error!("AI 返回 JSON 解析失败: {}", error);
            log::error!("原始响应: {}", json_str);
            log::error!("清理后的内容: {}", cleaned);
            anyhow::anyhow!("AI 返回格式错误: {}", error)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_prompt_format() {
        let icons = vec![
            DesktopIcon {
                display_name: "Chrome".to_string(),
                path: "C:\\Chrome.lnk".into(),
                target_path: Some("C:\\Program Files\\Chrome\\chrome.exe".into()),
            },
            DesktopIcon {
                display_name: "VSCode".to_string(),
                path: "C:\\VSCode.lnk".into(),
                target_path: Some("C:\\VSCode\\Code.exe".into()),
            },
        ];

        let prompt = AiClassifier::build_prompt(&icons);
        assert!(prompt.contains("浏览器"));
        assert!(prompt.contains("开发工具"));
        assert!(prompt.contains("Chrome"));
        assert!(prompt.contains("VSCode"));
        assert!(prompt.contains("1."));
        assert!(prompt.contains("2."));
    }

    #[test]
    fn test_parse_response_valid() {
        let json = r#"{"results": [{"name": "Chrome", "category": "浏览器"}, {"name": "VSCode", "category": "开发工具"}], "new_categories": []}"#;
        let parsed = AiClassifier::parse_response(json).unwrap();
        assert_eq!(parsed.results.len(), 2);
        assert_eq!(parsed.results[0].name, "Chrome");
        assert_eq!(parsed.results[0].category, "浏览器");
    }

    #[test]
    fn test_parse_response_new_categories() {
        let json = r#"{"results": [{"name": "Steam", "category": "游戏"}], "new_categories": ["设计工具"]}"#;
        let parsed = AiClassifier::parse_response(json).unwrap();
        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.new_categories, Some(vec!["设计工具".to_string()]));
    }

    #[test]
    fn test_parse_response_malformed_json() {
        let json = r#"{"invalid": "json"}"#;
        let result = AiClassifier::parse_response(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_response_missing_icons() {
        let json = r#"{"results": [{"name": "Chrome", "category": "浏览器"}]}"#;
        let parsed = AiClassifier::parse_response(json).unwrap();
        assert_eq!(parsed.results.len(), 1);
    }
}
