use actix_web::{HttpRequest, HttpResponse};
use serde::Deserialize;

/// 输入验证工具
pub struct InputValidator;

impl InputValidator {
    /// 验证名称（长度限制，防止注入）
    pub fn validate_name(name: &str) -> Result<(), String> {
        if name.is_empty() {
            return Err("名称不能为空".to_string());
        }
        if name.len() > 100 {
            return Err("名称长度不能超过100个字符".to_string());
        }
        // 检查是否包含危险字符
        if name.contains('<') || name.contains('>') || name.contains('{') || name.contains('}') {
            return Err("名称包含非法字符".to_string());
        }
        Ok(())
    }

    /// 验证 URL 格式
    pub fn validate_url(url: &str) -> Result<(), String> {
        if url.is_empty() {
            return Err("URL不能为空".to_string());
        }
        if url.len() > 500 {
            return Err("URL长度不能超过500个字符".to_string());
        }
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("URL必须以 http:// 或 https:// 开头".to_string());
        }
        // 基本的 URL 合法性检查
        if url.contains(' ') || url.contains('\n') || url.contains('\r') {
            return Err("URL包含非法字符".to_string());
        }
        Ok(())
    }

    /// 验证 API 前缀
    pub fn validate_prefix(prefix: &str) -> Result<(), String> {
        if prefix.is_empty() {
            return Err("前缀不能为空".to_string());
        }
        if prefix.len() > 100 {
            return Err("前缀长度不能超过100个字符".to_string());
        }
        if !prefix.starts_with('/') {
            return Err("前缀必须以 / 开头".to_string());
        }
        // 只允许字母、数字、下划线、斜杠、连字符
        if !prefix.chars().all(|c| c.is_alphanumeric() || c == '/' || c == '_' || c == '-') {
            return Err("前缀只能包含字母、数字、/、_、-".to_string());
        }
        Ok(())
    }

    /// 验证 API Key（可选字段）
    pub fn validate_api_key(key: &str) -> Result<(), String> {
        if key.is_empty() {
            return Ok(()); // 可以为空
        }
        if key.len() > 500 {
            return Err("API Key长度不能超过500个字符".to_string());
        }
        // 检查是否包含控制字符
        if key.chars().any(|c| c.is_control()) {
            return Err("API Key包含非法字符".to_string());
        }
        Ok(())
    }

    /// 验证密码强度
    pub fn validate_password(password: &str) -> Result<(), String> {
        if password.len() < 6 {
            return Err("密码长度不能少于6个字符".to_string());
        }
        if password.len() > 128 {
            return Err("密码长度不能超过128个字符".to_string());
        }
        Ok(())
    }

    /// 验证描述文本
    pub fn validate_description(desc: &str) -> Result<(), String> {
        if desc.len() > 500 {
            return Err("描述长度不能超过500个字符".to_string());
        }
        Ok(())
    }

    /// 验证 token 限额
    pub fn validate_token_limit(limit: u64) -> Result<(), String> {
        // 0 表示无上限，其他值有合理范围
        if limit > 0 && limit < 100 {
            return Err("Token限额不能小于100".to_string());
        }
        Ok(())
    }

    /// 验证超时时间
    pub fn validate_timeout(timeout: u64) -> Result<(), String> {
        if timeout < 1 {
            return Err("超时时间不能小于1秒".to_string());
        }
        if timeout > 3600 {
            return Err("超时时间不能超过3600秒".to_string());
        }
        Ok(())
    }
}
