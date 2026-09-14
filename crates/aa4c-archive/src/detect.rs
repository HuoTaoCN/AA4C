//! 文件类型识别（ARCHIVE_DESIGN.md §2.1）：扩展名表为主，~20 条 magic bytes 兜底
//! （不引入 `infer` 等新依赖，同项目一贯克制）。扩展名与 magic 冲突时信 magic——
//! 典型场景是模型文件用了不常见/被改过的扩展名，GGUF 头本身足够确定它是模型。

use std::io::Read;
use std::path::Path;

use aa4c_types::ArchiveCategory;

/// 只读文件开头一小段做 magic 探测所需的字节数（覆盖本文件用到的全部 magic 长度，
/// safetensors 的"header_len(8B)+`{`"检查最靠后，占 9 字节）。
const SNIFF_LEN: usize = 32;

/// 综合扩展名与文件头识别类别。I/O 失败（文件不存在/无权限）时退化为仅按扩展名判断，
/// 不向上传播错误——识别失败不应该阻断整条归档流程，最坏情况就是分到「其他」。
pub fn detect_category(path: &Path) -> ArchiveCategory {
    let sniffed = read_head(path);
    if let Some(head) = sniffed.as_deref() {
        if let Some(cat) = sniff_magic(head) {
            return cat;
        }
    }
    by_extension(path).unwrap_or(ArchiveCategory::Other)
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; SNIFF_LEN];
    let n = file.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

/// 只对**明确无歧义**的 magic 下判断——像 ZIP（`PK\x03\x04`）本身可能是 docx/xlsx/
/// apk/jar/普通压缩包等任意一种，光看头 4 字节判断不出具体是哪种，这类交给扩展名表，
/// 不在这里误判。
fn sniff_magic(head: &[u8]) -> Option<ArchiveCategory> {
    if head.len() >= 4 && &head[0..4] == b"GGUF" {
        return Some(ArchiveCategory::Model);
    }
    if is_safetensors(head) {
        return Some(ArchiveCategory::Model);
    }
    // 图片
    if head.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']) {
        return Some(ArchiveCategory::Image);
    }
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(ArchiveCategory::Image);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(ArchiveCategory::Image);
    }
    if head.starts_with(b"BM") {
        return Some(ArchiveCategory::Image);
    }
    if head.len() >= 12 && &head[0..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some(ArchiveCategory::Image);
    }
    // 视频
    if head.len() >= 8 && &head[4..8] == b"ftyp" {
        return Some(ArchiveCategory::Video);
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        // EBML 容器：Matroska/WebM 共用，都算视频（音频版 mka 少见，不单独分支）
        return Some(ArchiveCategory::Video);
    }
    // 音频
    if head.starts_with(b"ID3") || head.starts_with(&[0xFF, 0xFB]) {
        return Some(ArchiveCategory::Audio);
    }
    if head.starts_with(b"fLaC") {
        return Some(ArchiveCategory::Audio);
    }
    if head.starts_with(b"OggS") {
        return Some(ArchiveCategory::Audio);
    }
    // 文档
    if head.starts_with(b"%PDF-") {
        return Some(ArchiveCategory::Document);
    }
    // 压缩包
    if head.starts_with(&[0x1F, 0x8B]) {
        return Some(ArchiveCategory::Archive);
    }
    if head.starts_with(b"BZh") {
        return Some(ArchiveCategory::Archive);
    }
    if head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        return Some(ArchiveCategory::Archive);
    }
    if head.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
        return Some(ArchiveCategory::Archive);
    }
    if head.starts_with(b"Rar!\x1a\x07") {
        return Some(ArchiveCategory::Archive);
    }
    None
}

/// safetensors 格式：前 8 字节是小端 u64 JSON 头长度，紧接着是以 `{` 开头的 JSON。
/// 只做形状校验（长度落在合理区间 + 紧跟 `{`），不解析 JSON 内容——这里只是分类，
/// 不是像 GGUF 那样要展示元数据。
fn is_safetensors(head: &[u8]) -> bool {
    if head.len() < 9 {
        return false;
    }
    let header_len = u64::from_le_bytes(head[0..8].try_into().expect("checked len"));
    header_len > 0 && header_len < 100_000_000 && head[8] == b'{'
}

fn by_extension(path: &Path) -> Option<ArchiveCategory> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    use ArchiveCategory::*;
    Some(match ext.as_str() {
        "gguf" | "safetensors" | "ckpt" | "pt" | "pth" | "onnx" => Model,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "svg" | "tiff" | "ico" => Image,
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" => Video,
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" => Audio,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "rtf" | "odt"
        | "csv" => Document,
        "epub" | "mobi" | "azw3" | "azw" | "fb2" => Ebook,
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => Archive,
        "exe" | "dmg" | "pkg" | "deb" | "rpm" | "msi" | "appimage" => Installer,
        "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" | "json" | "yaml" | "yml"
        | "toml" | "sh" | "vue" | "html" | "css" => Code,
        "srt" | "ass" | "vtt" | "sub" => Subtitle,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(bytes: &[u8], ext: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("sample.{ext}"));
        std::fs::File::create(&path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
        dir
    }

    #[test]
    fn detects_by_extension_when_no_magic_matches() {
        let dir = write_temp(b"just some plain bytes", "mp3");
        // 内容不是真实 mp3 帧头，纯靠扩展名兜底
        assert_eq!(
            detect_category(&dir.path().join("sample.mp3")),
            ArchiveCategory::Audio
        );
    }

    #[test]
    fn gguf_magic_wins_over_wrong_extension() {
        // 扩展名故意写成 .bin（未登记在扩展名表里），GGUF 头必须让它仍被识别成模型
        let mut bytes = b"GGUF".to_vec();
        bytes.extend_from_slice(&[3, 0, 0, 0]); // version=3
        let dir = write_temp(&bytes, "bin");
        assert_eq!(
            detect_category(&dir.path().join("sample.bin")),
            ArchiveCategory::Model
        );
    }

    #[test]
    fn png_magic_detected() {
        let dir = write_temp(
            &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n', 0, 0],
            "jpg",
        );
        // 内容其实是 PNG，扩展名写错成 jpg——磁数应该赢
        assert_eq!(
            detect_category(&dir.path().join("sample.jpg")),
            ArchiveCategory::Image
        );
    }

    #[test]
    fn safetensors_magic_detected() {
        let json = br#"{"a":1}"#;
        let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(json);
        let dir = write_temp(&bytes, "dat");
        assert_eq!(
            detect_category(&dir.path().join("sample.dat")),
            ArchiveCategory::Model
        );
    }

    #[test]
    fn unknown_extension_and_no_magic_falls_back_to_other() {
        let dir = write_temp(b"whatever", "xyz123");
        assert_eq!(
            detect_category(&dir.path().join("sample.xyz123")),
            ArchiveCategory::Other
        );
    }

    #[test]
    fn missing_file_falls_back_to_extension_only() {
        let path = Path::new("/nonexistent/path/to/model.gguf");
        // 读不到文件内容，仍能靠扩展名兜底（不 panic、不报错向上传播）
        assert_eq!(detect_category(path), ArchiveCategory::Model);
    }

    #[test]
    fn zip_based_container_defers_to_extension_not_generic_archive() {
        // PK 头本身有歧义（docx/xlsx/apk/zip 都用它），交给扩展名表判断具体类型
        let dir = write_temp(&[b'P', b'K', 0x03, 0x04, 0, 0, 0, 0], "docx");
        assert_eq!(
            detect_category(&dir.path().join("sample.docx")),
            ArchiveCategory::Document
        );
    }
}
