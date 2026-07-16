/// 去除模型名中的上下文窗口标记，如 `deepseek-v4-pro[1m]` → `deepseek-v4-pro`
pub(crate) fn remove_size_marker(name: &str) -> String {
    let mut result = name.to_string();
    let mut i = 0;
    let chars = name.chars().collect::<Vec<_>>();
    while i < chars.len() {
        if chars[i] == '[' {
            let mut j = i + 1;
            let mut is_size = true;
            while j < chars.len() && chars[j] != ']' {
                let c = chars[j];
                if !(c.is_ascii_digit() || c == 'k' || c == 'm' || c == 'K' || c == 'M') {
                    is_size = false;
                }
                j += 1;
            }
            if j < chars.len() && chars[j] == ']' && is_size {
                let marker = &name[i..=j];
                result = result.replace(marker, "");
            }
        }
        i += 1;
    }
    result
}

/// 读取模型名末尾的上下文窗口标记，如 `deepseek-v4-pro[1m]` → `Some(1_000_000)`。
pub(crate) fn context_size_marker(name: &str) -> Option<u64> {
    let start = name.rfind('[')?;
    let marker = name.get(start + 1..)?.strip_suffix(']')?;
    let (number, multiplier) = match marker.chars().last()?.to_ascii_lowercase() {
        'k' => (&marker[..marker.len() - 1], 1_000),
        'm' => (&marker[..marker.len() - 1], 1_000_000),
        _ => return None,
    };
    number.parse::<u64>().ok()?.checked_mul(multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_size_marker_strips_size_markers() {
        assert_eq!(remove_size_marker("qwen3.7-plus[1m]"), "qwen3.7-plus");
        assert_eq!(remove_size_marker("deepseek-v4-pro[1M]"), "deepseek-v4-pro");
    }

    #[test]
    fn remove_size_marker_keeps_non_size_brackets() {
        assert_eq!(remove_size_marker("model[v2]"), "model[v2]");
    }

    #[test]
    fn context_size_marker_reads_terminal_size_marker() {
        assert_eq!(context_size_marker("gpt-5.6-sol[353k]"), Some(353_000));
        assert_eq!(context_size_marker("deepseek-v4-pro[1M]"), Some(1_000_000));
        assert_eq!(context_size_marker("model[v2]"), None);
        assert_eq!(context_size_marker("model[353k]-beta"), None);
    }
}
