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

/// Calculates realistic Performance Points (PP) for a score
pub fn calculate_score_pp(
    mode: u8,
    mods: u32,
    max_combo: i32,
    c300: i32,
    c100: i32,
    c50: i32,
    c_geki: i32,
    c_katu: i32,
    c_miss: i32,
    score: i64,
    map_stars: Option<f64>,
    map_max_combo: Option<i32>,
) -> f64 {
    let total_hits = match mode {
        0 => c300 + c100 + c50 + c_miss,
        1 => c300 + c100 + c_miss,
        2 => c300 + c100 + c50 + c_miss + c_katu,
        3 => c300 + c100 + c50 + c_geki + c_katu + c_miss,
        _ => c300 + c100 + c50 + c_miss,
    };

    if total_hits <= 0 {
        return 0.0;
    }

    let acc = calculate_accuracy(mode, c300, c100, c50, c_geki, c_katu, c_miss) as f64;

    let full_combo = map_max_combo.unwrap_or(0);
    let expected_combo = if full_combo > 0 {
        full_combo
    } else {
        total_hits.max(1)
    };
    let combo_ratio = (max_combo as f64 / expected_combo as f64).clamp(0.0, 1.0);

    let sr = match map_stars {
        Some(s) if s > 0.0 => s,
        _ => {
            let base = (total_hits as f64).max(30.0);
            ((base / 100.0).sqrt() * 1.6 + 1.2).clamp(1.0, 9.5)
        }
    };

    let mut pp = match mode {
        // Standard (osu!)
        0 => {
            let base_pp = (sr / 2.2).powf(3.2) * 16.0;
            let combo_factor = combo_ratio.powf(0.85);
            let acc_factor = ((acc - 60.0).max(0.0) / 40.0).powf(2.5);
            let miss_penalty = 0.97_f64.powi(c_miss)
                * (1.0 - (c_miss as f64 / total_hits as f64) * 4.0).max(0.0);
            base_pp * combo_factor * acc_factor * miss_penalty
        }
        // Taiko
        1 => {
            let base_pp = (sr / 2.1).powf(3.1) * 15.0;
            let combo_factor = combo_ratio.powf(0.9);
            let acc_factor = ((acc - 70.0).max(0.0) / 30.0).powf(2.8);
            let miss_penalty = 0.96_f64.powi(c_miss);
            base_pp * combo_factor * acc_factor * miss_penalty
        }
        // Catch the Beat
        2 => {
            let base_pp = (sr / 2.0).powf(3.0) * 15.0;
            let combo_factor = combo_ratio.powf(0.8);
            let acc_factor = (acc / 100.0).powf(4.0);
            let miss_penalty = 0.95_f64.powi(c_miss);
            base_pp * combo_factor * acc_factor * miss_penalty
        }
        // Mania
        3 => {
            let score_ratio = (score as f64 / 1_000_000.0).clamp(0.0, 1.0);
            let score_factor = if score_ratio > 0.9 {
                (score_ratio - 0.5) / 0.5
            } else if score_ratio > 0.8 {
                ((score_ratio - 0.5) / 0.5).powi(2)
            } else {
                score_ratio.powi(4)
            };
            let base_pp = (sr / 2.0).powf(3.2) * 14.0;
            let acc_factor = ((acc - 70.0).max(0.0) / 30.0).powf(2.2);
            base_pp * score_factor * acc_factor
        }
        _ => 0.0,
    };

    // Mods multiplier
    let has_hd = (mods & (1 << 3)) != 0;
    let has_hr = (mods & (1 << 4)) != 0;
    let has_dt = (mods & (1 << 6)) != 0 || (mods & (1 << 9)) != 0;
    let has_fl = (mods & (1 << 10)) != 0;
    let has_ez = (mods & (1 << 1)) != 0;
    let has_ht = (mods & (1 << 8)) != 0;
    let has_nf = (mods & (1 << 0)) != 0;
    let has_so = (mods & (1 << 12)) != 0;
    let has_rx = (mods & (1 << 7)) != 0;

    let mut mod_mult = 1.0;
    if has_hd {
        mod_mult *= 1.06;
    }
    if has_hr {
        mod_mult *= 1.10;
    }
    if has_dt {
        mod_mult *= 1.28;
    }
    if has_fl {
        mod_mult *= 1.20;
    }
    if has_ez {
        mod_mult *= 0.50;
    }
    if has_ht {
        mod_mult *= 0.65;
    }
    if has_so {
        mod_mult *= 0.95;
    }
    if has_rx {
        mod_mult *= 0.65;
    }
    if has_nf && c_miss > 0 {
        mod_mult *= 0.90;
    }

    pp *= mod_mult;
    if pp.is_nan() || pp < 0.0 {
        0.0
    } else {
        pp
    }
}

/// Calculates total weighted PP from an array of descending best PPs:
/// Total PP = sum(pp_i * 0.95^i) + bonus_pp
pub fn calculate_weighted_pp(sorted_desc_pps: &[f64]) -> f64 {
    let mut total_pp = 0.0;
    let mut weight = 1.0;
    for &pp in sorted_desc_pps {
        total_pp += pp * weight;
        weight *= 0.95;
    }
    // Bonus PP: up to 416.6667 for submitting plays
    let n = sorted_desc_pps.len();
    let bonus = 416.6667 * (1.0 - 0.9994_f64.powi(n as i32));
    total_pp + bonus
}

/// Calculates weighted profile accuracy from an array of accuracies ordered by descending PP:
/// Weighted Acc = sum(acc_i * 0.95^i) / sum(0.95^i)
pub fn calculate_weighted_accuracy(accs_by_best_pp: &[f64]) -> f64 {
    if accs_by_best_pp.is_empty() {
        return 0.0;
    }
    let mut weighted_acc_sum = 0.0;
    let mut weight_sum = 0.0;
    let mut weight = 1.0;
    for &acc in accs_by_best_pp {
        weighted_acc_sum += acc * weight;
        weight_sum += weight;
        weight *= 0.95;
    }
    if weight_sum <= 0.0 {
        0.0
    } else {
        (weighted_acc_sum / weight_sum).clamp(0.0, 100.0)
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

    #[test]
    fn test_calculate_score_pp_modes() {
        // Standard FC 5 stars 100% acc
        let pp_std = calculate_score_pp(0, 0, 500, 500, 0, 0, 0, 0, 0, 1000000, Some(5.0), Some(500));
        assert!(pp_std > 200.0 && pp_std < 300.0, "Expected std ~240pp, got {}", pp_std);

        // DT multiplier should increase PP
        let pp_dt = calculate_score_pp(0, 64, 500, 500, 0, 0, 0, 0, 0, 1000000, Some(5.0), Some(500));
        assert!(pp_dt > pp_std, "DT PP should be higher than nomod PP");

        // HD multiplier
        let pp_hd = calculate_score_pp(0, 8, 500, 500, 0, 0, 0, 0, 0, 1000000, Some(5.0), Some(500));
        assert!(pp_hd > pp_std, "HD PP should be higher than nomod PP");

        // EZ multiplier
        let pp_ez = calculate_score_pp(0, 2, 500, 500, 0, 0, 0, 0, 0, 1000000, Some(5.0), Some(500));
        assert!(pp_ez < pp_std, "EZ PP should be lower than nomod PP");

        // Taiko
        let pp_taiko = calculate_score_pp(1, 0, 500, 500, 0, 0, 0, 0, 0, 1000000, Some(5.0), Some(500));
        assert!(pp_taiko > 0.0);

        // Catch
        let pp_ctb = calculate_score_pp(2, 0, 500, 500, 0, 0, 0, 0, 0, 1000000, Some(5.0), Some(500));
        assert!(pp_ctb > 0.0);

        // Mania
        let pp_mania = calculate_score_pp(3, 0, 500, 500, 0, 0, 500, 0, 0, 1000000, Some(5.0), Some(500));
        assert!(pp_mania > 0.0);
    }

    #[test]
    fn test_weighted_pp_and_acc() {
        let pps = vec![250.0, 200.0, 150.0];
        let total_pp = calculate_weighted_pp(&pps);
        // 250*1.0 + 200*0.95 + 150*0.9025 = 250 + 190 + 135.375 = 575.375 + bonus (~0.75)
        assert!(total_pp > 575.0);

        let accs = vec![100.0, 95.0];
        let total_acc = calculate_weighted_accuracy(&accs);
        // (100*1.0 + 95*0.95) / (1.0 + 0.95) = (100 + 90.25) / 1.95 = 97.564...
        assert!((total_acc - 97.564).abs() < 0.01);
    }
}
