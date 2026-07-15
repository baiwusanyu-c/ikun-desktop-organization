//! AI 分类器

#![allow(dead_code)] // AI 分类器，部分方法在当前构建路径未使用
//!
//! 通过调用 LLM API 对桌面图标进行智能分类。
//! TODO: 实现 HTTP 请求和 AI 分类逻辑。

use crate::config::app_config::{AiConfig, CategoryRule};
use crate::platform::DesktopIcon;

/// AI 分类器
pub struct AiClassifier {
    config: AiConfig,
}

impl AiClassifier {
    pub fn new(config: AiConfig) -> Self {
        Self { config }
    }

    /// 使用 AI 分类桌面图标
    ///
    /// TODO: 实现以下流程：
    /// 1. 构建 prompt（包含图标列表和现有分类规则）
    /// 2. 调用 LLM API（POST /chat/completions）
    /// 3. 解析响应（JSON 格式的分类结果）
    /// 4. 返回分类映射
    pub async fn classify(
        &self,
        icons: &[DesktopIcon],
        _existing_rules: &[CategoryRule],
    ) -> anyhow::Result<Vec<(String, String)>> {
        if !self.config.enabled {
            anyhow::bail!("AI 分类未启用");
        }

        if self.config.api_key.is_empty() {
            anyhow::bail!("AI API Key 未配置");
        }

        // TODO: 实现实际 AI 调用
        log::warn!("AI 分类功能尚未实现，返回空结果");
        let _ = icons;
        Ok(Vec::new())
    }
}
