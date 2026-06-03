/// Fuzzy scoring algorithm matching the bash script's fuzzy_score function.
///
/// Scoring rules:
/// 1. Empty query → 0
/// 2. Substring match → 100000 - prefix_length*100 - model_length
/// 3. Fuzzy char-by-char match → 50000 - gaps*100 - last_position - model_length
/// 4. No match → None
pub fn fuzzy_score(query: &str, model: &str) -> Option<i64> {
    let q = query.to_lowercase();
    let m = model.to_lowercase();

    if q.is_empty() {
        return Some(0);
    }

    // Substring match: high bonus
    if let Some(pos) = m.find(&q) {
        return Some(100_000 - (pos as i64) * 100 - (model.len() as i64));
    }

    // Fuzzy character-by-character match
    let mut gaps = 0i64;
    let mut last = -1i64;
    let mut search_from = 0usize;

    for ch in q.chars() {
        if let Some(pos) = m[search_from..].find(ch) {
            let actual_pos = search_from + pos;
            gaps += (actual_pos - search_from) as i64;
            last = actual_pos as i64;
            search_from = actual_pos + ch.len_utf8();
        } else {
            return None;
        }
    }

    Some(50_000 - gaps * 100 - last - (model.len() as i64))
}

/// Rank and filter models based on query. Returns (filtered_models, scores).
#[allow(dead_code)]
pub fn rank_models(query: &str, models: &[String]) -> Vec<String> {
    let mut scored: Vec<(i64, &str)> = models
        .iter()
        .filter_map(|m| fuzzy_score(query, m).map(|s| (s, m.as_str())))
        .collect();

    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    scored.into_iter().map(|(_, m)| m.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_query() {
        assert_eq!(fuzzy_score("", "claude-sonnet"), Some(0));
    }

    #[test]
    fn test_substring_match() {
        let score = fuzzy_score("sonnet", "claude-sonnet-4-20250514");
        assert!(score.is_some());
        // Should be in the 100000 range (substring bonus)
        assert!(score.unwrap() > 50_000);
    }

    #[test]
    fn test_fuzzy_match() {
        // "sn" should match "claude-sonnet"
        let score = fuzzy_score("sn", "claude-sonnet");
        assert!(score.is_some());
        assert!(score.unwrap() < 50_000);
    }

    #[test]
    fn test_no_match() {
        assert!(fuzzy_score("xyz", "claude-sonnet").is_none());
    }

    #[test]
    fn test_ranking_order() {
        let models = vec![
            "claude-haiku-4-20250514".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-20250514".to_string(),
        ];
        let ranked = rank_models("sonnet", &models);
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0], "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_ranking_all_match() {
        let models = vec![
            "claude-haiku-4-20250514".to_string(),
            "claude-sonnet-4-20250514".to_string(),
            "claude-opus-4-20250514".to_string(),
        ];
        let ranked = rank_models("claude", &models);
        // All should match, "claude-haiku" first (shortest prefix)
        assert_eq!(ranked.len(), 3);
    }
}
