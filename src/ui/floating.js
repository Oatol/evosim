/**
 * 浮窗通用行为：拖头部移动。
 *
 * 检视器和物种工坊都是"不挡画布、可以拖到一边"的浮窗，
 * 拖动逻辑抽在这里，免得两处各写一份（写两份的结果通常是其中一份漏了边界钳制）。
 */

/**
 * 让 `handle` 可以拖动 `panel`。
 * 按钮上按下不触发拖动（否则点"关闭"会顺手把窗口挪走）。
 */
export function enableDrag(handle, panel) {
  let dragging = false;
  let startX = 0;
  let startY = 0;
  let baseX = 0;
  let baseY = 0;

  handle.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || e.target.closest('button')) return;
    const rect = panel.getBoundingClientRect();
    dragging = true;
    baseX = rect.left;
    baseY = rect.top;
    startX = e.clientX;
    startY = e.clientY;
    // 从 CSS 默认定位切到像素定位，拖动才不会跳
    panel.style.left = baseX + 'px';
    panel.style.top = baseY + 'px';
    panel.style.right = 'auto';
    panel.classList.add('is-dragging');
    // 合成事件里的 pointerId 不是真实指针，setPointerCapture 会抛
    try { handle.setPointerCapture(e.pointerId); } catch { /* 忽略 */ }
    e.preventDefault();
  });

  handle.addEventListener('pointermove', (e) => {
    if (!dragging) return;
    const nx = baseX + (e.clientX - startX);
    const ny = baseY + (e.clientY - startY);
    panel.style.left = Math.max(0, Math.min(nx, window.innerWidth - 80)) + 'px';
    panel.style.top = Math.max(0, Math.min(ny, window.innerHeight - 40)) + 'px';
  });

  const end = (e) => {
    if (!dragging) return;
    dragging = false;
    panel.classList.remove('is-dragging');
    try {
      if (handle.hasPointerCapture(e.pointerId)) handle.releasePointerCapture(e.pointerId);
    } catch { /* 忽略 */ }
  };
  handle.addEventListener('pointerup', end);
  handle.addEventListener('pointercancel', end);
}
