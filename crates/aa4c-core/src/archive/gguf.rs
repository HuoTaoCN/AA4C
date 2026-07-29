//! GGUF 头解析（ARCHIVE_DESIGN.md §2.2）：纯 Rust 手写，只读元数据头，**永远不读张量
//! 数据**。硬界限防御模型文件可能来路不明（kv 数/字符串长度/数组长度都设了上限），
//! 解析失败一律走 `Err`，调用方（AI1.4 规则引擎）应当把它当"识别失败"而不是致命错误
//! （同 `detect.rs` 的既有取舍）。
//!
//! `general.file_type` 的量化枚举映射（`FTYPE_NAMES`）已对照 llama.cpp 官方源码逐项
//! 核实（`include/llama.h` 的 `enum llama_ftype` + `src/llama-model-loader.cpp` 的
//! `llama_ftype_name()`，抓取自 `master` 分支，规划阶段一次性核对，不是凭记忆猜的）——
//! 未收录的新枚举值（llama.cpp 仍在持续新增）走 `_ => 原始数字` 兜底，不会解析失败。

use std::collections::HashMap;
use std::io::{BufReader, Read};
use std::path::Path;

use aa4c_types::ModelMeta;

const MAGIC: &[u8; 4] = b"GGUF";
const MAX_KV_COUNT: u64 = 4096;
const MAX_STRING_LEN: u64 = 64 * 1024;
const MAX_ARRAY_LEN: u64 = 65536;

#[derive(Debug)]
pub enum GgufError {
    Io(std::io::Error),
    /// 文件头不是合法 GGUF、版本不支持、或撞到硬界限——统一归为"文件头异常"，
    /// 不区分细分原因（同 `aa4c-server` Lookup 的防探测惯例：没必要，也没意义）。
    Malformed(&'static str),
}

impl std::fmt::Display for GgufError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Malformed(msg) => write!(f, "malformed gguf: {msg}"),
        }
    }
}

impl From<std::io::Error> for GgufError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// 完整表示 GGUF 规范的全部 13 种值类型是"能正确跳过任意不关心的 KV"的前提——
// 只有 String/U32/U64（及派生的 as_str/as_u64/as_u32）会被实际读取，其余变体的
// payload 天然不会被访问，不是遗漏。
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum Value {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    F32(f32),
    Bool(bool),
    String(String),
    U64(u64),
    I64(i64),
    F64(f64),
    Array(Vec<Value>),
}

impl Value {
    fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U8(v) => Some(*v as u64),
            Self::U16(v) => Some(*v as u64),
            Self::U32(v) => Some(*v as u64),
            Self::U64(v) => Some(*v),
            Self::I32(v) if *v >= 0 => Some(*v as u64),
            Self::I64(v) if *v >= 0 => Some(*v as u64),
            _ => None,
        }
    }

    fn as_u32(&self) -> Option<u32> {
        self.as_u64().and_then(|v| u32::try_from(v).ok())
    }
}

/// 解析文件开头的 GGUF 元数据（不读张量数据）。
pub fn parse_model_meta(path: &Path) -> Result<ModelMeta, GgufError> {
    let file = std::fs::File::open(path)?;
    let mut r = BufReader::new(file);

    let mut magic = [0u8; 4];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(GgufError::Malformed("bad magic"));
    }

    let version = read_u32(&mut r)?;
    if version != 2 && version != 3 {
        return Err(GgufError::Malformed("unsupported version (only 2/3)"));
    }

    let _tensor_count = read_u64(&mut r)?;
    let kv_count = read_u64(&mut r)?;
    if kv_count > MAX_KV_COUNT {
        return Err(GgufError::Malformed("kv_count exceeds limit"));
    }

    let mut kv: HashMap<String, Value> = HashMap::with_capacity(kv_count as usize);
    for _ in 0..kv_count {
        let key = read_string(&mut r)?;
        let value_type = read_u32(&mut r)?;
        let value = read_value(&mut r, value_type)?;
        kv.insert(key, value);
    }

    let architecture = kv
        .get("general.architecture")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let name = kv
        .get("general.name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let size_label = kv
        .get("general.size_label")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let file_type = kv
        .get("general.file_type")
        .and_then(Value::as_u32)
        .map(|code| ftype_label(code).to_string());
    let context_length = architecture
        .as_deref()
        .and_then(|arch| kv.get(&format!("{arch}.context_length")))
        .and_then(Value::as_u64);

    Ok(ModelMeta {
        architecture,
        name,
        size_label,
        file_type,
        context_length,
    })
}

fn read_value<R: Read>(r: &mut R, value_type: u32) -> Result<Value, GgufError> {
    Ok(match value_type {
        0 => Value::U8(read_u8(r)?),
        1 => Value::I8(read_u8(r)? as i8),
        2 => Value::U16(read_u16(r)?),
        3 => Value::I16(read_u16(r)? as i16),
        4 => Value::U32(read_u32(r)?),
        5 => Value::I32(read_u32(r)? as i32),
        6 => Value::F32(f32::from_le_bytes(read_u32(r)?.to_le_bytes())),
        7 => Value::Bool(read_u8(r)? != 0),
        8 => Value::String(read_string(r)?),
        9 => {
            let elem_type = read_u32(r)?;
            if elem_type == 9 {
                return Err(GgufError::Malformed("nested arrays not supported"));
            }
            let count = read_u64(r)?;
            if count > MAX_ARRAY_LEN {
                return Err(GgufError::Malformed("array length exceeds limit"));
            }
            let mut items = Vec::with_capacity(count as usize);
            for _ in 0..count {
                items.push(read_value(r, elem_type)?);
            }
            Value::Array(items)
        }
        10 => Value::U64(read_u64(r)?),
        11 => Value::I64(read_u64(r)? as i64),
        12 => Value::F64(f64::from_le_bytes(read_u64(r)?.to_le_bytes())),
        _ => return Err(GgufError::Malformed("unknown value type")),
    })
}

fn read_u8<R: Read>(r: &mut R) -> Result<u8, GgufError> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u16<R: Read>(r: &mut R) -> Result<u16, GgufError> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32, GgufError> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64, GgufError> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_string<R: Read>(r: &mut R) -> Result<String, GgufError> {
    let len = read_u64(r)?;
    if len > MAX_STRING_LEN {
        return Err(GgufError::Malformed("string length exceeds limit"));
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|_| GgufError::Malformed("string is not valid utf-8"))
}

/// `general.file_type` 枚举 → 简短量化标签（`enum llama_ftype`，实测核对，见模块文档）。
/// 未收录的值（llama.cpp 新增枚举比这张表快）落到 `_` 分支，原样展示数字，不报错。
fn ftype_label(code: u32) -> String {
    let label = match code {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XS",
        23 => "IQ3_XXS",
        24 => "IQ1_S",
        25 => "IQ4_NL",
        26 => "IQ3_S",
        27 => "IQ3_M",
        28 => "IQ2_S",
        29 => "IQ2_M",
        30 => "IQ4_XS",
        31 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        38 => "MXFP4_MOE",
        39 => "NVFP4",
        _ => return format!("未知量化（代码 {code}）"),
    };
    label.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 手工拼一个最小合法 GGUF 头字节流：version + tensor_count=0 + 指定的 KV 列表。
    fn build_gguf(version: u32, kvs: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        out.extend_from_slice(&(kvs.len() as u64).to_le_bytes()); // metadata_kv_count
        for (key, value_type, value_bytes) in kvs {
            out.extend_from_slice(&(key.len() as u64).to_le_bytes());
            out.extend_from_slice(key.as_bytes());
            out.extend_from_slice(&value_type.to_le_bytes());
            out.extend_from_slice(value_bytes);
        }
        out
    }

    fn string_value(s: &str) -> Vec<u8> {
        let mut out = (s.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(s.as_bytes());
        out
    }

    fn write_temp(bytes: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::File::create(dir.path().join("m.gguf"))
            .unwrap()
            .write_all(bytes)
            .unwrap();
        dir
    }

    #[test]
    fn parses_common_keys_and_maps_file_type() {
        let bytes = build_gguf(
            3,
            &[
                ("general.architecture", 8, string_value("qwen3")),
                ("general.name", 8, string_value("Qwen3-4B")),
                ("general.size_label", 8, string_value("4B")),
                ("general.file_type", 4, 15u32.to_le_bytes().to_vec()), // Q4_K_M
                ("qwen3.context_length", 4, 8192u32.to_le_bytes().to_vec()),
            ],
        );
        let dir = write_temp(&bytes);
        let meta = parse_model_meta(&dir.path().join("m.gguf")).unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("qwen3"));
        assert_eq!(meta.name.as_deref(), Some("Qwen3-4B"));
        assert_eq!(meta.size_label.as_deref(), Some("4B"));
        assert_eq!(meta.file_type.as_deref(), Some("Q4_K_M"));
        assert_eq!(meta.context_length, Some(8192));
    }

    #[test]
    fn unknown_file_type_falls_back_to_raw_code_not_error() {
        let bytes = build_gguf(
            3,
            &[("general.file_type", 4, 9999u32.to_le_bytes().to_vec())],
        );
        let dir = write_temp(&bytes);
        let meta = parse_model_meta(&dir.path().join("m.gguf")).unwrap();
        assert!(meta.file_type.unwrap().contains("9999"));
    }

    #[test]
    fn missing_keys_are_none_not_error() {
        let bytes = build_gguf(3, &[]);
        let dir = write_temp(&bytes);
        let meta = parse_model_meta(&dir.path().join("m.gguf")).unwrap();
        assert_eq!(meta, ModelMeta::default());
    }

    #[test]
    fn rejects_bad_magic() {
        let dir = write_temp(b"NOPE1234567890");
        assert!(matches!(
            parse_model_meta(&dir.path().join("m.gguf")),
            Err(GgufError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_unsupported_version() {
        let bytes = build_gguf(1, &[]);
        let dir = write_temp(&bytes);
        assert!(matches!(
            parse_model_meta(&dir.path().join("m.gguf")),
            Err(GgufError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_kv_count_over_limit() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&(MAX_KV_COUNT + 1).to_le_bytes()); // 声称的 kv 数超限
        let dir = write_temp(&bytes);
        assert!(matches!(
            parse_model_meta(&dir.path().join("m.gguf")),
            Err(GgufError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_string_length_over_limit() {
        // 手动拼一条声称超长字符串的 key（不实际写那么多字节，读取应该在检查长度后
        // 就报错，不会真的尝试分配/读取 64KiB+1 字节）
        let mut malformed = MAGIC.to_vec();
        malformed.extend_from_slice(&3u32.to_le_bytes());
        malformed.extend_from_slice(&0u64.to_le_bytes());
        malformed.extend_from_slice(&1u64.to_le_bytes()); // kv_count = 1
        malformed.extend_from_slice(&(MAX_STRING_LEN + 1).to_le_bytes()); // key 长度超限
        let dir = write_temp(&malformed);
        assert!(matches!(
            parse_model_meta(&dir.path().join("m.gguf")),
            Err(GgufError::Malformed(_))
        ));
    }

    #[test]
    fn rejects_truncated_file() {
        let dir = write_temp(b"GGUF"); // magic 齐了，后面什么都没有
        assert!(parse_model_meta(&dir.path().join("m.gguf")).is_err());
    }
}
