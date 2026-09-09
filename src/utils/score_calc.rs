#![allow(dead_code)]

/// Calculates accuracy percentage (0.00 to 100.00) based on game mode and hit counts
pub fn calculate_accuracy(
    mode: u8,
    c300: i32,
    c100: i32,
    c50: i32,
    c_geki: i32,
    c_katu: i32,
    miss: i32,
) -> f32 {
    match mode {
        // Standard (osu!)
        0 => {
            let total_hits = c300 + c100 + c50 + miss;
            if total_hits == 0 {
                return 0.0;
            }
            let total_points = (c300 * 300 + c100 * 100 + c50 * 50) as f64;
            let max_points = (total_hits * 300) as f64;
            ((total_points / max_points) * 100.0) as f32
        }
        // Taiko
        1 => {
            let total_hits = c300 + c100 + miss;
            if total_hits == 0 {
                return 0.0;
            }
            let total_points = (c300 * 2 + c100) as f64;
            let max_points = (total_hits * 2) as f64;
            ((total_points / max_points) * 100.0) as f32
        }
        // Catch the Beat
        2 => {
            let total_fruits = c300 + c100 + c50 + miss + c_katu;
            if total_fruits == 0 {
                return 0.0;
            }
            let caught = (c300 + c100 + c50) as f64;
            ((caught / total_fruits as f64) * 100.0) as f32
        }
        // Mania
        3 => {
            let total_hits = c300 + c100 + c50 + c_geki + c_katu + miss;
            if total_hits == 0 {
                return 0.0;
            }
            let total_points =
                (c_geki * 305 + c300 * 300 + c_katu * 200 + c100 * 100 + c50 * 50) as f64;
            let max_points = (total_hits * 305) as f64;
            ((total_points / max_points) * 100.0) as f32
        }
        _ => 0.0,
    }
}

/// Calculates letter grade string ("SS", "S", "A", "B", "C", "D")
pub fn calculate_grade(
    mode: u8,
    c300: i32,
    c100: i32,
    c50: i32,
    miss: i32,
    mods: u32,
) -> &'static str {
    let has_hd_fl = (mods & (1 << 3)) != 0 || (mods & (1 << 10)) != 0; // HD or FL
    let total = c300 + c100 + c50 + miss;
    if total == 0 {
        return "D";
    }

    if mode == 0 {
        let p300 = c300 as f32 / total as f32;
        let p50 = c50 as f32 / total as f32;

        if p300 == 1.0 {
            if has_hd_fl { "SSH" } else { "SS" }
        } else if p300 > 0.90 && p50 <= 0.01 && miss == 0 {
            if has_hd_fl { "SH" } else { "S" }
        } else if (p300 > 0.80 && miss == 0) || p300 > 0.90 {
            "A"
        } else if (p300 > 0.70 && miss == 0) || p300 > 0.80 {
            "B"
        } else if p300 > 0.60 {
            "C"
        } else {
            "D"
        }
    } else {
        let acc = calculate_accuracy(mode, c300, c100, c50, 0, 0, miss);
        if acc >= 100.0 {
            if has_hd_fl { "SSH" } else { "SS" }
        } else if acc >= 95.0 {
            if has_hd_fl { "SH" } else { "S" }
        } else if acc >= 90.0 {
            "A"
        } else if acc >= 80.0 {
            "B"
        } else if acc >= 70.0 {
            "C"
        } else {
            "D"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_perfect_accuracy() {
        let acc = calculate_accuracy(0, 500, 0, 0, 0, 0, 0);
        assert!((acc - 100.0).abs() < 0.001);
        assert_eq!(calculate_grade(0, 500, 0, 0, 0, 0), "SS");
    }

    #[test]
    fn test_accuracy_with_misses() {
        let acc = calculate_accuracy(0, 100, 10, 5, 0, 0, 5);
        assert!(acc > 0.0 && acc < 100.0);
    }
}
