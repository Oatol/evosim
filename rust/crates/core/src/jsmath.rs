//! 与 JavaScript `Math` 位级一致的数学函数。
//!
//! 为什么单独开一个模块：黄金快照要求"同种子同 tick 同状态"，而模拟是混沌系统——
//! 一次 `log` 差 1 ulp，几十 tick 后就可能改变某个个体走哪一格。
//!
//! 目前这里是 **std 直通**，靠 `tools/export-golden.mjs` 导出的 10 万组样本
//! 做经验校验（`evosim --check-math`）。如果校验发现不一致，只需要把这个文件
//! 换成 fdlibm 移植版——调用点一处都不用改。
//!
//! 实测记录（2026-10-17）：见 `rust/golden/README.md`。

#[inline]
pub fn log(x: f64) -> f64 {
    x.ln()
}

#[inline]
pub fn tanh(x: f64) -> f64 {
    x.tanh()
}

#[inline]
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_matches_known_values() {
        assert!((log(1.0) - 0.0).abs() < 1e-15);
        assert!((log(std::f64::consts::E) - 1.0).abs() < 1e-15);
        assert!((log(10.0) - std::f64::consts::LN_10).abs() < 1e-14);
    }

    #[test]
    fn tanh_matches_known_values() {
        assert!((tanh(0.0) - 0.0).abs() < 1e-15);
        assert!((tanh(1.0) - 0.761_594_155_955_764_9).abs() < 1e-14);
        assert!((tanh(-2.0) + 0.964_027_580_075_816_9).abs() < 1e-14);
        assert_eq!(tanh(30.0), 1.0);
    }
}
