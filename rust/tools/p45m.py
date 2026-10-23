import io

# ── 1) 移除临时诊断计数器 ──
p = 'crates/core/src/mutation.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('''
/// 临时诊断计数器（批次 45 调试用，定稿前删掉）。
pub static DBG_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static DBG_PASSED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static DBG_ADD: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static DBG_REMOVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);''', '', 1)
s = s.replace('''    DBG_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if rate <= 0.0 || !rng.chance(rate.min(100.0)) {
        return false;
    }
    DBG_PASSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);''',
'''    if rate <= 0.0 || !rng.chance(rate.min(100.0)) {
        return false;
    }''', 1)
s = s.replace('''    let before = a.cells.len();
    let changed = mutate_anatomy(&mut a, rng, m, cfg.life.max_body_cells);
    if a.cells.len() > before {
        DBG_ADD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else if a.cells.len() < before {
        DBG_REMOVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if changed {''',
'''    let changed = mutate_anatomy(&mut a, rng, m, cfg.life.max_body_cells);
    if changed {''', 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok mutation')

# ── 2) 探针去掉计数器打印 ──
p = 'crates/core/examples/grow_probe.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace('''    use std::sync::atomic::Ordering::Relaxed;
    println!(
        "变异调用 {} · 通过门控 {} · 加细胞 {} · 删细胞 {}",
        evosim_core::mutation::DBG_CALLS.load(Relaxed),
        evosim_core::mutation::DBG_PASSED.load(Relaxed),
        evosim_core::mutation::DBG_ADD.load(Relaxed),
        evosim_core::mutation::DBG_REMOVE.load(Relaxed),
    );
''', '', 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok probe')

# ── 3) 清掉 world.rs 里未使用的 Anatomy 导入 ──
p = 'crates/core/src/world.rs'
s = io.open(p, encoding='utf-8').read()
s = s.replace("use crate::anatomy::{Anatomy, BodyCell, Flags};",
              "use crate::anatomy::{BodyCell, Flags};", 1)
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('ok world import')
