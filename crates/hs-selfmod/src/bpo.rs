//! Bellman Policy Optimization training primitives.
//!
//! This is the paper's practical, critic-free RLVR objective. It is kept
//! separate from HAIRSPRING's discrete prompt/tool promotion loop: BPO updates
//! model weights from grouped terminal rewards and token probabilities; it is
//! not a substitute for held-out promotion evidence.

#[derive(Debug, Clone, Copy)]
pub struct BpoConfig {
    pub smoothing: f64,
    pub cap: f64,
    pub clip_low: f64,
    pub clip_high: f64,
}
impl Default for BpoConfig {
    fn default() -> Self {
        Self {
            smoothing: 0.1,
            cap: 3.0,
            clip_low: 0.2,
            clip_high: 0.28,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenTerm {
    pub advantage: f64,
    pub mismatch_weight: f64,
    pub mask: bool,
    pub loss: f64,
}

/// Group-normalized terminal-reward advantages. A constant-reward group has
/// zero learning signal, matching the RLVR objective rather than dividing by 0.
pub fn normalized_advantages(rewards: &[f64]) -> Vec<f64> {
    if rewards.is_empty() {
        return Vec::new();
    }
    let mean = rewards.iter().sum::<f64>() / rewards.len() as f64;
    let var = rewards.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / rewards.len() as f64;
    let std = var.sqrt();
    if std <= f64::EPSILON {
        vec![0.0; rewards.len()]
    } else {
        rewards.iter().map(|r| (r - mean) / std).collect()
    }
}

/// One token of the practical BPO loss:
/// `-advantage * mask * min((1+eps-mu)/(1+eps-pi), cap) * log(pi)`.
/// `rollout_probability` is mu; `current_probability` is pi.
pub fn token_term(
    advantage: f64,
    rollout_probability: f64,
    current_probability: f64,
    cfg: BpoConfig,
) -> Result<TokenTerm, String> {
    if !(0.0..=1.0).contains(&rollout_probability)
        || !(0.0..=1.0).contains(&current_probability)
        || current_probability == 0.0
    {
        return Err("token probabilities must be in (0,1] for pi and [0,1] for mu".into());
    }
    if cfg.smoothing < 0.0 || cfg.cap <= 0.0 {
        return Err("smoothing must be nonnegative and cap positive".into());
    }
    let w =
        (1.0 + cfg.smoothing - rollout_probability) / (1.0 + cfg.smoothing - current_probability);
    let mask = !((advantage > 0.0 && w > 1.0 + cfg.clip_high)
        || (advantage < 0.0 && w < 1.0 - cfg.clip_low));
    let loss = if mask {
        -advantage * w.min(cfg.cap) * current_probability.ln()
    } else {
        0.0
    };
    Ok(TokenTerm {
        advantage,
        mismatch_weight: w,
        mask,
        loss,
    })
}
