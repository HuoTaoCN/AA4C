//! 路径安全与文件清单（PROTOCOL.md §7 规则 5、V0.1 计划 M5 任务 5）。

use std::path::{Path, PathBuf};

use aa4c_proto::FileMeta;
use aa4c_types::{Aa4cError, Result};

/// 净化接收到的 rel_path（'/' 分隔）。
///
/// 拒绝：空路径、绝对路径、`..`、`.`、盘符/反斜杠、NUL、空组件。
/// 返回相对 PathBuf（仅普通组件）。
pub(crate) fn sanitize_rel_path(rel_path: &str) -> Result<PathBuf> {
    if rel_path.is_empty()
        || rel_path.starts_with('/')
        || rel_path.contains('\\')
        || rel_path.contains('\0')
        || rel_path.contains(':')
    {
        return Err(bad_path(rel_path));
    }
    let mut out = PathBuf::new();
    for component in rel_path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(bad_path(rel_path));
        }
        out.push(component);
    }
    Ok(out)
}

fn bad_path(p: &str) -> Aa4cError {
    Aa4cError::Protocol(format!("illegal rel_path: {p:?}"))
}

/// 目标已存在时追加 ` (1)` / ` (2)` … 后缀，返回可用路径。
pub(crate) fn dedup_target(target: &Path) -> PathBuf {
    if !target.exists() {
        return target.to_path_buf();
    }
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = target.extension().map(|e| e.to_string_lossy().into_owned());
    let parent = target.parent().unwrap_or_else(|| Path::new(""));
    for n in 1.. {
        let name = match &ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("dedup counter exhausted")
}

/// 发送清单条目：本地绝对路径 + 线路元数据。
#[derive(Debug, Clone)]
pub(crate) struct SendFile {
    pub abs: PathBuf,
    pub meta: FileMeta,
}

/// 枚举待发送路径：文件取文件名，目录递归（rel_path 保留目录名前缀，'/' 分隔）。
///
/// 空目录与符号链接跳过；无任何文件 → 错误。
pub(crate) async fn build_manifest(paths: &[PathBuf]) -> Result<Vec<SendFile>> {
    let mut out = Vec::new();
    for path in paths {
        let meta = tokio::fs::symlink_metadata(path).await?;
        if meta.is_symlink() {
            continue;
        }
        let name = path
            .file_name()
            .ok_or_else(|| Aa4cError::Protocol(format!("path has no file name: {path:?}")))?
            .to_string_lossy()
            .into_owned();
        if meta.is_file() {
            out.push(SendFile {
                abs: path.clone(),
                meta: FileMeta {
                    rel_path: name,
                    size: meta.len(),
                },
            });
        } else if meta.is_dir() {
            walk_dir(path, &name, &mut out).await?;
        }
    }
    if out.is_empty() {
        return Err(Aa4cError::Protocol("nothing to send".into()));
    }
    Ok(out)
}

/// 递归枚举目录（迭代式，避免 async 递归）。
async fn walk_dir(root: &Path, prefix: &str, out: &mut Vec<SendFile>) -> Result<()> {
    let mut stack = vec![(root.to_path_buf(), prefix.to_string())];
    while let Some((dir, rel_prefix)) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let meta = entry.metadata().await?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = format!("{rel_prefix}/{name}");
            if meta.is_symlink() {
                continue;
            }
            if meta.is_file() {
                out.push(SendFile {
                    abs: entry.path(),
                    meta: FileMeta {
                        rel_path: rel,
                        size: meta.len(),
                    },
                });
            } else if meta.is_dir() {
                stack.push((entry.path(), rel));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_accepts_normal_nested_path() {
        let p = sanitize_rel_path("照片/2026/IMG (1).jpg").unwrap();
        assert_eq!(p, PathBuf::from("照片").join("2026").join("IMG (1).jpg"));
    }

    #[test]
    fn sanitize_rejects_traversal_and_absolute() {
        for bad in [
            "",
            "/etc/passwd",
            "../secret",
            "a/../b",
            "a/./b",
            "a//b",
            "C:evil",
            "a\\b",
            "nul\0byte",
        ] {
            assert!(sanitize_rel_path(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn dedup_appends_counter() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        assert_eq!(dedup_target(&target), target);
        std::fs::write(&target, b"x").unwrap();
        assert_eq!(dedup_target(&target), dir.path().join("a (1).txt"));
        std::fs::write(dir.path().join("a (1).txt"), b"x").unwrap();
        assert_eq!(dedup_target(&target), dir.path().join("a (2).txt"));
        // 无扩展名
        let plain = dir.path().join("README");
        std::fs::write(&plain, b"x").unwrap();
        assert_eq!(dedup_target(&plain), dir.path().join("README (1)"));
    }

    #[tokio::test]
    async fn manifest_walks_directories_with_forward_slashes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("项目");
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join("a.txt"), b"1").unwrap();
        std::fs::write(root.join("src/deep/b.rs"), b"22").unwrap();
        std::fs::create_dir(root.join("empty")).unwrap();
        let single = dir.path().join("single.bin");
        std::fs::write(&single, b"333").unwrap();

        let mut manifest = build_manifest(&[root, single]).await.unwrap();
        manifest.sort_by(|a, b| a.meta.rel_path.cmp(&b.meta.rel_path));
        let rels: Vec<_> = manifest.iter().map(|f| f.meta.rel_path.as_str()).collect();
        assert_eq!(rels, vec!["single.bin", "项目/a.txt", "项目/src/deep/b.rs"]);
        assert_eq!(manifest[0].meta.size, 3);
    }

    #[tokio::test]
    async fn manifest_rejects_empty_input() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(build_manifest(&[empty]).await.is_err());
    }
}
