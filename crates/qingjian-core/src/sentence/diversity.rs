//! 按词路径前缀逐层挑代表；调用方按总分降序传入路径。

use std::collections::HashSet;

/// 优先覆盖最早发生的分歧，再深入共同前缀后的分歧；预算不足时高分先入。
pub(crate) fn representative_indices(routes: &[&str], limit: usize) -> Vec<usize> {
    let mut selected = Vec::new();
    let mut chosen = vec![false; routes.len()];
    let max_depth = routes
        .iter()
        .map(|route| route.split('\0').count())
        .max()
        .unwrap_or(0);
    for depth in 1..=max_depth {
        let mut seen = HashSet::new();
        for (index, route) in routes.iter().enumerate() {
            let prefix = route.split('\0').take(depth).collect::<Vec<_>>();
            if seen.insert(prefix) && !chosen[index] {
                selected.push(index);
                chosen[index] = true;
                if selected.len() >= limit {
                    return selected;
                }
            }
        }
    }
    selected
}
