use crate::models::AppConfig;
use anyhow::{Context, Result};
use std::path::PathBuf;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::{info, warn};

const DEFAULT_CONFIG_FILE: &str = "config.toml";

pub struct ConfigManager {
    config_path: PathBuf,
    /// 防止并发写配置导致文件覆盖或损坏
    write_lock: Mutex<()>,
}

impl ConfigManager {
    pub fn new(config_path: Option<&str>) -> Self {
        let path = config_path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_FILE));
        Self { config_path: path, write_lock: Mutex::new(()) }
    }

    /// 加载配置文件，如果不存在则创建默认配置
    pub async fn load(&self) -> Result<AppConfig> {
        if self.config_path.exists() {
            let content = fs::read_to_string(&self.config_path)
                .await
                .with_context(|| format!("读取配置文件失败: {:?}", self.config_path))?;
            let config: AppConfig = toml::from_str(&content)
                .with_context(|| format!("解析配置文件失败: {:?}", self.config_path))?;
            info!("已加载配置文件: {:?}", self.config_path);
            Ok(config)
        } else {
            warn!("配置文件不存在，创建默认配置: {:?}", self.config_path);
            let config = AppConfig::default();
            self.save(&config).await?;
            Ok(config)
        }
    }

    /// 获取配置文件所在目录
    pub fn config_dir(&self) -> PathBuf {
        self.config_path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// 保存配置到文件（串行化写入，防止并发覆盖）
    pub async fn save(&self, config: &AppConfig) -> Result<()> {
        let _guard = self.write_lock.lock().await;
        let content = toml::to_string_pretty(config)
            .context("序列化配置失败")?;
        // 确保父目录存在
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).await.ok();
        }
        fs::write(&self.config_path, content)
            .await
            .with_context(|| format!("写入配置文件失败: {:?}", self.config_path))?;
        info!("已保存配置文件: {:?}", self.config_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_load_default_config() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        // 删除文件以便测试默认配置创建
        tokio::fs::remove_file(path).await.ok();

        let manager = ConfigManager::new(Some(path));
        let config = manager.load().await.unwrap();

        assert_eq!(config.listen_addr, "0.0.0.0");
        assert_eq!(config.listen_port, 8080);
        assert_eq!(config.admin_password, "admin123");
        assert!(config.endpoints.is_empty());

        // 验证文件已创建
        assert!(Path::new(path).exists());
    }

    #[tokio::test]
    async fn test_save_and_load_config() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap();

        let manager = ConfigManager::new(Some(path));
        let mut config = AppConfig::default();
        config.listen_port = 9090;
        config.admin_password = "newpassword".to_string();

        manager.save(&config).await.unwrap();
        let loaded = manager.load().await.unwrap();

        assert_eq!(loaded.listen_port, 9090);
        assert_eq!(loaded.admin_password, "newpassword");
    }
}
