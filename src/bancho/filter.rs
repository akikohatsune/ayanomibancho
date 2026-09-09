use std::fs;
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug, Clone, Default)]
pub struct ChatFilter {
    replacements: Vec<(String, String)>,
}

impl ChatFilter {
    pub fn new() -> Self {
        Self {
            replacements: Vec::new(),
        }
    }

    pub fn load_or_create<P: AsRef<Path>>(path: P) -> Self {
        let p = path.as_ref();
        if !p.exists() {
            let default_content = "fuck=firetruck\nshit=shish\nbitch=fine lady\n";
            let _ = fs::write(p, default_content);
        }

        let mut replacements = Vec::new();
        match fs::read_to_string(p) {
            Ok(content) => {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }
                    if let Some((word, rep)) = trimmed.split_once('=') {
                        let w = word.trim().to_lowercase();
                        let r = rep.trim().to_string();
                        if !w.is_empty() {
                            replacements.push((w, r));
                        }
                    } else {
                        let w = trimmed.to_lowercase();
                        let r = "*".repeat(w.chars().count().max(1));
                        replacements.push((w, r));
                    }
                }
                info!("Loaded {} chat filter rules from {:?}", replacements.len(), p);
            }
            Err(e) => {
                warn!("Failed to read chat filters from {:?}: {}", p, e);
            }
        }

        // Sort descending by pattern length so longer phrases get replaced first
        replacements.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()));

        Self { replacements }
    }

    pub fn filter(&self, text: &str) -> String {
        if self.replacements.is_empty() || text.is_empty() {
            return text.to_string();
        }

        let mut current = text.to_string();
        for (pattern, replacement) in &self.replacements {
            let pattern_chars: Vec<char> = pattern.chars().collect();
            let pattern_len = pattern_chars.len();
            if pattern_len == 0 {
                continue;
            }

            let mut new_text = String::with_capacity(current.len());
            let char_indices: Vec<(usize, char)> = current.char_indices().collect();
            let total_chars = char_indices.len();
            let mut last_end_byte = 0;
            let mut i = 0;

            while i < total_chars {
                if i + pattern_len <= total_chars {
                    let mut matches = true;
                    for j in 0..pattern_len {
                        let current_char_lower: Vec<char> = char_indices[i + j].1.to_lowercase().collect();
                        if current_char_lower.as_slice() != &[pattern_chars[j]] {
                            matches = false;
                            break;
                        }
                    }

                    if matches {
                        let start_byte = char_indices[i].0;
                        let end_byte = if i + pattern_len < total_chars {
                            char_indices[i + pattern_len].0
                        } else {
                            current.len()
                        };

                        new_text.push_str(&current[last_end_byte..start_byte]);
                        new_text.push_str(replacement);
                        last_end_byte = end_byte;
                        i += pattern_len;
                        continue;
                    }
                }
                i += 1;
            }

            new_text.push_str(&current[last_end_byte..]);
            current = new_text;
        }

        current
    }
}
