import { Config } from './core/config.js';
import { RNG } from './core/RNG.js';
import { World } from './core/World.js';
import { Engine } from './core/Engine.js';
import { generateDemoWorld } from './core/WorldGen.js';
import { seedOriginOfLife, spawnPopulation, makeBlueprint, spawnFromBlueprint } from './life/OrganismFactory.js';
import { FossilRecord } from './stats/FossilRecord.js';
import {
  serializeWorld, deserializeWorld, readSaveFile, saveFilename,
  saveToStorage, loadFromStorage, storageInfo, clearStorage, storageAvailable,
} from './io/SaveLoad.js';
import { CreatureLibrary } from './io/CreatureLibrary.js';
import {
  fossilToCSV, speciesToCSV, saveText, describeSaveTarget, exportFilename, hostSaveDir,
} from './io/Export.js';
import { Camera } from './render/Camera.js';
import { ColorScheme, COLOR_SCHEMES } from './render/ColorScheme.js';
import { Renderer } from './render/Renderer.js';
import { Editor } from './ui/Editor.js';
import { Toolbar } from './ui/Toolbar.js';
import { Stats } from './ui/Stats.js';
import { Charts } from './ui/Charts.js';
import { Inspector } from './ui/Inspector.js';
import { ControlPanel } from './ui/ControlPanel.js';
import { SpeciesTree } from './ui/SpeciesTree.js';
import { SpeciesEditor } from './ui/SpeciesEditor.js';
import { Timeline } from './ui/Timeline.js';

function boot() {
  const canvas = document.getElementById('world-canvas');
  const sidebarLeft = document.getElementById('sidebar-left');
  const sidebarRight = document.getElementById('sidebar-right');
  const timelineEl = document.getElementById('timeline');
  const paramHost = document.getElementById('param-host');
  const toastEl = document.getElementById('toast');

  // ── 核心对象 ──
  const rng = new RNG(Config.sim.seed);
  const world = new World(Config, rng);
  const record = new FossilRecord(Config.stats.recordSize);
  const camera = new Camera({ zoom: 4 });
  const colorScheme = new ColorScheme('classic');
  const renderer = new Renderer(canvas, world, camera, colorScheme);
  const editor = new Editor(canvas, world, camera, renderer);
  const engine = new Engine(world, {
    baseTicksPerSecond: Config.sim.baseTicksPerSecond,
    maxStepsPerFrame: Config.sim.maxStepsPerFrame,
  });

  // ── 初始视图 ──
  renderer.resize();
  camera.fitGrid(world.grid.cols, world.grid.rows, renderer.width, renderer.height, 20);
  regenerateWorld();

  /** 造一个新世界，并在中央播下生命 */
  function regenerateWorld() {
    generateDemoWorld(world);
    seedOriginOfLife(world);
    spawnPopulation(world, 6);
  }

  // ── UI ──
  // 右栏拆成两个独立容器。曲线放在最上面——它是主要看点，
  // 读数面板行数多，放上面会把曲线挤到折叠线以下。
  const chartEl = document.createElement('div');
  const readoutEl = document.createElement('div');
  sidebarRight.appendChild(chartEl);
  sidebarRight.appendChild(readoutEl);

  const toolbar = new Toolbar(sidebarLeft, editor, camera, renderer, colorScheme, world);
  const stats = new Stats(readoutEl, world, renderer);
  const charts = new Charts(chartEl, record);
  const timeline = new Timeline(timelineEl, engine, world);
  const controlPanel = new ControlPanel(paramHost, Config);
  toolbar.onWorldChanged = () => refreshAll();

  // ── 个体检视器 ──
  const inspector = new Inspector(colorScheme, world);
  inspector.onClose = () => {
    renderer.highlight = null;
    renderer.needsRender = true;
  };
  editor.onInspect = (organism) => {
    if (!organism) return;          // 点到空地：不改变当前选中
    inspector.markDirty();
    inspector.open(organism);
    renderer.highlight = organism;
    renderer.needsRender = true;
  };
  editor.onBrushChanged = () => toolbar.refreshBrush();

  // ── 系统发生树 ──
  const speciesTree = new SpeciesTree(world, colorScheme);
  // 树上选一个物种 → 世界画布上高亮它的全部存活个体
  speciesTree.onSelect = (sp) => {
    renderer.highlightFingerprint = sp ? sp.fingerprint : null;
    renderer.needsRender = true;
  };

  // ── 物种工坊（手绘蓝图 + 生物库）──
  const library = new CreatureLibrary();
  const speciesEditor = new SpeciesEditor(world, library, colorScheme);
  speciesEditor.onToast = (msg, isError) => toast(msg, isError);

  /** 视野中心对应的网格坐标 */
  function viewportCenterCell() {
    return camera.screenToCell(renderer.width / 2, renderer.height / 2);
  }

  speciesEditor.onSpawn = (blueprint, name) => {
    const [c0, r0] = viewportCenterCell();
    let placed = null;
    // 放不下就一圈圈往外找——总比什么都不做、让用户以为按钮坏了强
    for (let r = 0; r <= 8 && !placed; r++) {
      for (let dy = -r; dy <= r && !placed; dy++) {
        for (let dx = -r; dx <= r && !placed; dx++) {
          if (Math.max(Math.abs(dx), Math.abs(dy)) !== r) continue;
          placed = spawnFromBlueprint(world, c0 + dx, r0 + dy, blueprint);
        }
      }
    }
    if (placed) {
      refreshAll();
      toast(`已投放${name ? '：' + name : ''}（${placed.anatomy.describe()}）`);
    } else {
      toast('视野中心附近没有空位，先把画布移到空旷处再投放', true);
    }
  };

  /**
   * 手动放置：把蓝图装进放置笔刷，并**收起工坊**让出画布——
   * 下一步就是点画布，面板挡着会很难受（和"克隆选中"是同一条道理）。
   */
  speciesEditor.onPlace = (blueprint, name) => {
    editor.setCloneBlueprint(blueprint);
    toolbar.refreshBrush();
    speciesEditor.close();
    toast(`已装入「${name || blueprint.label}」　在画布上点哪里就放在哪里（Esc 取消）`);
  };

  /** 放置结果反馈：一次点击/拖动结束后汇报成功与失败 */
  editor.onPlaceFeedback = (placed, missed) => {
    if (placed > 0) {
      refreshAll();
      toast(`已放置 ${placed} 个个体${missed > 0 ? `（${missed} 处被占，跳过）` : ''}`);
    } else if (missed > 0) {
      toast('这里放不下——换个空一点的位置', true);
    }
  };

  // 检视器里的"保存生物"：连大脑一起存进生物库
  inspector.onSaveCreature = (org) => {
    const sp = world.species.byFingerprint(org.speciesFingerprint);
    const entry = library.add(makeBlueprint(org), sp ? sp.name : '');
    if (!entry) { toast('保存失败：这个个体没有身体', true); return; }
    speciesEditor.refreshLibrary();
    toast(`已存进生物库：${entry.name}（${org.anatomy.describe()}）`);
  };

  // ── 导出 ──
  // 桌面版（evosim.exe）会写进程序所在目录；浏览器版退回下载。
  toolbar.onExportFossil = async () => {
    const csv = fossilToCSV(record);
    const result = await saveText(csv, exportFilename('history', world.tick));
    toast(`${record.count} 个采样点 · ${describeSaveTarget(result)}`);
  };
  toolbar.onExportSpecies = async () => {
    const csv = speciesToCSV(world.species);
    const result = await saveText(csv, exportFilename('species', world.tick));
    toast(`${world.species.list.length} 条物种记录 · ${describeSaveTarget(result)}`);
  };

  // 克隆笔刷：把检视中的个体装进去，之后可以随手投放
  toolbar.onCloneSelected = () => {
    const org = inspector.organism;
    if (!org) {
      toast('先用「检视」工具点一个个体，再克隆它');
      return;
    }
    editor.setCloneBlueprint(makeBlueprint(org));
    toolbar.refreshBrush();
    // 检视器现在是浮窗、不再遮住画布，所以不用再关它——锁上还能边看边投放
    toast(`已装入蓝图：${org.anatomy.describe()}　点击画布即可投放`);
  };

  // ── 顶栏：配色方案 ──
  const schemeSelect = document.getElementById('scheme-select');
  for (const [key, scheme] of Object.entries(COLOR_SCHEMES)) {
    const opt = document.createElement('option');
    opt.value = key;
    opt.textContent = scheme.label;
    schemeSelect.appendChild(opt);
  }
  schemeSelect.value = colorScheme.name;
  schemeSelect.addEventListener('change', () => {
    colorScheme.setScheme(schemeSelect.value);
    renderer.refreshPalette();
    toolbar.refreshColors();
  });

  // ── 顶栏：着色模式（按类型 / 按物种）──
  const colorModeSelect = document.getElementById('color-mode');
  colorModeSelect.value = renderer.colorMode;
  colorModeSelect.addEventListener('change', () => {
    renderer.setColorMode(colorModeSelect.value);
  });

  // ── 顶栏：物种树 ──
  const speciesTreeBtn = document.getElementById('btn-species-tree');
  speciesTreeBtn.addEventListener('click', () => {
    speciesTree.toggle();
    speciesTreeBtn.classList.toggle('is-active', speciesTree.isOpen);
  });
  speciesTree.onOpenChanged = (open) => speciesTreeBtn.classList.toggle('is-active', open);

  // ── 顶栏：物种工坊 ──
  const speciesEditorBtn = document.getElementById('btn-species-editor');
  speciesEditorBtn.addEventListener('click', () => speciesEditor.toggle());
  speciesEditor.onOpenChanged = (open) => speciesEditorBtn.classList.toggle('is-active', open);

  // ── 顶栏：世界尺寸 ──
  const colsInput = document.getElementById('world-cols');
  const rowsInput = document.getElementById('world-rows');
  colsInput.value = String(world.grid.cols);
  rowsInput.value = String(world.grid.rows);

  const applyResize = () => {
    const cols = clampInt(colsInput.value, 40, 800, world.grid.cols);
    const rows = clampInt(rowsInput.value, 30, 600, world.grid.rows);
    colsInput.value = String(cols);
    rowsInput.value = String(rows);
    if (cols === world.grid.cols && rows === world.grid.rows) return;

    // 重建世界期间先停下，避免在半截状态里跑 tick；重建完再恢复原来的运行状态
    const wasRunning = engine.running;
    engine.pause();
    world.resize(cols, rows);
    renderer.notifyWorldResized();
    camera.fitGrid(cols, rows, renderer.width, renderer.height, 20);
    regenerateWorld();
    resetHistory();
    controlPanel.syncAll();
    refreshAll();
    if (wasRunning) engine.start();
    toast(`世界已重建为 ${cols} × ${rows}`);
  };
  document.getElementById('btn-apply-size').addEventListener('click', applyResize);
  colsInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') applyResize(); });
  rowsInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') applyResize(); });

  // ── 顶栏：世界操作 ──
  document.getElementById('btn-regenerate').addEventListener('click', () => {
    const wasRunning = engine.running;
    engine.pause();
    world.rng.seed(Date.now() & 0x7fffffff);
    regenerateWorld();
    resetHistory();
    refreshAll();
    if (wasRunning) engine.start();
  });

  // 清空是例外：清完世界是空的，停下来等用户画东西比空转更合理
  document.getElementById('btn-clear').addEventListener('click', () => {
    engine.pause();
    world.reset(false);
    resetHistory();
    refreshAll();
    timeline.refresh();
  });

  document.getElementById('btn-fit').addEventListener('click', () => toolbar.fitToWindow());

  // ── 顶栏：参数抽屉 ──
  const paramsBtn = document.getElementById('btn-params');
  paramsBtn.addEventListener('click', () => {
    controlPanel.toggle();
    paramsBtn.classList.toggle('is-active', controlPanel.isOpen);
  });

  controlPanel.onToggle = (open) => {
    paramsBtn.classList.toggle('is-active', open);
  };

  // 参数改动的副作用。绝大多数参数是"每 tick 现读"的，改了就生效；
  // 只有少数需要在改的瞬间做点事，全部收在这一个 switch 里。
  controlPanel.onChange = (section, key, value) => {
    if (section === 'world' && key === 'wrapEdges') {
      world.grid.wrap = value;
      renderer.requestFullRedraw();
    } else if (section === 'stats' && key === 'recordSize') {
      record.allocate(value);
      charts.markDirty();
    } else if (section === 'sim' && key === 'baseTicksPerSecond') {
      engine.setBaseTps(value);
    } else if (section === 'sim' && key === 'maxStepsPerFrame') {
      engine.maxStepsPerFrame = value;
    }
    renderer.needsRender = true;
  };

  // 网格尺寸是延迟生效的：滑块只把待应用值送过来，世界不会立刻被清空
  controlPanel.onDeferredChange = (section, key, value) => {
    if (section !== 'world') return;
    if (key === 'cols') colsInput.value = String(value);
    else if (key === 'rows') rowsInput.value = String(value);
  };

  controlPanel.onResizeRequest = () => applyResize();

  /** 读档会整体替换 Config，有些参数需要重新落到运行时对象上 */
  function applyConfigSideEffects() {
    world.grid.wrap = Config.world.wrapEdges;
    if (record.size !== Config.stats.recordSize) record.allocate(Config.stats.recordSize);
    engine.setBaseTps(Config.sim.baseTicksPerSecond);
    engine.maxStepsPerFrame = Config.sim.maxStepsPerFrame;
  }

  // ── 顶栏：存档 ──
  const fileInput = document.getElementById('save-file');

  const doSave = async () => {
    const data = serializeWorld(world, rng, { config: JSON.parse(JSON.stringify(Config)) });
    const result = await saveText(JSON.stringify(data), saveFilename(world),
      { mime: 'application/json', bom: false });
    toast(`${data.organisms.length} 个个体 · ${world.species.list.length} 条物种记录 · `
      + describeSaveTarget(result));
  };

  const doLoad = (data, source) => {
    const wasRunning = engine.running;
    engine.pause();
    try {
      const info = deserializeWorld(world, rng, data);
      applyConfigSideEffects();
      renderer.notifyWorldResized();
      camera.fitGrid(world.grid.cols, world.grid.rows, renderer.width, renderer.height, 20);
      resetHistory();
      controlPanel.syncAll();
      // 读档可能带来不同的世界尺寸，顶栏那两个输入框要跟着走
      colsInput.value = String(world.grid.cols);
      rowsInput.value = String(world.grid.rows);
      refreshAll();
      timeline.refresh();
      if (wasRunning) engine.start();
      const warn = info.brainSkipped > 0 ? ` · ${info.brainSkipped} 个大脑拓扑不符已重置` : '';
      toast(`${source}：tick ${world.tick} · ${info.organisms} 个个体 · ${info.species} 个物种${warn}`);
    } catch (err) {
      engine.start();
      toast(`读档失败：${err.message}`, true);
    }
  };

  document.getElementById('btn-save').addEventListener('click', doSave);

  document.getElementById('btn-load').addEventListener('click', () => {
    const quick = storageInfo();
    // 有快存就先问一句：直接弹文件选择框会让人以为快存不存在
    if (quick) {
      const when = quick.savedAt ? new Date(quick.savedAt).toLocaleString() : '未知时间';
      if (confirm(`检测到快速存档（${when}，tick ${quick.tick}）。\n\n确定 = 读取快速存档\n取消 = 从文件读取`)) {
        try {
          doLoad(loadFromStorage(), '快速读档');
        } catch (err) {
          toast(`快速读档失败：${err.message}`, true);
        }
        return;
      }
    }
    fileInput.click();
  });

  fileInput.addEventListener('change', async () => {
    const file = fileInput.files && fileInput.files[0];
    fileInput.value = '';              // 允许连续读同一个文件
    if (!file) return;
    try {
      doLoad(await readSaveFile(file), `已读取 ${file.name}`);
    } catch (err) {
      toast(`读档失败：${err.message}`, true);
    }
  });

  // ── 引擎回调 ──
  engine.onFrame = () => {
    // 相机跟随：把被跟随个体的当前位置喂给相机
    if (inspector.following) {
      const o = inspector.organism;
      if (!o || !o.alive) {
        inspector.setFollow(false);
      } else {
        camera.follow = { col: o.col, row: o.row };
        if (camera.updateFollow(renderer.width, renderer.height)) renderer.needsRender = true;
      }
    }
    if (renderer.needsRender) renderer.render();
  };

  // ── 化石记录采样 ──
  engine.onTick = () => {
    const interval = Config.stats.interval;
    if (interval > 0 && world.tick % interval === 0) sampleWorld();
    if (Config.world.autoReset && world.organisms.length === 0) autoReset();
  };

  let autoResetPending = false;
  function autoReset() {
    if (autoResetPending) return;
    autoResetPending = true;
    // 延迟到下一帧再重建：此刻还在 engine 的 tick 循环里，直接重建会让
    // 循环里的 organisms 数组引用变成悬空的旧数组
    setTimeout(() => {
      autoResetPending = false;
      world.rng.seed(Date.now() & 0x7fffffff);
      regenerateWorld();
      resetHistory();
      refreshAll();
      toast('种群灭绝，已自动重置世界');
    }, 0);
  }

  function sampleWorld() {
    const tick = world.tick;
    world.species.update(world, tick);
    world.species.prune();

    const bio = world.detailedStats();
    record.push({
      tick,
      population: world.organisms.length,
      species: world.species.aliveCount,
      avgCells: bio.avgCells,
      avgMutability: bio.avgMutability,
      cellShares: bio.cellShares,
    });
  }

  /** 世界重建后清空历史，并立刻采一个起点样本，曲线才不会从空白开始 */
  function resetHistory() {
    record.reset();
    charts.markDirty();
    inspector.close();              // 被检视的个体已经不存在了
    speciesTree.close();            // 物种表也重建了，旧树没有意义
    sampleWorld();
  }

  let readoutTimer = 0;
  engine.onAfterFrame = () => {
    readoutTimer += 1;
    // 用帧数节流而不是时间，保证暂停时读数依然会刷新一次
    if (readoutTimer >= 8) {
      readoutTimer = 0;
      refreshReadouts();
    }
  };

  function refreshReadouts() {
    timeline.refresh();
    timeline.refreshRates();
    stats.refresh();
    charts.refresh();     // 内部会跳过没有新样本的曲线
    if (inspector.isOpen) inspector.refresh();
    if (speciesTree.isOpen) speciesTree.refresh();
  }

  function refreshAll() {
    renderer.render();
    refreshReadouts();
  }

  // ── 提示条 ──
  let toastTimer = 0;
  function toast(message, isError = false) {
    toastEl.textContent = message;
    toastEl.classList.toggle('is-error', isError);
    toastEl.hidden = false;
    // 下一帧再加 is-visible，否则 hidden 刚去掉时 transition 不会触发
    requestAnimationFrame(() => toastEl.classList.add('is-visible'));
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      toastEl.classList.remove('is-visible');
      setTimeout(() => { toastEl.hidden = true; }, 220);
    }, isError ? 5200 : 2600);
  }

  // ── 画布尺寸跟随 ──
  const resizeObserver = new ResizeObserver(() => {
    renderer.resize();
    renderer.render();
  });
  resizeObserver.observe(canvas.parentElement);

  window.addEventListener('resize', () => {
    charts.onResize();
    speciesTree.onResize();
  });

  // ── 快捷键 ──
  window.addEventListener('keydown', (event) => {
    const tag = event.target && event.target.tagName;
    const typing = tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA';

    // Ctrl+S 存盘：比浏览器自带的"保存网页"更符合这个场景
    if ((event.ctrlKey || event.metaKey) && (event.key === 's' || event.key === 'S')) {
      event.preventDefault();
      doSave();
      return;
    }

    if (typing) return;

    if (event.key === 'Escape') {
      if (inspector.isOpen) inspector.close();
      else if (speciesEditor.isOpen) speciesEditor.close();
      else if (speciesTree.isOpen) speciesTree.close();
      else if (controlPanel.isOpen) controlPanel.close();
      // 都关完了还按 Esc，就退出"放置模式"——装好蓝图后它是唯一还挂着的状态
      else if (editor.isPlacing) {
        editor.cancelClone();
        toast('已取消放置');
      }
      return;
    }

    if (event.code === 'Space') {
      event.preventDefault();
      engine.toggle();
      timeline.refresh();
    } else if (event.key === 's' || event.key === 'S') {
      engine.pause();
      engine.stepOnce();
      timeline.refresh();
    } else if (event.key === 'r' || event.key === 'R') {
      const wasRunning = engine.running;
      engine.pause();
      world.reset(Config.world.clearWallsOnReset);
      resetHistory();
      refreshAll();
      timeline.refresh();
      if (wasRunning) engine.start();
    } else if (event.key === 'f' || event.key === 'F') {
      toolbar.fitToWindow();
    } else if (event.key === 'p' || event.key === 'P') {
      controlPanel.toggle();
    } else if (event.key === 't' || event.key === 'T') {
      speciesTree.toggle();
    } else if (event.key === 'g' || event.key === 'G') {
      speciesEditor.toggle();
    } else if (event.key === 'i' || event.key === 'I') {
      editor.setTool(editor.tool === 'inspect' ? 'paint' : 'inspect');
      toolbar.refreshBrush();
    }
  });

  // ── 起飞 ──
  resetHistory();
  refreshAll();
  engine.start();

  // 方便在控制台里调试
  window.evosim = {
    Config, world, camera, renderer, editor, engine, colorScheme, rng,
    record, charts, stats, inspector, toolbar, controlPanel, timeline, speciesTree,
    library, speciesEditor,
    save: () => serializeWorld(world, rng, { config: JSON.parse(JSON.stringify(Config)) }),
    load: (data) => deserializeWorld(world, rng, data),
    quickSave: () => saveToStorage(serializeWorld(world, rng, { config: JSON.parse(JSON.stringify(Config)) })),
    quickLoad: () => doLoad(loadFromStorage(), '快速读档'),
    quickInfo: () => storageInfo(),
    quickClear: () => clearStorage(),
    storageAvailable,
    exportFossil: () => toolbar.onExportFossil(),
    exportSpecies: () => toolbar.onExportSpecies(),
    saveText,
    hostSaveDir,
    toast,
  };
}

function clampInt(raw, min, max, fallback) {
  const n = parseInt(raw, 10);
  if (!Number.isFinite(n)) return fallback;
  return Math.max(min, Math.min(max, n));
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}
