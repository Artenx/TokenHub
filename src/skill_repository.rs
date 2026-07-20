use crate::models::{LocalSkill, SkillImportPreview, SkillOrigin, SkillRepositoryConfig};
use anyhow::{bail, Context, Result};
use chrono::{Duration, Utc};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;
use zip::ZipArchive;

const PREVIEW_TTL_MINUTES: i64 = 15;

#[derive(Debug, Clone)]
pub struct PreparedSkillPackage {
    pub directory_name: String,
    pub name: String,
    pub description: String,
    pub skill_md_summary: String,
    pub files: Vec<PreparedSkillFile>,
}

#[derive(Debug, Clone)]
pub struct PreparedSkillFile {
    pub relative_path: PathBuf,
    pub contents: Vec<u8>,
}

pub fn repository_root(config_dir: &Path, config: &SkillRepositoryConfig) -> Result<PathBuf> {
    let root = Path::new(&config.root_dir);
    if root.is_absolute() || root.components().any(|component| matches!(component, Component::ParentDir)) {
        bail!("技能仓库根目录必须是配置目录下的相对路径");
    }
    Ok(config_dir.join(root))
}

pub fn scan_local_skills(root: &Path, config: &SkillRepositoryConfig) -> Result<Vec<LocalSkill>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut skills = Vec::new();
    for entry in fs::read_dir(root).with_context(|| format!("读取技能仓库目录失败: {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let directory_name = entry.file_name().to_string_lossy().to_string();
        let id = format!("local:{}", directory_name);
        match read_skill_directory(&path, config) {
            Ok(package) => skills.push(LocalSkill {
                id,
                directory_name,
                name: package.name,
                description: package.description,
                skill_md_summary: package.skill_md_summary,
                file_count: package.files.len(),
                validation_status: "valid".to_string(),
                validation_message: None,
                source: None,
                imported_at: None,
            }),
            Err(error) => skills.push(LocalSkill {
                id,
                directory_name: directory_name.clone(),
                name: directory_name,
                description: String::new(),
                skill_md_summary: String::new(),
                file_count: 0,
                validation_status: "invalid".to_string(),
                validation_message: Some(error.to_string()),
                source: None,
                imported_at: None,
            }),
        }
    }
    skills.sort_by(|left, right| left.directory_name.cmp(&right.directory_name));
    Ok(skills)
}

pub fn preview_zip_archive(
    archive: &[u8],
    source: SkillOrigin,
    root: &Path,
    config: &SkillRepositoryConfig,
) -> Result<(SkillImportPreview, PreparedSkillPackage)> {
    let mut zip = ZipArchive::new(Cursor::new(archive)).context("技能包归档格式无效")?;
    let mut skill_roots = Vec::new();
    let mut total_size = 0_u64;
    let mut files = Vec::new();

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_string();
        if entry.is_dir() {
            continue;
        }
        if entry.unix_mode().map(|mode| mode & 0o170000 == 0o120000).unwrap_or(false) {
            bail!("技能包包含符号链接: {}", name);
        }
        let enclosed = entry.enclosed_name().ok_or_else(|| anyhow::anyhow!("技能包包含不安全路径: {}", name))?;
        if enclosed.components().count() < 2 {
            bail!("技能包文件必须位于技能根目录内: {}", name);
        }
        if entry.size() > config.max_file_size_bytes {
            bail!("技能包文件超过单文件容量上限: {}", name);
        }
        total_size = total_size.saturating_add(entry.size());
        if total_size > config.max_total_size_bytes {
            bail!("技能包超过总容量上限");
        }
        if files.len() >= config.max_file_count {
            bail!("技能包超过文件数量上限");
        }
        let relative_path = enclosed.to_path_buf();
        if relative_path.file_name().is_some_and(|file_name| file_name == "SKILL.md") {
            skill_roots.push(relative_path.parent().unwrap().to_path_buf());
        }
        let mut contents = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut contents)?;
        files.push(PreparedSkillFile { relative_path, contents });
    }

    if skill_roots.len() != 1 {
        bail!("技能包应包含唯一的根目录 SKILL.md");
    }
    let archive_root = skill_roots.pop().unwrap();
    let directory_name = archive_root.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| is_safe_directory_name(name))
        .ok_or_else(|| anyhow::anyhow!("技能根目录名称无效"))?
        .to_string();

    let mut package_files = Vec::with_capacity(files.len());
    for file in files {
        let relative_path = file.relative_path.strip_prefix(&archive_root)
            .map_err(|_| anyhow::anyhow!("技能包包含技能根目录之外的文件"))?
            .to_path_buf();
        if relative_path.as_os_str().is_empty() {
            bail!("技能包包含无效文件路径");
        }
        package_files.push(PreparedSkillFile { relative_path, contents: file.contents });
    }
    let package = package_from_files(directory_name.clone(), package_files, config)?;
    let preview = SkillImportPreview {
        id: Uuid::new_v4().to_string(),
        target_directory_name: directory_name,
        source,
        files: package.files.iter().map(|file| file.relative_path.to_string_lossy().to_string()).collect(),
        valid: true,
        validation_message: None,
        conflict: root.join(&package.directory_name).exists(),
        expires_at: Utc::now() + Duration::minutes(PREVIEW_TTL_MINUTES),
    };
    Ok((preview, package))
}

pub fn import_skill_package(root: &Path, package: &PreparedSkillPackage, replace: bool) -> Result<PathBuf> {
    validate_package(package)?;
    fs::create_dir_all(root).with_context(|| format!("创建技能仓库目录失败: {}", root.display()))?;

    let target = root.join(&package.directory_name);
    let staging = root.join(format!(".{}-{}.staging", package.directory_name, Uuid::new_v4()));
    fs::create_dir(&staging)?;
    let result = write_package(&staging, package);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    if !target.exists() {
        fs::rename(&staging, &target).context("提交技能包失败")?;
        return Ok(target);
    }
    if !replace {
        let _ = fs::remove_dir_all(&staging);
        bail!("技能目录已存在: {}", package.directory_name);
    }

    let backup = root.join(format!(".{}-{}.backup", package.directory_name, Uuid::new_v4()));
    fs::rename(&target, &backup).context("备份现有技能包失败")?;
    if let Err(error) = fs::rename(&staging, &target) {
        let _ = fs::rename(&backup, &target);
        let _ = fs::remove_dir_all(&staging);
        return Err(error).context("替换技能包失败");
    }
    fs::remove_dir_all(&backup).context("清理旧技能包备份失败")?;
    Ok(target)
}

pub fn delete_skill_package(root: &Path, directory_name: &str, confirmation: &str) -> Result<()> {
    if confirmation != directory_name {
        bail!("删除确认内容与技能目录不匹配");
    }
    if !is_safe_directory_name(directory_name) {
        bail!("技能目录名称无效");
    }
    let target = root.join(directory_name);
    if !target.is_dir() {
        bail!("技能包不存在: {}", directory_name);
    }
    fs::remove_dir_all(target).context("删除技能包失败")
}

fn read_skill_directory(directory: &Path, config: &SkillRepositoryConfig) -> Result<PreparedSkillPackage> {
    let directory_name = directory.file_name().and_then(|name| name.to_str())
        .filter(|name| is_safe_directory_name(name))
        .ok_or_else(|| anyhow::anyhow!("技能目录名称无效"))?
        .to_string();
    let mut files = Vec::new();
    collect_directory_files(directory, directory, config, &mut files)?;
    package_from_files(directory_name, files, config)
}

fn collect_directory_files(
    root: &Path,
    directory: &Path,
    config: &SkillRepositoryConfig,
    files: &mut Vec<PreparedSkillFile>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!("技能包包含符号链接: {}", path.display());
        }
        if file_type.is_dir() {
            collect_directory_files(root, &path, config, files)?;
            continue;
        }
        if !file_type.is_file() {
            bail!("技能包包含不支持的文件类型: {}", path.display());
        }
        if files.len() >= config.max_file_count {
            bail!("技能包超过文件数量上限");
        }
        let contents = fs::read(&path)?;
        if contents.len() as u64 > config.max_file_size_bytes {
            bail!("技能包文件超过单文件容量上限: {}", path.display());
        }
        files.push(PreparedSkillFile {
            relative_path: path.strip_prefix(root).unwrap().to_path_buf(),
            contents,
        });
    }
    let total_size: u64 = files.iter().map(|file| file.contents.len() as u64).sum();
    if total_size > config.max_total_size_bytes {
        bail!("技能包超过总容量上限");
    }
    Ok(())
}

fn package_from_files(
    directory_name: String,
    files: Vec<PreparedSkillFile>,
    config: &SkillRepositoryConfig,
) -> Result<PreparedSkillPackage> {
    let skill_md = files.iter().find(|file| file.relative_path == Path::new("SKILL.md"))
        .ok_or_else(|| anyhow::anyhow!("技能包缺少根目录 SKILL.md"))?;
    if files.iter().filter(|file| file.relative_path.file_name().is_some_and(|name| name == "SKILL.md")).count() != 1 {
        bail!("技能包应包含唯一的根目录 SKILL.md");
    }
    let content = std::str::from_utf8(&skill_md.contents).context("SKILL.md 必须为 UTF-8 文本")?;
    let (name, description, skill_md_summary) = parse_skill_metadata(content, &directory_name);
    let package = PreparedSkillPackage { directory_name, name, description, skill_md_summary, files };
    validate_package_with_config(&package, config)?;
    Ok(package)
}

fn validate_package(package: &PreparedSkillPackage) -> Result<()> {
    if !is_safe_directory_name(&package.directory_name) {
        bail!("技能目录名称无效");
    }
    if package.files.is_empty() || !package.files.iter().any(|file| file.relative_path == Path::new("SKILL.md")) {
        bail!("技能包缺少根目录 SKILL.md");
    }
    for file in &package.files {
        if file.relative_path.is_absolute() || file.relative_path.components().any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
            bail!("技能包包含不安全路径: {}", file.relative_path.display());
        }
    }
    Ok(())
}

fn validate_package_with_config(package: &PreparedSkillPackage, config: &SkillRepositoryConfig) -> Result<()> {
    validate_package(package)?;
    if package.files.len() > config.max_file_count {
        bail!("技能包超过文件数量上限");
    }
    let mut total_size = 0_u64;
    for file in &package.files {
        if file.contents.len() as u64 > config.max_file_size_bytes {
            bail!("技能包文件超过单文件容量上限: {}", file.relative_path.display());
        }
        total_size = total_size.saturating_add(file.contents.len() as u64);
    }
    if total_size > config.max_total_size_bytes {
        bail!("技能包超过总容量上限");
    }
    Ok(())
}

fn write_package(target: &Path, package: &PreparedSkillPackage) -> Result<()> {
    for file in &package.files {
        let path = target.join(&file.relative_path);
        let parent = path.parent().ok_or_else(|| anyhow::anyhow!("技能包文件路径无效"))?;
        fs::create_dir_all(parent)?;
        fs::write(path, &file.contents)?;
    }
    Ok(())
}

fn parse_skill_metadata(content: &str, fallback_name: &str) -> (String, String, String) {
    let mut lines = content.lines().map(str::trim).filter(|line| !line.is_empty());
    let name = lines.find_map(|line| line.strip_prefix("# ").map(str::to_string))
        .unwrap_or_else(|| fallback_name.to_string());
    let description = content.lines().map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("---"))
        .unwrap_or_default()
        .to_string();
    let summary = content.chars().take(500).collect();
    (name, description, summary)
}

fn is_safe_directory_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && Path::new(name).components().count() == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{SkillRepositoryConfig, SkillSourceType};
    use std::io::Write;
    use tempfile::TempDir;
    use zip::write::FileOptions;
    use zip::ZipWriter;

    fn origin() -> SkillOrigin {
        SkillOrigin {
            source_type: SkillSourceType::Github,
            url: "https://github.com/example/skill".to_string(),
            version: None,
            content_digest: None,
        }
    }

    fn archive(entries: &[(&str, &str)]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        for (path, contents) in entries {
            writer.start_file(*path, FileOptions::default()).unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn preview_accepts_valid_skill_archive() {
        let tmp = TempDir::new().unwrap();
        let archive = archive(&[("example/SKILL.md", "# Example\nA useful skill."), ("example/reference.txt", "data")]);
        let (preview, package) = preview_zip_archive(&archive, origin(), tmp.path(), &SkillRepositoryConfig::default()).unwrap();
        assert!(preview.valid);
        assert!(!preview.conflict);
        assert_eq!(package.directory_name, "example");
        assert_eq!(package.name, "Example");
        assert_eq!(package.files.len(), 2);
    }

    #[test]
    fn preview_rejects_missing_or_multiple_skill_roots() {
        let tmp = TempDir::new().unwrap();
        let missing = archive(&[("example/readme.md", "missing")]);
        assert!(preview_zip_archive(&missing, origin(), tmp.path(), &SkillRepositoryConfig::default()).is_err());
        let multiple = archive(&[("one/SKILL.md", "# One"), ("two/SKILL.md", "# Two")]);
        assert!(preview_zip_archive(&multiple, origin(), tmp.path(), &SkillRepositoryConfig::default()).is_err());
    }

    #[test]
    fn preview_rejects_path_traversal() {
        let tmp = TempDir::new().unwrap();
        let archive = archive(&[("example/SKILL.md", "# Example"), ("example/../../escape.txt", "escape")]);
        assert!(preview_zip_archive(&archive, origin(), tmp.path(), &SkillRepositoryConfig::default()).is_err());
    }

    #[test]
    fn import_detects_conflict_and_replaces_atomically() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("skills");
        let first = archive(&[("example/SKILL.md", "# First")]);
        let (_, first_package) = preview_zip_archive(&first, origin(), &root, &SkillRepositoryConfig::default()).unwrap();
        import_skill_package(&root, &first_package, false).unwrap();

        let second = archive(&[("example/SKILL.md", "# Second")]);
        let (_, second_package) = preview_zip_archive(&second, origin(), &root, &SkillRepositoryConfig::default()).unwrap();
        assert!(import_skill_package(&root, &second_package, false).is_err());
        import_skill_package(&root, &second_package, true).unwrap();
        assert_eq!(fs::read_to_string(root.join("example/SKILL.md")).unwrap(), "# Second");
        assert_eq!(scan_local_skills(&root, &SkillRepositoryConfig::default()).unwrap()[0].name, "Second");
    }

    #[test]
    fn delete_requires_matching_confirmation() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("skills");
        let archive = archive(&[("example/SKILL.md", "# Example")]);
        let (_, package) = preview_zip_archive(&archive, origin(), &root, &SkillRepositoryConfig::default()).unwrap();
        import_skill_package(&root, &package, false).unwrap();
        assert!(delete_skill_package(&root, "example", "wrong").is_err());
        delete_skill_package(&root, "example", "example").unwrap();
        assert!(!root.join("example").exists());
    }
}
