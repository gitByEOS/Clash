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
                if !(c.is_ascii_digit() || c == 'k' || c == 'm') {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_size_marker_strips_1m() {
        assert_eq!(remove_size_marker("qwen3.7-plus[1m]"), "qwen3.7-plus");
    }

    #[test]
    fn remove_size_marker_keeps_non_size_brackets() {
        assert_eq!(remove_size_marker("model[v2]"), "model[v2]");
    }
}
