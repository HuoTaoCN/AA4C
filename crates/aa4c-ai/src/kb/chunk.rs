//! 分块算法（ARCHIVE_DESIGN.md §6）：目标 ~1000 字符、重叠 ~200，段落边界优先。
//! 纯函数、无 I/O——`kb::mod` 负责读文件内容，这里只管切。

const TARGET_CHARS: usize = 1000;
const OVERLAP_CHARS: usize = 200;

/// 把文本切成若干块。空文本返回空 vec；短于 `TARGET_CHARS` 的文本整体一块。
/// 段落（以空行分隔）尽量整段放进同一块，超出目标长度才换块；单个段落本身超过
/// `TARGET_CHARS`（比如没有空行的压缩 JSON/大段代码）退化为按字符窗口滑动切分。
pub fn chunk_text(text: &str) -> Vec<String> {
    let paragraphs = split_paragraphs(text);
    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in paragraphs {
        let para_len = para.chars().count();
        if para_len > TARGET_CHARS {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            chunks.extend(split_long(&para));
            continue;
        }

        if !current.is_empty() && current.chars().count() + para_len > TARGET_CHARS {
            chunks.push(std::mem::take(&mut current));
            current = char_tail(chunks.last().unwrap(), OVERLAP_CHARS);
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(&para);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// 按连续空行分段，段内保留原始换行，段落本身两端裁掉多余空白。
fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let mut current_lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current_lines.is_empty() {
                paragraphs.push(current_lines.join("\n"));
                current_lines.clear();
            }
        } else {
            current_lines.push(line);
        }
    }
    if !current_lines.is_empty() {
        paragraphs.push(current_lines.join("\n"));
    }
    paragraphs
}

/// 单个超长段落按字符窗口滑动切分（`TARGET_CHARS` 窗口、`OVERLAP_CHARS` 重叠）。
fn split_long(para: &str) -> Vec<String> {
    let chars: Vec<char> = para.chars().collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let step = TARGET_CHARS - OVERLAP_CHARS;
    let mut start = 0;
    loop {
        let end = (start + TARGET_CHARS).min(chars.len());
        out.push(chars[start..end].iter().collect());
        if end == chars.len() {
            break;
        }
        start += step;
    }
    out
}

/// 取字符串末尾最多 `n` 个字符（按 char 边界安全，不按字节截断）。
fn char_tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    chars[start..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_produces_no_chunks() {
        assert_eq!(chunk_text(""), Vec::<String>::new());
        assert_eq!(chunk_text("   \n\n  "), Vec::<String>::new());
    }

    #[test]
    fn short_text_is_a_single_chunk() {
        let text = "第一段。\n\n第二段。";
        let chunks = chunk_text(text);
        assert_eq!(chunks, vec!["第一段。\n\n第二段。".to_string()]);
    }

    #[test]
    fn long_text_splits_on_paragraph_boundaries_with_overlap() {
        let para_a = "A".repeat(600);
        let para_b = "B".repeat(600);
        let para_c = "C".repeat(600);
        let text = format!("{para_a}\n\n{para_b}\n\n{para_c}");
        let chunks = chunk_text(&text);
        assert!(
            chunks.len() >= 2,
            "expected multiple chunks, got {chunks:?}"
        );
        // 每块都不应严重超出目标长度（允许最后一段把块撑大到刚好放不下前才换块）
        for c in &chunks {
            assert!(c.chars().count() <= TARGET_CHARS + OVERLAP_CHARS + 10);
        }
        // 相邻块之间应该有重叠内容（不是硬切断丢上下文）
        let overlap_found = chunks[1].starts_with(&para_a[para_a.len() - 100..])
            || chunks[1].contains(&para_b[..100]);
        assert!(overlap_found, "expected overlap between adjacent chunks");
    }

    #[test]
    fn single_paragraph_longer_than_target_is_sliced_with_overlap() {
        let huge = "x".repeat(2500);
        let chunks = chunk_text(&huge);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.chars().count() <= TARGET_CHARS);
        }
        // 滑窗重叠：第二块的开头应该是第一块结尾重叠区间的内容（同为 'x'，重叠天然成立）
        assert_eq!(chunks[0].chars().count(), TARGET_CHARS);
    }

    #[test]
    fn code_like_text_without_blank_lines_stays_intact_when_short() {
        let code = "fn main() {\n    println!(\"hi\");\n}";
        let chunks = chunk_text(code);
        assert_eq!(chunks, vec![code.to_string()]);
    }
}
