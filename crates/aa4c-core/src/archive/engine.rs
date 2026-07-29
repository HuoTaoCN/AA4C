//! 归档规则引擎（ARCHIVE_DESIGN.md §2.3/§2.4，里程碑 AI1）：匹配 → 模板展开 → 移动 →
//! 落库 → 事件；以及撤销。纯逻辑 + fs 操作，不依赖 Tauri（同 `sync_index`/`unified` 的
//!既有归属先例，见 ARCHIVE_DESIGN.md §1）。

use std::path::{Path, PathBuf};

use aa4c_store::Store;
use aa4c_types::{
    ArchiveAction, ArchiveCategory, ArchiveMatch, ArchiveRule, CoreEvent, ModelMeta, Result,
    TagSource,
};

use super::detect::detect_category;
use super::gguf::parse_model_meta;

/// 五条内置预设规则，全部**默认停用**（ARCHIVE_DESIGN.md §2.3：装完就悄悄移动用户
/// 文件是意外行为）。首次启动时（`archive_rules` 表为空）一次性写入，之后不会重复
/// 插入、也不会覆盖用户已经改过的同名规则。
pub async fn ensure_default_rules(store: &Store) -> Result<()> {
    if !store.list_archive_rules().await?.is_empty() {
        return Ok(());
    }
    let presets: [(&str, ArchiveCategory, &str, &[&str]); 5] = [
        (
            "模型",
            ArchiveCategory::Model,
            "模型/{模型.架构}",
            &["模型"],
        ),
        ("图片", ArchiveCategory::Image, "图片/{年}/{月}", &[]),
        ("视频", ArchiveCategory::Video, "视频/{年}/{月}", &[]),
        ("文档", ArchiveCategory::Document, "文档/{年}/{月}", &[]),
        ("压缩包", ArchiveCategory::Archive, "压缩包", &[]),
    ];
    for (i, (name, category, template, tags)) in presets.into_iter().enumerate() {
        let rule = ArchiveRule {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            enabled: false,
            position: i as i64,
            matcher: ArchiveMatch {
                categories: vec![category],
                extensions: None,
                glob: None,
                min_size: None,
                max_size: None,
            },
            action: ArchiveAction {
                target_template: template.to_string(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
            },
            created_at: 0,
            updated_at: 0,
        };
        store.upsert_archive_rule(&rule).await?;
    }
    Ok(())
}

/// 结果：命中并成功归档 / 没有规则命中（原样跳过，不是错误）。`entry_id` 目前只有
/// 测试会读（校验落库结果），生产调用方（下载钩子、`archive_files` Command）都只用
/// `to_path`——保留这个字段是因为它是"这次归档产生的记录是哪一条"的唯一线索，未来
/// 前端想要立即定位新纪录时用得上，删掉后再加回来的成本比放一个 `#[allow(dead_code)]`
/// 高。
pub enum ApplyOutcome {
    Applied {
        #[allow(dead_code)]
        entry_id: String,
        to_path: PathBuf,
    },
    NoRuleMatched,
}

/// 对一个文件跑一遍规则引擎：识别类别 → 找第一条命中的启用规则 → 展开目标目录模板 →
/// 移动 → 写 `archive_entries`/`archive_tags`/`archive_log` → 发 `ArchiveApplied` 事件。
/// 没有规则命中时原样跳过（`NoRuleMatched`），不是错误——调用方（下载完成钩子、批量
/// 归档命令）不需要为"这个文件没配规则"专门处理异常路径。
pub async fn apply_rules(
    store: &Store,
    events: &tokio::sync::broadcast::Sender<CoreEvent>,
    archive_root: &Path,
    source_path: &Path,
) -> Result<ApplyOutcome> {
    let category = detect_category(source_path);
    let size = std::fs::metadata(source_path).map(|m| m.len()).unwrap_or(0);
    let rules = store.list_archive_rules().await?;
    let Some(rule) = rules
        .into_iter()
        .find(|r| r.enabled && rule_matches(r, category, source_path, size))
    else {
        return Ok(ApplyOutcome::NoRuleMatched);
    };

    let (entry_id, to_path) = record_move(store, events, archive_root, source_path, &rule).await?;
    Ok(ApplyOutcome::Applied { entry_id, to_path })
}

/// 用户从归档页手选某条规则、强制应用到指定文件——**不检查该规则的匹配条件**
/// （用户主动选了这条规则，就是想覆盖自动匹配的结果，ARCHIVE_DESIGN §2.4"应用某条
/// 规则"手动路径）。规则不存在（比如列表刷新之间被删了）报错，不静默跳过。
pub async fn apply_selected_rule(
    store: &Store,
    events: &tokio::sync::broadcast::Sender<CoreEvent>,
    archive_root: &Path,
    source_path: &Path,
    rule_id: &str,
) -> Result<(String, PathBuf)> {
    let rule = store
        .list_archive_rules()
        .await?
        .into_iter()
        .find(|r| r.id == rule_id)
        .ok_or_else(|| {
            aa4c_types::Aa4cError::Protocol(format!("archive rule {rule_id} not found"))
        })?;
    record_move(store, events, archive_root, source_path, &rule).await
}

/// 手动归档：跳过规则匹配，直接按调用方给定的目标目录移动（归档页/统一文件视图的
/// "手选目标"手动路径，ARCHIVE_DESIGN §2.4）。`rule_id` 恒为 `None`，不追加标签
/// （没有规则可言，标签留给用户后续自己打）。
pub async fn apply_manual(
    store: &Store,
    events: &tokio::sync::broadcast::Sender<CoreEvent>,
    source_path: &Path,
    target_dir: &Path,
) -> Result<(String, PathBuf)> {
    let category = detect_category(source_path);
    let model_meta = if category == ArchiveCategory::Model {
        parse_model_meta(source_path).ok()
    } else {
        None
    };
    let size = std::fs::metadata(source_path).map(|m| m.len()).unwrap_or(0);

    let to_path = move_into(source_path, target_dir)?;
    finish_move(
        store,
        events,
        source_path,
        &to_path,
        category,
        size,
        model_meta.as_ref(),
        None,
        &[],
    )
    .await
}

/// `apply_rules`/`apply_selected_rule` 共用：给定已经确定要用的规则，展开模板、移动、
/// 落库、发事件。
async fn record_move(
    store: &Store,
    events: &tokio::sync::broadcast::Sender<CoreEvent>,
    archive_root: &Path,
    source_path: &Path,
    rule: &ArchiveRule,
) -> Result<(String, PathBuf)> {
    let category = detect_category(source_path);
    let model_meta = if category == ArchiveCategory::Model {
        parse_model_meta(source_path).ok()
    } else {
        None
    };
    let size = std::fs::metadata(source_path).map(|m| m.len()).unwrap_or(0);

    let target_dir = archive_root.join(expand_template(
        &rule.action.target_template,
        category,
        model_meta.as_ref(),
    ));
    let to_path = move_into(source_path, &target_dir)?;
    finish_move(
        store,
        events,
        source_path,
        &to_path,
        category,
        size,
        model_meta.as_ref(),
        Some(&rule.id),
        &rule.action.tags,
    )
    .await
}

/// 落库 + 发事件的公共尾段：写 `archive_entries`、按需追加标签、写 `archive_log`、
/// 发 `ArchiveApplied`。文件本身在调用这个函数之前就已经移动完成。
#[allow(clippy::too_many_arguments)]
async fn finish_move(
    store: &Store,
    events: &tokio::sync::broadcast::Sender<CoreEvent>,
    source_path: &Path,
    to_path: &Path,
    category: ArchiveCategory,
    size: u64,
    model_meta: Option<&ModelMeta>,
    rule_id: Option<&str>,
    tags: &[String],
) -> Result<(String, PathBuf)> {
    let entry_id = uuid::Uuid::new_v4().to_string();
    store
        .insert_archive_entry(
            &entry_id,
            &to_path.to_string_lossy(),
            category,
            size,
            model_meta,
        )
        .await?;
    if !tags.is_empty() {
        let tags: Vec<(String, TagSource)> =
            tags.iter().map(|t| (t.clone(), TagSource::Rule)).collect();
        store.add_archive_tags(&entry_id, &tags).await?;
    }
    store
        .append_archive_log(
            &entry_id,
            &source_path.to_string_lossy(),
            &to_path.to_string_lossy(),
            rule_id,
        )
        .await?;
    let _ = events.send(CoreEvent::ArchiveApplied {
        entry_id: entry_id.clone(),
        from_path: source_path.to_string_lossy().into_owned(),
        to_path: to_path.to_string_lossy().into_owned(),
        rule_id: rule_id.map(str::to_owned),
    });
    Ok((entry_id, to_path.to_path_buf()))
}

/// 撤销一条移动历史：把文件挪回 `from_path`，回写 `archive_entries.current_path`，
/// 摘掉规则当初追加的标签（按 `rule.action.tags` 反查——规则若已被删除就跳过标签清理，
/// 只保证路径这个安全关键的部分一定完成，见 ARCHIVE_DESIGN §2.4）。
pub async fn undo(store: &Store, log_id: i64) -> Result<()> {
    let log = store.get_archive_log_entry(log_id).await?.ok_or_else(|| {
        aa4c_types::Aa4cError::Protocol(format!("archive log {log_id} not found"))
    })?;
    if log.undone {
        return Ok(());
    }

    let from = PathBuf::from(&log.to_path); // 当前所在位置
    let to = PathBuf::from(&log.from_path); // 撤销后应该回到的位置
    move_back(&from, &to)?;

    store
        .update_archive_entry_path(&log.entry_id, &log.from_path)
        .await?;

    if let Some(rule_id) = &log.rule_id {
        if let Some(rule) = store
            .list_archive_rules()
            .await?
            .into_iter()
            .find(|r| &r.id == rule_id)
        {
            for tag in &rule.action.tags {
                store.remove_archive_tag(&log.entry_id, tag).await?;
            }
        }
    }

    store.mark_archive_log_undone(log_id).await?;
    Ok(())
}

fn rule_matches(rule: &ArchiveRule, category: ArchiveCategory, path: &Path, size: u64) -> bool {
    let m = &rule.matcher;
    if !m.categories.is_empty() && !m.categories.contains(&category) {
        return false;
    }
    if let Some(exts) = &m.extensions {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !ext.is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(&e))) {
            return false;
        }
    }
    if let Some(glob) = &m.glob {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !glob_match(glob, name) {
            return false;
        }
    }
    if let Some(min) = m.min_size {
        if size < min {
            return false;
        }
    }
    if let Some(max) = m.max_size {
        if size > max {
            return false;
        }
    }
    true
}

/// 极简 glob：只支持 `*`（任意长度任意字符），不支持 `?`/字符集——规则里的文件名
/// 模式只是"大概筛一下"，不需要完整 glob 语义，够用即可（不引入新依赖）。
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !rest.starts_with(part) {
                return false;
            }
            rest = &rest[part.len()..];
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(idx) = rest.find(part) {
            rest = &rest[idx + part.len()..];
        } else {
            return false;
        }
    }
    true
}

/// 占位符缺值时用「未知」，绝不失败中断（ARCHIVE_DESIGN §2.3）。
fn expand_template(template: &str, category: ArchiveCategory, model: Option<&ModelMeta>) -> String {
    let (year, month) = current_year_month();
    let unknown = "未知";
    template
        .replace("{类别}", category_label(category))
        .replace("{年}", &year.to_string())
        .replace("{月}", &format!("{month:02}"))
        .replace(
            "{模型.架构}",
            model
                .and_then(|m| m.architecture.as_deref())
                .unwrap_or(unknown),
        )
        .replace(
            "{模型.名称}",
            model.and_then(|m| m.name.as_deref()).unwrap_or(unknown),
        )
        .replace(
            "{模型.量化}",
            model
                .and_then(|m| m.file_type.as_deref())
                .unwrap_or(unknown),
        )
}

fn category_label(c: ArchiveCategory) -> &'static str {
    match c {
        ArchiveCategory::Model => "模型",
        ArchiveCategory::Image => "图片",
        ArchiveCategory::Video => "视频",
        ArchiveCategory::Audio => "音频",
        ArchiveCategory::Document => "文档",
        ArchiveCategory::Ebook => "电子书",
        ArchiveCategory::Archive => "压缩包",
        ArchiveCategory::Installer => "安装包",
        ArchiveCategory::Code => "代码",
        ArchiveCategory::Subtitle => "字幕",
        ArchiveCategory::Other => "其他",
    }
}

/// 当前 UTC 年/月（不引入 `chrono`：Howard Hinnant 的公开算法把 Unix 天数转公历
/// 年月日，只用得到年月两个数，没必要为此加一整个日期时间库依赖）。
fn current_year_month() -> (i64, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    civil_from_days(days)
}

/// <https://howardhinnant.github.io/date_algorithms.html#civil_from_days>（公开算法，
/// CC0/无版权限制），只取年月。
fn civil_from_days(z: i64) -> (i64, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32)
}

/// 把 `source` 移动进 `target_dir`（不存在则创建），同名冲突加序号（`unified.rs` 的
/// 既有算法：第 1 份原名，其余在扩展名前插 ` (n)`）。跨卷（`EXDEV`）自动回退到
/// 拷贝+fsync+删除，`rename` 本身就是原子的，回退路径才需要手动保证落盘顺序。
fn move_into(source: &Path, target_dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(target_dir)?;
    let file_name = source
        .file_name()
        .ok_or_else(|| aa4c_types::Aa4cError::Protocol("source has no file name".into()))?;
    let to = unique_target(target_dir, file_name.to_string_lossy().as_ref());
    move_file(source, &to)?;
    Ok(to)
}

/// 撤销专用：`to` 是撤销后应落回的原路径，若该位置已被别的文件占用则报错，不强行覆盖
/// （ARCHIVE_DESIGN §2.4："原位置已被占用则报错不强行覆盖"）。
fn move_back(from: &Path, to: &Path) -> Result<()> {
    if to.exists() {
        return Err(aa4c_types::Aa4cError::Protocol(format!(
            "undo target already occupied: {}",
            to.display()
        )));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    move_file(from, to)
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc_exdev()) => {
            std::fs::copy(from, to)?;
            let f = std::fs::File::open(to)?;
            f.sync_all()?;
            std::fs::remove_file(from)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(unix)]
fn libc_exdev() -> i32 {
    18 // EXDEV，跨平台一致（Linux/macOS 均为 18）
}

#[cfg(windows)]
fn libc_exdev() -> i32 {
    17 // ERROR_NOT_SAME_DEVICE
}

fn unique_target(dir: &Path, file_name: &str) -> PathBuf {
    for seq in 1.. {
        let candidate = dir.join(numbered_name(file_name, seq));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("seq is unbounded")
}

/// 同 `unified.rs::numbered` 的算法，只是作用对象是文件名字符串而不是限定展示路径。
fn numbered_name(name: &str, seq: usize) -> String {
    if seq <= 1 {
        return name.to_string();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({}){}", &name[..dot], seq, &name[dot..]),
        _ => format!("{name} ({seq})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rule(id: &str, categories: Vec<ArchiveCategory>, template: &str) -> ArchiveRule {
        ArchiveRule {
            id: id.into(),
            name: id.into(),
            enabled: true,
            position: 0,
            matcher: ArchiveMatch {
                categories,
                extensions: None,
                glob: None,
                min_size: None,
                max_size: None,
            },
            action: ArchiveAction {
                target_template: template.into(),
                tags: vec![],
            },
            created_at: 0,
            updated_at: 0,
        }
    }

    /// 手写的公历转换算法本身没有经过官方库背书，用几个已知真实日期的 epoch 天数
    /// （`python3 -c "print((date(Y,M,D)-date(1970,1,1)).days)"` 算出来的参考值）
    /// 交叉核实，而不是假设算法抄对了就一定没错（执行纪律 #1：外部事实/算法必须实证）。
    #[test]
    fn civil_from_days_matches_known_reference_dates() {
        assert_eq!(civil_from_days(19723), (2024, 1)); // 2024-01-01
        assert_eq!(civil_from_days(20082), (2024, 12)); // 2024-12-25
        assert_eq!(civil_from_days(11017), (2000, 3)); // 2000-03-01（世纪闰年边界）
        assert_eq!(civil_from_days(0), (1970, 1)); // 1970-01-01（纪元本身）
        assert_eq!(civil_from_days(20655), (2026, 7)); // 2026-07-21
    }

    #[test]
    fn template_expands_and_falls_back_to_unknown() {
        let meta = ModelMeta {
            architecture: Some("qwen3".into()),
            ..Default::default()
        };
        let expanded = expand_template(
            "{类别}/{模型.架构}/{模型.量化}",
            ArchiveCategory::Model,
            Some(&meta),
        );
        assert_eq!(expanded, "模型/qwen3/未知");
    }

    #[test]
    fn numbered_name_inserts_before_extension() {
        assert_eq!(numbered_name("a.gguf", 1), "a.gguf");
        assert_eq!(numbered_name("a.gguf", 2), "a (2).gguf");
        assert_eq!(numbered_name("no_ext", 3), "no_ext (3)");
    }

    #[test]
    fn glob_matches_simple_patterns() {
        assert!(glob_match("*.gguf", "model.gguf"));
        assert!(!glob_match("*.gguf", "model.bin"));
        assert!(glob_match("Qwen*", "Qwen3-4B.gguf"));
        assert!(glob_match("exact.txt", "exact.txt"));
        assert!(!glob_match("exact.txt", "other.txt"));
    }

    #[test]
    fn rule_matches_respects_size_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.gguf");
        std::fs::write(&path, vec![0u8; 100]).unwrap();
        let mut rule = sample_rule("r1", vec![ArchiveCategory::Model], "{类别}");
        rule.matcher.min_size = Some(200);
        assert!(!rule_matches(&rule, ArchiveCategory::Model, &path, 100));
        rule.matcher.min_size = Some(50);
        assert!(rule_matches(&rule, ArchiveCategory::Model, &path, 100));
    }

    #[tokio::test]
    async fn apply_rules_moves_file_and_records_entry_when_matched() {
        let dir = tempfile::tempdir().unwrap();
        let store = aa4c_store::Store::open(&dir.path().join("aa4c.db"))
            .await
            .unwrap();
        let (tx, _rx) = tokio::sync::broadcast::channel(16);

        let src_dir = dir.path().join("downloads");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("model.gguf");
        std::fs::write(
            &src,
            b"GGUF\x03\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00",
        )
        .unwrap();

        store
            .upsert_archive_rule(&sample_rule("r1", vec![ArchiveCategory::Model], "模型"))
            .await
            .unwrap();

        let archive_root = dir.path().join("archive");
        let outcome = apply_rules(&store, &tx, &archive_root, &src).await.unwrap();
        let (entry_id, to_path) = match outcome {
            ApplyOutcome::Applied { entry_id, to_path } => (entry_id, to_path),
            ApplyOutcome::NoRuleMatched => panic!("expected a match"),
        };
        assert!(!src.exists(), "source should have been moved away");
        assert!(to_path.exists());
        assert_eq!(to_path, archive_root.join("模型/model.gguf"));

        let entry = store.get_archive_entry(&entry_id).await.unwrap().unwrap();
        assert_eq!(entry.category, ArchiveCategory::Model);
        assert_eq!(store.list_archive_log().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn apply_rules_no_rule_matched_leaves_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let store = aa4c_store::Store::open(&dir.path().join("aa4c.db"))
            .await
            .unwrap();
        let (tx, _rx) = tokio::sync::broadcast::channel(16);

        let src = dir.path().join("readme.txt");
        std::fs::write(&src, b"hello").unwrap();

        let outcome = apply_rules(&store, &tx, &dir.path().join("archive"), &src)
            .await
            .unwrap();
        assert!(matches!(outcome, ApplyOutcome::NoRuleMatched));
        assert!(src.exists(), "no rule matched, file must stay put");
    }

    #[tokio::test]
    async fn undo_moves_file_back_and_removes_rule_tags() {
        let dir = tempfile::tempdir().unwrap();
        let store = aa4c_store::Store::open(&dir.path().join("aa4c.db"))
            .await
            .unwrap();
        let (tx, _rx) = tokio::sync::broadcast::channel(16);

        let src_dir = dir.path().join("downloads");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("photo.png");
        std::fs::write(&src, [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']).unwrap();

        let mut rule = sample_rule("r1", vec![ArchiveCategory::Image], "图片");
        rule.action.tags = vec!["图片".into()];
        store.upsert_archive_rule(&rule).await.unwrap();

        let outcome = apply_rules(&store, &tx, &dir.path().join("archive"), &src)
            .await
            .unwrap();
        let entry_id = match outcome {
            ApplyOutcome::Applied { entry_id, .. } => entry_id,
            ApplyOutcome::NoRuleMatched => panic!("expected a match"),
        };
        assert_eq!(store.list_archive_tags(&entry_id).await.unwrap().len(), 1);

        let log_id = store.list_archive_log().await.unwrap()[0].id;
        undo(&store, log_id).await.unwrap();

        assert!(src.exists(), "file should be back at its original path");
        assert_eq!(store.list_archive_tags(&entry_id).await.unwrap().len(), 0);
        let log = store.get_archive_log_entry(log_id).await.unwrap().unwrap();
        assert!(log.undone);

        // 重复撤销是幂等的（已经 undone 的直接返回，不会再挪一次文件）
        undo(&store, log_id).await.unwrap();
    }

    #[tokio::test]
    async fn ensure_default_rules_inserts_five_disabled_presets_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = aa4c_store::Store::open(&dir.path().join("aa4c.db"))
            .await
            .unwrap();

        ensure_default_rules(&store).await.unwrap();
        let rules = store.list_archive_rules().await.unwrap();
        assert_eq!(rules.len(), 5);
        assert!(rules.iter().all(|r| !r.enabled));

        // 用户改了一条（启用它）；再次调用不应该覆盖或重复插入
        let mut edited = rules[0].clone();
        edited.enabled = true;
        store.upsert_archive_rule(&edited).await.unwrap();
        ensure_default_rules(&store).await.unwrap();
        let after = store.list_archive_rules().await.unwrap();
        assert_eq!(after.len(), 5);
        assert!(after.iter().any(|r| r.enabled));
    }
}
