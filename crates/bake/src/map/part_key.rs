use std::collections::HashMap;

/// A mesh part's key in `keys` (shared by names that agree once `suffix` is dropped), whether it ignores backfaces, and its lowercase name without that suffix
pub(crate) fn part_key(name: &str, suffix: &str, backface_suffix: &str, keys: &mut HashMap<String, u32>) -> (u32, bool, String) {
    let name = name.trim().to_lowercase();
    let ignore_backface = !backface_suffix.is_empty() && name.ends_with(backface_suffix);
    let name = if ignore_backface { name.strip_suffix(backface_suffix).unwrap_or(&name).to_owned() } else { name };

    (match_key(&name, suffix, keys), ignore_backface, name)
}

/// The key in `keys` of the lowercase part name `name` without `suffix`
fn match_key(name: &str, suffix: &str, keys: &mut HashMap<String, u32>) -> u32 {
    let suffix = suffix.trim().to_lowercase();
    let base = name.strip_suffix(suffix.as_str()).filter(|_| !suffix.is_empty()).unwrap_or(name).to_string();
    let next = keys.len() as u32;
    *keys.entry(base).or_insert(next)
}
