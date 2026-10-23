import io

# 临时探针：统计变异调用次数
p = 'crates/core/src/mutation.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('''const MAX_ADD_ATTEMPTS: usize = 6;''',
'''const MAX_ADD_ATTEMPTS: usize = 6;

/// 临时诊断计数器（批次 45 调试用，定稿前删掉）。
pub static DBG_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static DBG_PASSED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static DBG_ADD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static DBG_REMOVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);''', 1)

s = s.replace('''    let rate = base * rad_boost;
    if rate <= 0.0 || !rng.chance(rate.min(100.0)) {
        return false;
    }''',
'''    let rate = base * rad_boost;
    DBG_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if rate <= 0.0 || !rng.chance(rate.min(100.0)) {
        return false;
    }
    DBG_PASSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);''', 1)

s = s.replace('''    let changed = mutate_anatomy(&mut a, rng, m, cfg.life.max_body_cells);
    if changed {''',
'''    let before = a.cells.len();
    let changed = mutate_anatomy(&mut a, rng, m, cfg.life.max_body_cells);
    if a.cells.len() > before {
        DBG_ADD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else if a.cells.len() < before {
        DBG_REMOVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if changed {''', 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok mutation')

p = 'crates/core/examples/grow_probe.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('''        w.step();
    }
}''',
'''        w.step();
    }
    use std::sync::atomic::Ordering::Relaxed;
    println!(
        "变异调用 {} · 通过门控 {} · 加细胞 {} · 删细胞 {}",
        evosim_core::mutation::DBG_CALLS.load(Relaxed),
        evosim_core::mutation::DBG_PASSED.load(Relaxed),
        evosim_core::mutation::DBG_ADD.load(Relaxed),
        evosim_core::mutation::DBG_REMOVE.load(Relaxed),
    );
}''', 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok probe')
